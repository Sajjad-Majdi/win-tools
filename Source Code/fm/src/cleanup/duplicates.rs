use super::{Group, Item, Progress, Scan, ancestors, check_cancel, key, packages};
use crate::{evipc, util::*};
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use windows::Win32::Security::Cryptography::*;
use windows::Win32::Storage::FileSystem::{FILE_FLAG_OPEN_REPARSE_POINT, FILE_FLAG_SEQUENTIAL_SCAN, FILE_SHARE_READ};

pub(super) const MIN_BYTES: u64 = 10 * 1024 * 1024;

struct Hash(BCRYPT_HASH_HANDLE);

impl Drop for Hash {
    fn drop(&mut self) {
        let _ = unsafe { BCryptDestroyHash(self.0) };
    }
}

fn digest(path: &Path, size: u64, modified: u64, buffer: &mut [u8], stop: impl Fn() -> bool) -> Option<[u8; 32]> {
    let mut file = File::options().read(true).share_mode(FILE_SHARE_READ.0)
        .custom_flags((FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_SEQUENTIAL_SCAN).0).open(path).ok()?;
    let md = file.metadata().ok()?;
    if !md.is_file() || md.len() != size || md.last_write_time() != modified
        || md.file_attributes() & (ATTR_REPARSE | ATTR_OFFLINE | ATTR_RECALL_ON_DATA_ACCESS) != 0
    {
        return None;
    }
    let mut hash = BCRYPT_HASH_HANDLE::default();
    unsafe { BCryptCreateHash(BCRYPT_SHA256_ALG_HANDLE, &mut hash, None, None, 0) }.ok().ok()?;
    let hash = Hash(hash);
    let mut left = size;
    while left > 0 {
        if stop() { return None; }
        let chunk = left.min(buffer.len() as u64) as usize;
        file.read_exact(&mut buffer[..chunk]).ok()?;
        unsafe { BCryptHashData(hash.0, &buffer[..chunk], 0) }.ok().ok()?;
        left -= chunk as u64;
    }
    let mut out = [0; 32];
    unsafe { BCryptFinishHash(hash.0, &mut out, 0) }.ok().ok()?;
    Some(out)
}

pub(super) fn collect(scan: &mut Scan, group: &mut Group, hits: Vec<evipc::Item>, cancel: &AtomicBool,
    deadline: Instant, progress: &dyn Fn(Progress)) -> Res<()> {
    let stop = || cancel.load(Ordering::Relaxed) || Instant::now() >= deadline;
    let candidates = hits.len();
    let truncated = group.limited;
    let mut checked = 0;
    let mut buffer = vec![0; 256 * 1024];
    for bucket in hits.chunk_by(|a, b| a.size == b.size) {
        if stop() {
            group.limited = true;
            break;
        }
        checked += bucket.len();
        // The final bucket may have more index hits beyond the returned range.
        if truncated && checked == candidates { break; }
        let Some(size) = bucket[0].size.filter(|&size| size >= MIN_BYTES && bucket.len() > 1) else { continue };
        progress(Progress { group: group.id, checked, candidates });
        let mut files = Vec::new();
        for hit in bucket {
            if stop() { break; }
            let path = PathBuf::from(hit.full());
            let Some(name) = key(&path) else { continue };
            if scan.ignore.contains(&name) || packages::contains(&scan.packages, &name) { continue; }
            let Some(facts) = scan.rules.check(&path, &name, &mut scan.parents) else { continue };
            if !facts.is_dir() && facts.size == size && facts.links == 1 {
                files.push((facts.created, facts.modified, name, path));
            }
        }
        if stop() {
            group.limited = true;
            break;
        }
        if files.len() < 2 { continue; }
        files.sort_unstable();
        let mut keepers: HashMap<_, (String, PathBuf)> = HashMap::new();
        for (_, modified, name, path) in files {
            let Some(hash) = digest(&path, size, modified, &mut buffer, stop) else {
                // An unread older file could be the keeper for any hash in this size bucket.
                group.limited = true;
                break;
            };
            if let Some((kept_name, kept_path)) = keepers.get(&hash) {
                for reserved in [&name, kept_name] {
                    scan.duplicate_parents.extend(ancestors(reserved).map(str::to_owned));
                    scan.seen.insert(reserved.clone());
                }
                group.total_size += size;
                group.items.push(Item { path, size, modified, reason: group.reason, copy_of: Some(kept_path.clone()) });
            } else {
                keepers.insert(hash, (name, path));
            }
        }
        if stop() {
            group.limited = true;
            break;
        }
    }
    progress(Progress { group: group.id, checked, candidates });
    check_cancel(cancel)
}

pub(super) fn still_duplicate(item: &Item, kept: &Path, rules: &super::Rules) -> bool {
    let Some(facts) = key(kept).and_then(|name| rules.check(kept, &name, &mut HashMap::new())) else { return false };
    if facts.is_dir() || facts.size != item.size { return false; }
    let mut buffer = vec![0; 256 * 1024];
    let Some(hash) = digest(kept, item.size, facts.modified, &mut buffer, || false) else { return false };
    digest(&item.path, item.size, item.modified, &mut buffer, || false) == Some(hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cleanup::GroupId;
    use std::fs::{FileTimes, create_dir_all, remove_dir_all};
    use std::io::{Seek, SeekFrom, Write};
    use std::os::windows::fs::FileTimesExt;
    use std::time::{Duration, SystemTime};

    #[test]
    fn size_buckets_keep_oldest_full_match_and_mark_partial() {
        let base = std::env::var_os("WFM_DATA_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let root = base.join(format!("wfm-duplicate-{}-{}", std::process::id(), now_ft()));
        let blocked = root.join("protected");
        create_dir_all(&blocked).unwrap();
        let mut scan = Scan::new().unwrap();
        scan.rules.fixed = vec![crate::recycle::drive_root(&root).unwrap().to_ascii_lowercase()];
        scan.rules.blocked_roots.push(key(&blocked).unwrap());
        let now = SystemTime::now();
        let fixture = |path: &Path, n, age, tail| {
            let mut file = File::create(path).unwrap();
            file.set_len(n).unwrap();
            file.seek(SeekFrom::End(-1)).unwrap();
            file.write_all(&[tail]).unwrap();
            file.set_times(FileTimes::new().set_created(now - Duration::from_secs(age)).set_modified(now)).unwrap();
            evipc::Item {
                name: path.file_name().unwrap().to_string_lossy().into_owned(),
                path: path.parent().unwrap().display().to_string(),
                size: Some(n),
                ..Default::default()
            }
        };
        let oldest = root.join("old.bin");
        let copy = root.join("copy.bin");
        let hits = vec![
            fixture(&root.join("tiny.bin"), MIN_BYTES - 1, 10, 0),
            fixture(&root.join("tiny-copy.bin"), MIN_BYTES - 1, 5, 0),
            fixture(&copy, MIN_BYTES, 5, 0),
            fixture(&root.join("different-tail.bin"), MIN_BYTES, 100, 1),
            fixture(&blocked.join("protected.bin"), MIN_BYTES, 200, 0),
            fixture(&oldest, MIN_BYTES, 10, 0),
        ];
        let group = || GroupId::DuplicateFiles.group("Duplicate files", "");
        let cancel = AtomicBool::default();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut complete = group();
        collect(&mut scan, &mut complete, hits.clone(), &cancel, deadline, &|_| {}).unwrap();
        assert!(!complete.limited);
        assert_eq!(complete.total_size, MIN_BYTES);
        assert_eq!(complete.items[0].path, copy);
        assert_eq!(complete.items[0].copy_of.as_ref(), Some(&oldest));
        assert!(scan.seen.contains(&key(&oldest).unwrap()));
        assert!(still_duplicate(&complete.items[0], &oldest, &scan.rules));

        let mut capped = group();
        capped.limited = true;
        collect(&mut scan, &mut capped, hits.clone(), &cancel, deadline, &|_| {}).unwrap();
        assert!(capped.items.is_empty());
        let mut timeout = group();
        collect(&mut scan, &mut timeout, hits, &cancel, Instant::now(), &|_| {}).unwrap();
        assert!(timeout.limited);
        assert!(timeout.items.is_empty());
        remove_dir_all(root).unwrap();
    }
}
