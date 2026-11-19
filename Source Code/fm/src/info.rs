//! Details of one path: size, dates, attributes, type name, version resource (exe/dll), shortcut target (.lnk/.url),
//! link target (symlink/junction) and folder size (Everything index first, else a counted walk with a time limit).
use crate::util::*;
use std::path::Path;
use std::time::{Duration, Instant};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAGS_AND_ATTRIBUTES, GetFileAttributesExW, GetFileExInfoStandard, GetFileVersionInfoSizeW, GetFileVersionInfoW, VS_FIXEDFILEINFO,
    VerQueryValueW, WIN32_FILE_ATTRIBUTE_DATA,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile, STGM_READ};
use windows::Win32::UI::Shell::{IShellLinkW, SHFILEINFOW, SHGFI_TYPENAME, SHGFI_USEFILEATTRIBUTES, SHGetFileInfoW, ShellLink};
use windows::core::{Interface, PCWSTR};

#[derive(Clone, Debug, Default)]
pub struct VersionInfo {
    pub file_version: String,
    pub product_version: String,
    pub description: String,
    pub company: String,
    pub product: String,
    pub original_name: String,
    pub copyright: String,
}

#[derive(Clone, Debug, Default)]
pub struct LinkInfo {
    /// Target path (for .url files: the URL).
    pub target: String,
    pub arguments: String,
    pub work_dir: String,
    pub description: String,
    /// The icon the shortcut names ("" = none) and its index.
    pub icon_file: String,
    pub icon_index: i32,
}

#[derive(Clone, Debug, Default)]
pub struct FolderSize {
    pub bytes: u64,
    pub files: u64,
    /// Sub-folder count (walk only; 0 from the index).
    pub folders: u64,
    /// "everything" or "walk".
    pub source: &'static str,
    /// false when the walk hit its time limit (the numbers are then a lower bound).
    pub complete: bool,
    pub ms: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Info {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub created: u64,
    pub modified: u64,
    pub accessed: u64,
    pub attrs: u32,
    pub type_name: String,
    pub version: Option<VersionInfo>,
    pub shortcut: Option<LinkInfo>,
    /// Target of a symbolic link or junction.
    pub link_target: Option<String>,
    pub folder: Option<FolderSize>,
}

/// Basic facts plus extras that fit the type. `folder_limit`: how long a folder walk may take (None = skip folder size).
pub fn info(path: &Path, folder_limit: Option<Duration>, use_index: bool) -> Res<Info> {
    let path = abs(path);
    let w = wide_path(&path);
    let mut d = WIN32_FILE_ATTRIBUTE_DATA::default();
    unsafe { GetFileAttributesExW(PCWSTR(w.as_ptr()), GetFileExInfoStandard, &mut d as *mut _ as *mut _) }.map_err(|e| {
        if !path.exists() { format!("not found: {}", path.display()) } else { win_err(&format!("cannot read {}", path.display()), &e) }
    })?;
    let attrs = d.dwFileAttributes;
    let is_dir = attrs & ATTR_DIRECTORY != 0;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string_lossy().into_owned());
    let mut i = Info {
        path: path.to_string_lossy().into_owned(),
        name,
        is_dir,
        size: ((d.nFileSizeHigh as u64) << 32) | d.nFileSizeLow as u64,
        created: ft_u64(d.ftCreationTime),
        modified: ft_u64(d.ftLastWriteTime),
        accessed: ft_u64(d.ftLastAccessTime),
        attrs,
        type_name: type_name(&path, is_dir),
        ..Default::default()
    };
    if attrs & ATTR_REPARSE != 0 {
        i.link_target = std::fs::read_link(&path).ok().map(|t| t.to_string_lossy().into_owned());
    }
    let ext = ext_of(&i.name);
    if !is_dir && matches!(ext.as_str(), "exe" | "dll" | "sys" | "ocx" | "cpl" | "scr" | "msi" | "mui" | "drv" | "efi" | "ax") {
        i.version = version_info(&path);
    }
    if !is_dir && ext == "lnk" {
        let _c = com();
        i.shortcut = read_shortcut(&path);
    } else if !is_dir && ext == "url" {
        i.shortcut = read_url_file(&path);
    }
    if is_dir {
        if let Some(limit) = folder_limit {
            i.folder = Some(folder_size(&path, use_index, limit));
            if let Some(f) = &i.folder {
                i.size = f.bytes;
            }
        }
    }
    Ok(i)
}

/// Explorer's type name ("Text Document", "Application", ...). Uses only the extension, so it is fast.
pub fn type_name(path: &Path, is_dir: bool) -> String {
    let mut sfi = SHFILEINFOW::default();
    let w = wide(path.as_os_str());
    let attr = if is_dir { FILE_FLAGS_AND_ATTRIBUTES(ATTR_DIRECTORY) } else { FILE_FLAGS_AND_ATTRIBUTES(0x80) };
    let r = unsafe { SHGetFileInfoW(PCWSTR(w.as_ptr()), attr, Some(&mut sfi), size_of::<SHFILEINFOW>() as u32, SHGFI_TYPENAME | SHGFI_USEFILEATTRIBUTES) };
    if r == 0 { String::new() } else { from_wide(&sfi.szTypeName) }
}

/// Version resource of an exe/dll (None when it has none).
pub fn version_info(path: &Path) -> Option<VersionInfo> {
    let w = wide_path(path);
    unsafe {
        let n = GetFileVersionInfoSizeW(PCWSTR(w.as_ptr()), None);
        if n == 0 {
            return None;
        }
        let mut buf = vec![0u8; n as usize];
        GetFileVersionInfoW(PCWSTR(w.as_ptr()), None, n, buf.as_mut_ptr() as *mut _).ok()?;
        let base = buf.as_ptr() as *const core::ffi::c_void;
        let query = |sub: &str| -> Option<(*const u8, u32)> {
            let s = wide(sub);
            let mut p: *mut core::ffi::c_void = std::ptr::null_mut();
            let mut len = 0u32;
            if VerQueryValueW(base, PCWSTR(s.as_ptr()), &mut p, &mut len).as_bool() && !p.is_null() { Some((p as *const u8, len)) } else { None }
        };
        let mut vi = VersionInfo::default();
        if let Some((p, len)) = query("\\") {
            if len as usize >= size_of::<VS_FIXEDFILEINFO>() {
                let f = &*(p as *const VS_FIXEDFILEINFO);
                vi.file_version = format!("{}.{}.{}.{}", f.dwFileVersionMS >> 16, f.dwFileVersionMS & 0xffff, f.dwFileVersionLS >> 16, f.dwFileVersionLS & 0xffff);
                vi.product_version = format!("{}.{}.{}.{}", f.dwProductVersionMS >> 16, f.dwProductVersionMS & 0xffff, f.dwProductVersionLS >> 16, f.dwProductVersionLS & 0xffff);
            }
        }
        // Language/code page pairs; fall back to US English + Unicode.
        let mut langs: Vec<(u16, u16)> = vec![];
        if let Some((p, len)) = query("\\VarFileInfo\\Translation") {
            let words = std::slice::from_raw_parts(p as *const u16, len as usize / 2);
            for c in words.chunks(2) {
                if c.len() == 2 {
                    langs.push((c[0], c[1]));
                }
            }
        }
        langs.extend([(0x0409, 0x04B0), (0x0409, 0x04E4), (0x0000, 0x04B0)]);
        let read = |key: &str| -> String {
            for (l, cp) in &langs {
                if let Some((p, len)) = query(&format!("\\StringFileInfo\\{l:04x}{cp:04x}\\{key}")) {
                    let s = std::slice::from_raw_parts(p as *const u16, len as usize);
                    let t = from_wide(s).trim().to_string();
                    if !t.is_empty() {
                        return t;
                    }
                }
            }
            String::new()
        };
        vi.description = read("FileDescription");
        vi.company = read("CompanyName");
        vi.product = read("ProductName");
        vi.original_name = read("OriginalFilename");
        vi.copyright = read("LegalCopyright");
        let fv = read("FileVersion");
        if vi.file_version.is_empty() {
            vi.file_version = fv;
        }
        let pv = read("ProductVersion");
        if !pv.is_empty() {
            vi.product_version = pv;
        }
        Some(vi)
    }
}

/// Read a .lnk shortcut (needs COM on this thread; see `util::com`). Never resolves or searches, so it is fast and
/// shows no UI.
pub fn read_shortcut(lnk: &Path) -> Option<LinkInfo> {
    unsafe {
        let sl: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let pf: IPersistFile = sl.cast().ok()?;
        let w = wide(lnk.as_os_str());
        pf.Load(PCWSTR(w.as_ptr()), STGM_READ).ok()?;
        let mut buf = vec![0u16; 2048];
        let mut li = LinkInfo::default();
        if sl.GetPath(&mut buf, std::ptr::null_mut(), 0).is_ok() {
            li.target = from_wide(&buf);
        }
        buf.fill(0);
        if sl.GetArguments(&mut buf).is_ok() {
            li.arguments = from_wide(&buf);
        }
        buf.fill(0);
        if sl.GetWorkingDirectory(&mut buf).is_ok() {
            li.work_dir = from_wide(&buf);
        }
        buf.fill(0);
        if sl.GetDescription(&mut buf).is_ok() {
            li.description = from_wide(&buf);
        }
        buf.fill(0);
        let mut idx = 0i32;
        if sl.GetIconLocation(&mut buf, &mut idx).is_ok() {
            li.icon_file = from_wide(&buf);
            li.icon_index = idx;
        }
        Some(li)
    }
}

/// Create a .lnk shortcut to `target` (needs COM on this thread).
pub fn create_shortcut(lnk: &Path, target: &Path, arguments: &str) -> Res<()> {
    unsafe {
        let sl: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| win_err("cannot create shortcut", &e))?;
        let t = wide(target.as_os_str());
        sl.SetPath(PCWSTR(t.as_ptr())).map_err(|e| win_err("cannot set shortcut target", &e))?;
        if !arguments.is_empty() {
            let a = wide(arguments);
            let _ = sl.SetArguments(PCWSTR(a.as_ptr()));
        }
        if let Some(p) = target.parent() {
            let d = wide(p.as_os_str());
            let _ = sl.SetWorkingDirectory(PCWSTR(d.as_ptr()));
        }
        let pf: IPersistFile = sl.cast().map_err(|e| win_err("cannot save shortcut", &e))?;
        let l = wide(lnk.as_os_str());
        pf.Save(PCWSTR(l.as_ptr()), true).map_err(|e| win_err(&format!("cannot save {}", lnk.display()), &e))
    }
}

/// Read a .url Internet shortcut: its URL and the icon it names.
pub fn read_url_file(p: &Path) -> Option<LinkInfo> {
    let s = std::fs::read_to_string(p).ok()?;
    let url = s.lines().find_map(|l| l.trim().strip_prefix("URL=").map(String::from))?;
    let icon_file = s.lines().find_map(|l| l.trim().strip_prefix("IconFile=").map(String::from)).unwrap_or_default();
    let icon_index = s.lines().find_map(|l| l.trim().strip_prefix("IconIndex=").and_then(|x| x.trim().parse().ok())).unwrap_or(0);
    Some(LinkInfo { target: url, icon_file, icon_index, ..Default::default() })
}

/// Folder size: Everything's index when allowed and available (instant), else a walk that stops after `limit`.
pub fn folder_size(dir: &Path, use_index: bool, limit: Duration) -> FolderSize {
    let t0 = Instant::now();
    if use_index {
        if let Some((bytes, files)) = crate::search::indexed_folder_size(dir) {
            return FolderSize { bytes, files, folders: 0, source: "everything", complete: true, ms: t0.elapsed().as_millis() as u64 };
        }
    }
    let mut fs = FolderSize { source: "walk", complete: true, ..Default::default() };
    let deadline = t0 + limit;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        if Instant::now() > deadline {
            fs.complete = false;
            break;
        }
        let Ok(list) = crate::list::list_dir(&d, true) else { continue };
        for e in list {
            if e.is_dir() {
                fs.folders += 1;
                // Do not follow junctions/symlinks (they can loop and they double-count).
                if e.attrs & ATTR_REPARSE == 0 {
                    stack.push(d.join(&e.name));
                }
            } else {
                fs.files += 1;
                fs.bytes += e.size;
            }
        }
    }
    fs.ms = t0.elapsed().as_millis() as u64;
    fs
}
