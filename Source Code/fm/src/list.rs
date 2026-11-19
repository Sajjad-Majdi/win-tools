//! Folder listing and sorting. One FindFirstFileExW pass with FIND_FIRST_EX_LARGE_FETCH and FindExInfoBasic
//! (no 8.3 names), so a 10k-file folder lists in a few milliseconds.
use crate::util::*;
use std::cmp::Ordering;
use std::path::Path;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES};
use windows::Win32::Storage::FileSystem::{
    FIND_FIRST_EX_LARGE_FETCH, FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW, FindNextFileW, WIN32_FIND_DATAW,
};
use windows::core::PCWSTR;

/// One file or folder in a listing. Times are raw FILETIME values (100 ns since 1601, UTC); see `util::ft_unix`.
#[derive(Clone, Debug, Default)]
pub struct Entry {
    pub name: String,
    pub size: u64,
    pub modified: u64,
    pub created: u64,
    pub accessed: u64,
    pub attrs: u32,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.attrs & ATTR_DIRECTORY != 0
    }
    pub fn is_hidden(&self) -> bool {
        self.attrs & ATTR_HIDDEN != 0
    }
    /// Lower-case extension ("" for folders and names without one).
    pub fn ext(&self) -> String {
        if self.is_dir() { String::new() } else { ext_of(&self.name) }
    }
    /// Short type label: "folder" or the extension ("file" when there is none).
    pub fn kind(&self) -> String {
        if self.is_dir() {
            "folder".into()
        } else {
            let e = self.ext();
            if e.is_empty() { "file".into() } else { e }
        }
    }
}

fn entry_from(fd: &WIN32_FIND_DATAW) -> Entry {
    Entry {
        name: from_wide(&fd.cFileName),
        size: ((fd.nFileSizeHigh as u64) << 32) | fd.nFileSizeLow as u64,
        modified: ft_u64(fd.ftLastWriteTime),
        created: ft_u64(fd.ftCreationTime),
        accessed: ft_u64(fd.ftLastAccessTime),
        attrs: fd.dwFileAttributes,
    }
}

/// List one folder (not recursive). `show_hidden` keeps hidden and system entries.
pub fn list_dir(dir: &Path, show_hidden: bool) -> Res<Vec<Entry>> {
    let pattern = wide_path(&dir.join("*"));
    let mut fd = WIN32_FIND_DATAW::default();
    let h = unsafe {
        FindFirstFileExW(PCWSTR(pattern.as_ptr()), FindExInfoBasic, &mut fd as *mut _ as *mut _, FindExSearchNameMatch, None, FIND_FIRST_EX_LARGE_FETCH)
    };
    let h = match h {
        Ok(h) => h,
        Err(e) if e.code() == ERROR_FILE_NOT_FOUND.to_hresult() => return Ok(vec![]),
        Err(e) => {
            if !dir.exists() {
                return Err(format!("folder not found: {}", dir.display()));
            }
            if !dir.is_dir() {
                return Err(format!("not a folder: {}", dir.display()));
            }
            return Err(win_err(&format!("cannot list {}", dir.display()), &e));
        }
    };
    let mut out = Vec::with_capacity(256);
    loop {
        let n = &fd.cFileName;
        let dot = n[0] == b'.' as u16 && (n[1] == 0 || (n[1] == b'.' as u16 && n[2] == 0));
        if !dot && (show_hidden || fd.dwFileAttributes & ATTR_HIDDEN == 0) {
            out.push(entry_from(&fd));
        }
        if let Err(e) = unsafe { FindNextFileW(h, &mut fd) } {
            if e.code() != ERROR_NO_MORE_FILES.to_hresult() {
                unsafe { let _ = FindClose(h); }
                return Err(win_err("listing stopped", &e));
            }
            break;
        }
    }
    unsafe {
        let _ = FindClose(h);
    }
    Ok(out)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Size,
    Date,
    Created,
    Type,
}

impl SortKey {
    pub fn parse(s: &str) -> Option<SortKey> {
        Some(match s {
            "name" => SortKey::Name,
            "size" => SortKey::Size,
            "date" | "modified" | "mtime" => SortKey::Date,
            "created" | "ctime" => SortKey::Created,
            "type" | "ext" | "kind" => SortKey::Type,
            _ => return None,
        })
    }
}

/// Sort like Explorer: natural name order ("file2" before "file10"), optional folders first, optional descending.
pub fn sort_entries(v: &mut [Entry], key: SortKey, desc: bool, dirs_first: bool) {
    // Pre-compute lower-case names once (much faster than lower-casing inside every compare).
    let mut idx: Vec<(Vec<char>, String, usize)> = v.iter().enumerate().map(|(i, e)| (e.name.to_lowercase().chars().collect(), e.ext(), i)).collect();
    idx.sort_by(|a, b| {
        let (ea, eb) = (&v[a.2], &v[b.2]);
        if dirs_first && ea.is_dir() != eb.is_dir() {
            return if ea.is_dir() { Ordering::Less } else { Ordering::Greater };
        }
        let name = || natural_cmp_chars(&a.0, &b.0);
        let o = match key {
            SortKey::Name => name(),
            SortKey::Size => ea.size.cmp(&eb.size).then_with(name),
            SortKey::Date => ea.modified.cmp(&eb.modified).then_with(name),
            SortKey::Created => ea.created.cmp(&eb.created).then_with(name),
            SortKey::Type => (!ea.is_dir()).cmp(&!eb.is_dir()).then_with(|| a.1.cmp(&b.1)).then_with(name),
        };
        if desc { o.reverse() } else { o }
    });
    let order: Vec<usize> = idx.into_iter().map(|t| t.2).collect();
    let sorted: Vec<Entry> = order.iter().map(|&i| std::mem::take(&mut v[i])).collect();
    for (slot, e) in v.iter_mut().zip(sorted) {
        *slot = e;
    }
}

/// Natural, case-insensitive compare of two names ("a2" < "a10").
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let a: Vec<char> = a.to_lowercase().chars().collect();
    let b: Vec<char> = b.to_lowercase().chars().collect();
    natural_cmp_chars(&a, &b)
}

fn natural_cmp_chars(a: &[char], b: &[char]) -> Ordering {
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        let (ca, cb) = (a[i], b[j]);
        if ca.is_ascii_digit() && cb.is_ascii_digit() {
            let si = i;
            while i < a.len() && a[i] == '0' {
                i += 1;
            }
            let sj = j;
            while j < b.len() && b[j] == '0' {
                j += 1;
            }
            let ni = i;
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            let nj = j;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let (da, db) = (&a[ni..i], &b[nj..j]);
            let o = da.len().cmp(&db.len()).then_with(|| da.cmp(db)).then_with(|| (j - sj).cmp(&(i - si)));
            if o != Ordering::Equal {
                return o;
            }
        } else {
            if ca != cb {
                return ca.cmp(&cb);
            }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j))
}

/// Sum of sizes and counts of a listing (for status bars).
pub fn totals(v: &[Entry]) -> (u64, usize, usize) {
    let mut bytes = 0;
    let (mut files, mut dirs) = (0, 0);
    for e in v {
        if e.is_dir() {
            dirs += 1;
        } else {
            files += 1;
            bytes += e.size;
        }
    }
    (bytes, files, dirs)
}
