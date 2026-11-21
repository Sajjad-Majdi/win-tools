//! Cleanup uses the Shell's recycle-only transfer and records its own batch for undo.
use super::{Item, Rules, can_recycle, key, recycle_one, under};
use crate::{journal, recycle, util::*};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Shell::{COPYENGINE_E_RECYCLE_BIN_NOT_FOUND, COPYENGINE_E_RECYCLE_FORCE_NUKE, COPYENGINE_E_RECYCLE_SIZE_TOO_BIG, SHQUERYRBINFO, SHQueryRecycleBinW};
use windows::core::PCWSTR;

#[derive(Clone, Copy, Debug, Default)]
pub struct BinInfo {
    pub bytes: u64,
    pub count: u64,
}

fn bin_query(root: &str) -> Option<BinInfo> {
    let root = wide(root);
    let mut info = SHQUERYRBINFO { cbSize: size_of::<SHQUERYRBINFO>() as u32, ..Default::default() };
    unsafe { SHQueryRecycleBinW(PCWSTR(root.as_ptr()), &mut info) }.ok()?;
    Some(BinInfo { bytes: info.i64Size.max(0) as u64, count: info.i64NumItems.max(0) as u64 })
}

pub fn bin_info(roots: &[String]) -> BinInfo {
    let mut out = BinInfo::default();
    for root in roots {
        if let Some(info) = bin_query(root) {
            out.bytes += info.bytes;
            out.count += info.count;
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skip {
    NoBin,
    TooBig,
    InUse,
    Changed,
    Refused,
}

#[derive(Default)]
pub struct MoveReport {
    pub moved: Vec<Item>,
    pub skipped: Vec<(PathBuf, Skip)>,
    pub records: Vec<recycle::Recycled>,
    pub journal: u64,
    pub warning: Option<String>,
}

fn volume_setting(root: &str, name: &str) -> Option<u32> {
    let mut vol = [0; 64];
    let path = wide(root);
    unsafe { GetVolumeNameForVolumeMountPointW(PCWSTR(path.as_ptr()), &mut vol) }.ok()?;
    let volume = from_wide(&vol);
    let guid = volume.get(volume.find('{')?..=volume.find('}')?)?;
    let sub = wide(format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\BitBucket\Volume\{guid}"));
    let name = wide(name);
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    (unsafe { RegGetValueW(HKEY_CURRENT_USER, PCWSTR(sub.as_ptr()), PCWSTR(name.as_ptr()), RRF_RT_REG_DWORD,
        None, Some((&mut value as *mut u32).cast()), Some(&mut size)) }.0 == 0).then_some(value)
}

fn limit(root: &str) -> Result<Option<u64>, Skip> {
    if !recycle::has_recycle_bin(Path::new(root)) || bin_query(root).is_none() || volume_setting(root, "NukeOnDelete") == Some(1) {
        return Err(Skip::NoBin);
    }
    // An unset quota is resolved by the Shell's recycle-only operation.
    Ok(volume_setting(root, "MaxCapacity").filter(|&mb| mb != u32::MAX).map(|mb| u64::from(mb) * 1024 * 1024))
}

fn check_limit(item: &Item, limits: &mut HashMap<String, Result<Option<u64>, Skip>>) -> Result<String, Skip> {
    let root = recycle::drive_root(&item.path).ok_or(Skip::NoBin)?;
    if let Some(max) = (*limits.entry(root.clone()).or_insert_with(|| limit(&root)))? && item.size > max {
        return Err(Skip::TooBig);
    }
    Ok(root)
}

pub fn bin_limits(items: &[Item]) -> Vec<(PathBuf, Skip)> {
    let mut limits = HashMap::new();
    items.iter().filter_map(|item| check_limit(item, &mut limits).err().map(|why| (item.path.clone(), why))).collect()
}

fn in_use(path: &Path) -> bool {
    let path = wide_path(path);
    match unsafe { CreateFileW(PCWSTR(path.as_ptr()), (DELETE | FILE_READ_ATTRIBUTES).0, FILE_SHARE_MODE(0), None,
        OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT, None) }
    {
        Ok(h) => {
            let _ = unsafe { windows::Win32::Foundation::CloseHandle(h) };
            false
        }
        Err(e) => matches!(e.code().0 as u32, 0x8007_0020 | 0x8007_0021),
    }
}

fn move_one(item: &Item, rules: &Rules) -> Result<(), Skip> {
    let name = key(&item.path).ok_or(Skip::Refused)?;
    if rules.blocks(&name) {
        return Err(Skip::Refused);
    }
    let facts = rules.check(&item.path, &name, &mut HashMap::new())
        .ok_or_else(|| if in_use(&item.path) { Skip::InUse } else { Skip::Refused })?;
    if facts.modified != item.modified || (!facts.is_dir() && (facts.size != item.size || facts.links != 1)) {
        return Err(Skip::Changed);
    }
    if facts.is_dir() && can_recycle(rules, &item.path) != Some(item.size) {
        return Err(Skip::Changed);
    }
    if let Some(kept) = &item.copy_of && !super::duplicates::still_duplicate(item, kept, rules)
    {
        return Err(Skip::Changed);
    }
    recycle_one(&item.path).map_err(|e| match e.code() {
        COPYENGINE_E_RECYCLE_SIZE_TOO_BIG => Skip::TooBig,
        COPYENGINE_E_RECYCLE_BIN_NOT_FOUND | COPYENGINE_E_RECYCLE_FORCE_NUKE => Skip::NoBin,
        _ if in_use(&item.path) => Skip::InUse,
        _ => Skip::Refused,
    })
}

/// Move only the items in the confirmed snapshot. A refusal leaves the original in place.
pub fn move_items(items: Vec<Item>) -> MoveReport {
    let mut out = MoveReport::default();
    let rules = match Rules::load() {
        Ok(rules) => rules,
        Err(e) => {
            out.warning = Some(e);
            out.skipped = items.into_iter().map(|i| (i.path, Skip::Refused)).collect();
            return out;
        }
    };
    let _com = com();
    let mut limits = HashMap::new();
    let mut before = HashMap::<String, HashSet<PathBuf>>::new();
    let ignored: HashSet<_> = super::ignored().iter().filter_map(|p| key(p)).collect();
    for item in items {
        let result = check_limit(&item, &mut limits).and_then(|root| {
            if key(&item.path).is_some_and(|p| ignored.contains(&p) || ignored.iter().any(|i| under(i, &p))) {
                return Err(Skip::Refused);
            }
            before.entry(root.clone()).or_insert_with(|| recycle::list_bin(Path::new(&root)).into_iter().map(|r| r.record).collect());
            move_one(&item, &rules)
        });
        match result {
            Ok(()) => out.moved.push(item),
            Err(why) => out.skipped.push((item.path, why)),
        }
    }
    let moved: HashSet<_> = out.moved.iter().filter_map(|i| key(&i.path)).collect();
    for (root, before) in before {
        out.records.extend(recycle::list_bin(Path::new(&root)).into_iter().filter(|r| {
            !before.contains(&r.record) && key(&r.original).is_some_and(|p| moved.contains(&p))
        }));
    }
    if out.records.len() != out.moved.len() {
        out.warning = Some("Some moved items could not be recorded for Undo".into());
    }
    let mut op = journal::Op::new("cleanup");
    op.recycled.clone_from(&out.records);
    match journal::record(op) {
        Ok(id) => out.journal = id,
        Err(e) => out.warning = Some(e),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_bin_limits_refuse_oversize_and_missing_bins() {
        let mut limits = HashMap::from([("C:\\".into(), Ok(Some(100))), ("D:\\".into(), Err(Skip::NoBin))]);
        let item = |path: &str, size| Item { path: path.into(), size, modified: 0, reason: "", copy_of: None };
        assert_eq!(check_limit(&item(r"C:\Temp\fits", 100), &mut limits), Ok("C:\\".into()));
        assert_eq!(check_limit(&item(r"C:\Temp\too-big", 101), &mut limits), Err(Skip::TooBig));
        assert_eq!(check_limit(&item(r"D:\Temp\no-bin", 1), &mut limits), Err(Skip::NoBin));
        assert_eq!(check_limit(&item(r"\\server\share\file", 1), &mut limits), Err(Skip::NoBin));
    }
}
