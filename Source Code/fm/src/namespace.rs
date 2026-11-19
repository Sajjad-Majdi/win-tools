//! The Shell namespace: places with no drive letter, mainly phones and cameras (MTP) under "This PC".
//! Items are addressed by their parsing name (for files that is the normal path; for a phone it looks like
//! `::{20D04FE0-...}\\\?\usb#vid_...`). Copying uses IFileOperation with no UI.
use crate::util::*;
use std::path::Path;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::Win32::System::SystemServices::{SFGAO_FILESYSTEM, SFGAO_FLAGS, SFGAO_FOLDER};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOF_NO_UI, FOLDERID_ComputerFolder, FileOperation, IEnumShellItems, IFileOperation, IFileOperationProgressSink, IShellItem, KF_FLAG_DEFAULT,
    SHCreateItemFromParsingName, SHGetKnownFolderItem, SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_NORMALDISPLAY,
};
use windows::core::PCWSTR;

/// One item in the Shell namespace.
#[derive(Clone, Debug)]
pub struct ShellEntry {
    /// The name people see.
    pub name: String,
    /// The address to browse or copy it (a normal path for files and folders on drives).
    pub parsing: String,
    pub is_folder: bool,
    /// true for normal files and folders on a drive; false for phones, cameras and virtual folders.
    pub is_filesystem: bool,
}

fn entry(it: &IShellItem) -> Option<ShellEntry> {
    unsafe {
        let name = take_pwstr(it.GetDisplayName(SIGDN_NORMALDISPLAY).ok()?);
        let parsing = take_pwstr(it.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING).ok()?);
        let a = it.GetAttributes(SFGAO_FLAGS(SFGAO_FOLDER.0 | SFGAO_FILESYSTEM.0)).unwrap_or_default();
        Some(ShellEntry { name, parsing, is_folder: a.0 & SFGAO_FOLDER.0 != 0, is_filesystem: a.0 & SFGAO_FILESYSTEM.0 != 0 })
    }
}

/// A shell item from a parsing name or a normal path.
pub fn item(parsing: &str) -> Res<IShellItem> {
    let w = wide(parsing);
    unsafe { SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None) }.map_err(|e| win_err(&format!("cannot find '{parsing}'"), &e))
}

fn enum_children(folder: &IShellItem) -> Res<Vec<ShellEntry>> {
    unsafe {
        let en: IEnumShellItems = folder.BindToHandler(None, &BHID_EnumItems).map_err(|e| win_err("cannot open this folder", &e))?;
        let mut out = vec![];
        loop {
            let mut slot: [Option<IShellItem>; 1] = [None];
            let mut got = 0u32;
            if en.Next(&mut slot, Some(&mut got)).is_err() || got == 0 {
                break;
            }
            if let Some(e) = slot[0].as_ref().and_then(entry) {
                out.push(e);
            }
        }
        Ok(out)
    }
}

/// Children of any shell folder (a phone, a folder on a phone, or a normal folder).
pub fn children(parsing: &str) -> Res<Vec<ShellEntry>> {
    let _c = com();
    enum_children(&item(parsing)?)
}

/// Everything under "This PC": drives, known folders, phones and cameras.
pub fn this_pc() -> Res<Vec<ShellEntry>> {
    let _c = com();
    let pc: IShellItem = unsafe { SHGetKnownFolderItem(&FOLDERID_ComputerFolder, KF_FLAG_DEFAULT, None) }.map_err(|e| win_err("cannot open This PC", &e))?;
    enum_children(&pc)
}

/// Phones, cameras and other portable (MTP) devices: the "This PC" items that are devices, not drives.
pub fn portable_devices() -> Res<Vec<ShellEntry>> {
    Ok(this_pc()?.into_iter().filter(|e| !e.is_filesystem && e.is_folder && e.parsing.contains(r"\\?\")).collect())
}

/// Copy shell items (e.g. photos on a phone) into a folder (a normal path or another shell folder), no UI.
pub fn copy(items: &[String], dest: &str) -> Res<()> {
    let _c = com();
    unsafe {
        let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL).map_err(|e| win_err("cannot start the copy", &e))?;
        op.SetOperationFlags(FOF_NO_UI).map_err(|e| win_err("cannot start the copy", &e))?;
        let d = item(dest)?;
        for it in items {
            let s = item(it)?;
            op.CopyItem(&s, &d, PCWSTR::null(), None::<&IFileOperationProgressSink>).map_err(|e| win_err(&format!("cannot copy '{it}'"), &e))?;
        }
        op.PerformOperations().map_err(|e| win_err("copy failed", &e))?;
        if op.GetAnyOperationsAborted().map(|b| b.as_bool()).unwrap_or(false) {
            return Err("the copy was stopped".into());
        }
    }
    Ok(())
}

/// Normal path -> its parsing name (the same text for files on drives).
pub fn parsing_name_of(path: &Path) -> Res<String> {
    let _c = com();
    let it = item(&abs(path).to_string_lossy())?;
    entry(&it).map(|e| e.parsing).ok_or_else(|| "no parsing name".into())
}
