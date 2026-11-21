//! One set of path and live filesystem checks for both scanning and recycling.
use crate::util::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::UI::Shell::*;
use windows::core::{GUID, PCWSTR};

const UNSAFE_ATTRS: u32 = ATTR_HIDDEN | ATTR_SYSTEM | ATTR_REPARSE | ATTR_OFFLINE | ATTR_RECALL_ON_DATA_ACCESS;
const PROTECTED_NAMES: &[&str] = &[
    "windows", "program files", "program files (x86)", "programdata", "$recycle.bin",
    "system volume information", "recovery", "boot", "pagefile.sys", "swapfile.sys", "hiberfil.sys", "bootmgr",
];

pub(super) fn known(id: &GUID) -> Option<PathBuf> {
    let p = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DONT_VERIFY, None) }.ok()?;
    let s = take_pwstr(p);
    key(Path::new(&s))?;
    Some(PathBuf::from(s))
}

/// Accept DOS drive paths only. Names must survive conversion to Win32 and Shell paths unchanged.
pub(super) fn key(path: &Path) -> Option<String> {
    let s = path.to_str()?.replace('/', "\\");
    let b = s.as_bytes();
    if b.len() < 3 || !b[0].is_ascii_alphabetic() || b[1..3] != *b":\\" {
        return None;
    }
    if b.len() > 3 && s[3..].split('\\').any(|c| {
        c.is_empty() || c.ends_with(['.', ' ']) || c.chars().any(|ch| ch < ' ' || ":*?\"<>|".contains(ch))
    }) {
        return None;
    }
    Some(s.to_lowercase())
}

pub(super) fn under(path: &str, root: &str) -> bool {
    path.strip_prefix(root).is_some_and(|tail| tail.starts_with('\\'))
}

#[derive(Default)]
pub(super) struct Rules {
    pub fixed: Vec<String>,
    pub blocked_roots: Vec<String>,
    pub profile: String,
    pub win_temp: String,
    pub app_data: String,
}

impl Rules {
    pub fn load() -> Res<Self> {
        let folder = |id| known(id).and_then(|p| key(&p));
        let windows = folder(&FOLDERID_Windows).ok_or("cannot locate Windows")?;
        let profile = folder(&FOLDERID_Profile).ok_or("cannot locate the user profile")?;
        let program_data = folder(&FOLDERID_ProgramData).ok_or("cannot locate ProgramData")?;
        let program_files = folder(&FOLDERID_ProgramFiles).ok_or("cannot locate Program Files")?;
        let mut blocked_roots = vec![windows.clone(), program_data, program_files];
        blocked_roots.extend([FOLDERID_ProgramFilesX86, FOLDERID_ProgramFilesX64].iter().filter_map(folder));
        Ok(Self {
            fixed: fixed_drives(),
            blocked_roots,
            app_data: format!("{profile}\\appdata"),
            profile,
            win_temp: format!("{windows}\\temp"),
        })
    }

    pub fn blocks(&self, path: &str) -> bool {
        if path.len() <= 3 || !self.fixed.iter().any(|d| path.starts_with(d)) || path == self.profile || path == self.app_data {
            return true;
        }
        if under(path, &self.win_temp) {
            return false;
        }
        self.blocked_roots.iter().any(|p| path == p || under(path, p))
            || PROTECTED_NAMES.contains(&path[3..].split('\\').next().unwrap_or(""))
    }

    pub fn attrs_block(&self, path: &str, attrs: u32) -> bool {
        let mask = if path == self.app_data { UNSAFE_ATTRS & !ATTR_HIDDEN } else { UNSAFE_ATTRS };
        attrs & mask != 0
    }

    pub fn check(&self, path: &Path, normalized: &str, parents: &mut HashMap<PathBuf, bool>) -> Option<Facts> {
        if self.blocks(normalized) || !self.parents_ok(path.parent()?, parents) {
            return None;
        }
        let facts = probe(path, normalized)?;
        if self.attrs_block(normalized, facts.attrs) || facts.attrs & ATTR_READONLY != 0 {
            return None;
        }
        Some(facts)
    }

    fn parents_ok(&self, path: &Path, cache: &mut HashMap<PathBuf, bool>) -> bool {
        if let Some(&ok) = cache.get(path) {
            return ok;
        }
        let ok = key(path).is_some_and(|name| {
            let Some(parent) = path.parent() else { return name.len() == 3 };
            self.parents_ok(parent, cache) && std::fs::symlink_metadata(path).is_ok_and(|m| {
                use std::os::windows::fs::MetadataExt;
                !self.attrs_block(&name, m.file_attributes())
            })
        });
        cache.insert(path.to_path_buf(), ok);
        ok
    }
}

fn fixed_drives() -> Vec<String> {
    let mask = unsafe { GetLogicalDrives() };
    (0..26).filter(|i| mask & (1 << i) != 0).filter_map(|i| {
        let letter = char::from(b'a' + i);
        let root = format!("{letter}:\\");
        if !crate::recycle::has_recycle_bin(Path::new(&root)) {
            return None;
        }
        // SUBST and similar DOS aliases may report as fixed drives.
        let mut target = [0; 1024];
        let drive = wide(format!("{letter}:"));
        let n = unsafe { QueryDosDeviceW(PCWSTR(drive.as_ptr()), Some(&mut target)) };
        (n != 0 && from_wide(&target).starts_with("\\Device\\HarddiskVolume")).then_some(root)
    }).collect()
}

pub(super) struct Facts {
    pub attrs: u32,
    pub size: u64,
    pub modified: u64,
}

impl Facts {
    pub fn is_dir(&self) -> bool {
        self.attrs & ATTR_DIRECTORY != 0
    }
}

fn probe(path: &Path, normalized: &str) -> Option<Facts> {
    let wide = wide_path(path);
    unsafe {
        // Ask for delete access with exclusive sharing. This only opens the object and checks access.
        let h = CreateFileW(
            PCWSTR(wide.as_ptr()), (DELETE | FILE_READ_ATTRIBUTES).0, FILE_SHARE_MODE(0), None,
            OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT, None,
        ).ok()?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        let result = GetFileInformationByHandle(h, &mut info).ok().and_then(|()| {
            let mut buf = [0u16; 512];
            let flags = FILE_NAME_NORMALIZED; // The default volume format is a DOS drive letter.
            let len = GetFinalPathNameByHandleW(h, &mut buf, flags) as usize;
            let canonical = if len >= buf.len() {
                let mut buf = vec![0; len];
                let n = GetFinalPathNameByHandleW(h, &mut buf, flags) as usize;
                String::from_utf16(buf.get(..n)?).ok()?
            } else {
                String::from_utf16(&buf[..len]).ok()?
            };
            // Reject short names, unexpected aliases, and paths Windows resolved to another volume.
            let resolved = canonical.strip_prefix("\\\\?\\")?;
            if key(Path::new(resolved)).as_deref() != Some(normalized) {
                return None;
            }
            Some(Facts {
                attrs: info.dwFileAttributes,
                size: (u64::from(info.nFileSizeHigh) << 32) | u64::from(info.nFileSizeLow),
                modified: ft_u64(info.ftLastWriteTime),
            })
        });
        let _ = CloseHandle(h);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_paths_and_components() {
        let rules = Rules {
            fixed: vec!["c:\\".into(), "d:\\".into()],
            blocked_roots: vec![r"d:\custom apps".into()],
            profile: r"c:\users\me".into(),
            win_temp: r"c:\windows\temp".into(),
            app_data: r"c:\users\me\appdata".into(),
        };
        for p in [
            r"C:\", r"C:\Windows", r"C:\Windows\System32\log.txt", r"C:\Windows\Temp", r"C:\Windows\Temporary\old",
            r"C:\Program Files\a", r"C:\Program Files (x86)\a", r"C:\ProgramData\app\cache", r"D:\Custom Apps\cache",
            r"C:\Users\Me", r"C:\Users\Me\AppData", r"C:\$Recycle.Bin\entry", r"Z:\temp\file",
        ] {
            assert!(rules.blocks(&key(Path::new(p)).unwrap()), "{p}");
        }
        for p in [r"C:\Windows\Temp\old.txt", r"C:\WINDOWS\TEMP\folder\old.txt", r"C:\Program Files Backup\old.txt", r"c:\Users\Me\Documents"] {
            assert!(!rules.blocks(&key(Path::new(p)).unwrap()), "{p}");
        }
        for p in [r"C:foo", r"C:\Temp\..\Windows", r"C:\Windows.\file", r"C:\Temp\file:stream", r"\\server\share\file", r"\\?\C:\file"] {
            assert!(key(Path::new(p)).is_none(), "{p}");
        }
        assert!(rules.attrs_block(r"c:\file", ATTR_HIDDEN | ATTR_SYSTEM));
        assert!(rules.attrs_block(&rules.app_data, ATTR_HIDDEN | ATTR_SYSTEM));
        assert!(!rules.attrs_block(&rules.app_data, ATTR_HIDDEN));
        assert!(rules.attrs_block(r"c:\link", ATTR_REPARSE));
    }
}
