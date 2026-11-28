//! What is behind a drive letter and how it is ejected, as Explorer does it: a mounted disc image (ISO / VHD) is
//! dismounted, a USB stick or USB disk is removed safely (the whole device, like "Safely Remove Hardware"), and a
//! DVD or a memory card gets its media ejected. Also mounts disc images (Explorer's "Mount", without its window).
//!
//! Everything here is blocking (an eject can take a second or two): the GUI runs it on a worker thread.
use crate::util::*;
use std::path::Path;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, GetVolumeInformationW, OPEN_EXISTING};
use windows::Win32::System::IO::DeviceIoControl;
use windows::core::PCWSTR;

/// How a drive is ejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eject {
    /// Not ejectable (internal disks, network drives, the Windows drive).
    No,
    /// A mounted disc image (ISO, VHD): dismount it; the drive letter goes away.
    Image,
    /// A USB stick or USB disk: remove the whole device safely; the drive letter goes away.
    Device,
    /// A DVD / CD drive or a memory card reader: eject the media; the drive stays, empty.
    Media,
}

impl Eject {
    pub fn as_str(self) -> &'static str {
        match self {
            Eject::No => "",
            Eject::Image => "image",
            Eject::Device => "device",
            Eject::Media => "media",
        }
    }
}

const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002D_1400;
const IOCTL_STORAGE_GET_HOTPLUG_INFO: u32 = 0x002D_0C14;
const IOCTL_STORAGE_GET_DEVICE_NUMBER: u32 = 0x002D_1080;
const IOCTL_STORAGE_MEDIA_REMOVAL: u32 = 0x002D_4804;
const IOCTL_STORAGE_EJECT_MEDIA: u32 = 0x002D_4808;
const FSCTL_LOCK_VOLUME: u32 = 0x0009_0018;
const FSCTL_DISMOUNT_VOLUME: u32 = 0x0009_0020;

struct H(HANDLE);
impl Drop for H {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// "E", "e:", "E:\\" -> 'E'.
pub fn letter_of(drive: &str) -> Res<char> {
    let l = drive.trim().trim_end_matches(['\\', '/', ':']).to_ascii_uppercase();
    match l.chars().next() {
        Some(c) if l.len() == 1 && c.is_ascii_alphabetic() => Ok(c),
        _ => Err(format!("not a drive letter: {drive}")),
    }
}

/// The drive Windows runs from: never ejected.
fn is_system_drive(l: char) -> bool {
    let sys = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    let win = std::env::var("SystemRoot").unwrap_or_default();
    [sys, win].iter().any(|s| s.chars().next().is_some_and(|c| c.eq_ignore_ascii_case(&l)))
}

/// `\\.\E:` with the given access (0 = query only: needs no rights and touches no media).
fn open_volume(l: char, access: u32) -> windows::core::Result<H> {
    let dev = wide(format!(r"\\.\{l}:"));
    unsafe { CreateFileW(PCWSTR(dev.as_ptr()), access, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None) }.map(H)
}

fn ioctl(h: &H, code: u32, input: &[u8], out: &mut [u8]) -> windows::core::Result<u32> {
    let mut n = 0u32;
    unsafe {
        DeviceIoControl(
            h.0,
            code,
            (!input.is_empty()).then_some(input.as_ptr() as *const _),
            input.len() as u32,
            (!out.is_empty()).then_some(out.as_mut_ptr() as *mut _),
            out.len() as u32,
            Some(&mut n),
            None,
        )
    }?;
    Ok(n)
}

/// The storage bus of a volume's disk (7 USB, 4 FireWire, 12 SD, 13 MMC, 14 virtual, 15 file-backed virtual, ...).
fn bus_type(h: &H) -> Option<u32> {
    // STORAGE_PROPERTY_QUERY { StorageDeviceProperty, PropertyStandardQuery }
    let q = [0u8; 12];
    let mut out = vec![0u8; 1024];
    let n = ioctl(h, IOCTL_STORAGE_QUERY_PROPERTY, &q, &mut out).ok()?;
    // STORAGE_DEVICE_DESCRIPTOR.BusType is at byte 28
    (n >= 32).then(|| u32::from_le_bytes(out[28..32].try_into().unwrap()))
}

/// (media removable, device hot-pluggable) from STORAGE_HOTPLUG_INFO.
fn hotplug(h: &H) -> Option<(bool, bool)> {
    let mut out = [0u8; 8];
    ioctl(h, IOCTL_STORAGE_GET_HOTPLUG_INFO, &[], &mut out).ok()?;
    Some((out[4] != 0, out[6] != 0))
}

/// One virtual disk a volume lives on: its type and the image file (host volume GUID path + path inside it).
struct Backing {
    vst: windows::Win32::Storage::Vhd::VIRTUAL_STORAGE_TYPE,
    level: u32,
    host: String,
    rel: String,
}

impl Backing {
    /// `\\?\Volume{...}\dir\file.iso`: a valid path for OpenVirtualDisk even when the host has no letter.
    fn raw_path(&self) -> String {
        format!("{}\\{}", self.host.trim_end_matches('\\'), self.rel.trim_start_matches('\\'))
    }

    /// The image file with a drive letter when its volume has one (C:\dir\file.iso).
    fn path(&self) -> String {
        let mut names = vec![0u16; 1024];
        let mut n = 0u32;
        let g = wide(format!("{}\\", self.host.trim_end_matches('\\')));
        let ok = unsafe { windows::Win32::Storage::FileSystem::GetVolumePathNamesForVolumeNameW(PCWSTR(g.as_ptr()), Some(&mut names), &mut n) }.is_ok();
        let first = if ok { from_wide(&names) } else { String::new() };
        if first.is_empty() { self.raw_path() } else { format!("{}\\{}", first.trim_end_matches('\\'), self.rel.trim_start_matches('\\')) }
    }
}

/// The virtual disks under a volume (none for a real disk).
fn backing(h: &H) -> Vec<Backing> {
    use windows::Win32::Storage::Vhd::{GET_STORAGE_DEPENDENCY_FLAG_HOST_VOLUMES, GetStorageDependencyInformation, STORAGE_DEPENDENCY_INFO, STORAGE_DEPENDENCY_INFO_VERSION};
    let mut size = 4096u32;
    for _ in 0..2 {
        let mut buf = vec![0u64; size as usize / 8 + 1];
        let info = buf.as_mut_ptr() as *mut STORAGE_DEPENDENCY_INFO;
        unsafe { (*info).Version = STORAGE_DEPENDENCY_INFO_VERSION(2) };
        let mut used = 0u32;
        let r = unsafe { GetStorageDependencyInformation(h.0, GET_STORAGE_DEPENDENCY_FLAG_HOST_VOLUMES, (buf.len() * 8) as u32, info, Some(&mut used)) };
        if r.0 == 122 && used > size {
            // ERROR_INSUFFICIENT_BUFFER
            size = used;
            continue;
        }
        if r.0 != 0 {
            return vec![];
        }
        let mut out = vec![];
        unsafe {
            let n = (*info).NumberEntries as usize;
            let first = std::ptr::addr_of!((*info).Anonymous.Version2Entries) as *const windows::Win32::Storage::Vhd::STORAGE_DEPENDENCY_INFO_TYPE_2;
            for i in 0..n {
                let e = &*first.add(i);
                let s = |p: windows::core::PWSTR| if p.is_null() { String::new() } else { p.to_string().unwrap_or_default() };
                let (host, rel) = (s(e.HostVolumeName), s(e.DependentVolumeRelativePath));
                if !host.is_empty() && !rel.is_empty() {
                    out.push(Backing { vst: e.VirtualStorageType, level: e.AncestorLevel, host, rel });
                }
            }
        }
        out.sort_by_key(|b| b.level);
        return out;
    }
    vec![]
}

/// The image file mounted as this drive (`C:\dir\file.iso`), if it is a mounted ISO / VHD.
pub fn image_of(root: &str) -> Option<String> {
    let l = letter_of(root).ok()?;
    let h = open_volume(l, 0).ok()?;
    backing(&h).first().map(|b| b.path())
}

/// How this drive is ejected (Explorer's rules): mounted images are dismounted; hot-pluggable USB / FireWire / SD
/// devices are removed safely; optical drives and card readers eject their media; internal disks, network drives
/// and the Windows drive cannot be ejected. `kind` = the drive type from `shell::drives_fast`.
pub fn eject_kind(root: &str, kind: &str) -> Eject {
    let Ok(l) = letter_of(root) else { return Eject::No };
    if !matches!(kind, "fixed" | "removable" | "cdrom") || is_system_drive(l) {
        return Eject::No;
    }
    let Ok(h) = open_volume(l, 0) else {
        return if kind == "fixed" { Eject::No } else { Eject::Media };
    };
    if !backing(&h).is_empty() {
        return Eject::Image;
    }
    let bus = bus_type(&h);
    let (_, device_hotplug) = hotplug(&h).unwrap_or((false, false));
    match kind {
        // a real DVD / CD drive (also a USB one): the disc comes out, as with Explorer's Eject
        "cdrom" => Eject::Media,
        "removable" => {
            if device_hotplug {
                Eject::Device
            } else {
                Eject::Media
            }
        }
        // external USB / FireWire / SD disks; never an internal SATA or NVMe disk (even on a hot-swap port)
        _ => {
            if device_hotplug && matches!(bus, Some(4 | 7 | 12 | 13)) {
                Eject::Device
            } else {
                Eject::No
            }
        }
    }
}

fn in_use(root: &str) -> String {
    format!("{root} is in use. Close the files and programs that use it, then try again.")
}

fn win32(ctx: &str, code: u32) -> String {
    win_err(ctx, &windows::core::Error::from(windows::core::HRESULT::from_win32(code)))
}

/// Lock a volume opened for read/write, trying for about two seconds (the indexer, an antivirus or a thumbnail
/// reader may hold it for a moment).
fn lock(v: &H) -> bool {
    for i in 0..10 {
        if ioctl(v, FSCTL_LOCK_VOLUME, &[], &mut []).is_ok() {
            return true;
        }
        if i < 9 {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    false
}

/// Eject a drive the way Explorer does (see [`eject_kind`]); returns how it was ejected. Open files on it make
/// it fail with a clear "in use" error, and nothing is forced.
pub fn eject(drive: &str) -> Res<Eject> {
    let l = letter_of(drive)?;
    let root = format!("{l}:\\");
    let kind = crate::shell::drive_kind(&root);
    if kind == "none" {
        return Err(format!("{root} is not there any more"));
    }
    if is_system_drive(l) {
        return Err(format!("{root} holds Windows and cannot be ejected"));
    }
    let how = eject_kind(&root, kind);
    match how {
        Eject::Image => eject_image(l, &root)?,
        Eject::Device => return eject_device(l, &root),
        Eject::Media => eject_media(l, &root)?,
        Eject::No => return Err(format!("{root} cannot be ejected (an internal or network drive)")),
    }
    Ok(how)
}

/// Dismount a mounted ISO / VHD: refuse when its files are open, then detach the image (the drive letter goes).
fn eject_image(l: char, root: &str) -> Res<()> {
    use windows::Win32::Storage::Vhd::{DETACH_VIRTUAL_DISK_FLAG_NONE, DetachVirtualDisk, OPEN_VIRTUAL_DISK_FLAG_NONE, OpenVirtualDisk, VIRTUAL_DISK_ACCESS_DETACH};
    let list = {
        let h = open_volume(l, 0).map_err(|e| win_err(&format!("cannot open {root}"), &e))?;
        backing(&h)
    };
    if list.is_empty() {
        return Err(format!("{root} is not a mounted disc image"));
    }
    // in use? Lock it like a safe removal does (a VHD may refuse a normal user the write open: then the detach
    // below decides). The lock is let go before the detach, which would otherwise wait on our own handle.
    if let Ok(v) = open_volume(l, GENERIC_READ.0 | GENERIC_WRITE.0) {
        if !lock(&v) {
            return Err(in_use(root));
        }
        let _ = ioctl(&v, FSCTL_DISMOUNT_VOLUME, &[], &mut []);
    }
    let mut last = 0u32;
    for b in &list {
        let p = wide(b.raw_path());
        let mut h = HANDLE::default();
        let r = unsafe { OpenVirtualDisk(&b.vst, PCWSTR(p.as_ptr()), VIRTUAL_DISK_ACCESS_DETACH, OPEN_VIRTUAL_DISK_FLAG_NONE, None, &mut h) };
        if r.0 != 0 {
            last = r.0;
            continue;
        }
        let h = H(h);
        let r = unsafe { DetachVirtualDisk(h.0, DETACH_VIRTUAL_DISK_FLAG_NONE, 0) };
        if r.0 == 0 {
            return Ok(());
        }
        last = r.0;
    }
    Err(if last == 5 { format!("Windows did not allow ejecting {root} (it needs administrator rights)") } else { win32(&format!("cannot eject {root}"), last) })
}

/// Eject the media of a DVD drive or card reader: lock (refuse when in use), dismount, allow removal, eject. An
/// empty drive just opens its tray.
fn eject_media(l: char, root: &str) -> Res<()> {
    let w = wide(root);
    let ready = unsafe { GetVolumeInformationW(PCWSTR(w.as_ptr()), None, None, None, None, None) }.is_ok();
    if !ready {
        let v = open_volume(l, GENERIC_READ.0).map_err(|e| win_err(&format!("cannot open {root}"), &e))?;
        return ioctl(&v, IOCTL_STORAGE_EJECT_MEDIA, &[], &mut []).map(|_| ()).map_err(|e| win_err(&format!("cannot eject {root}"), &e));
    }
    let v = open_volume(l, GENERIC_READ.0 | GENERIC_WRITE.0).or_else(|_| open_volume(l, GENERIC_READ.0)).map_err(|e| win_err(&format!("cannot open {root}"), &e))?;
    if !lock(&v) {
        return Err(in_use(root));
    }
    ioctl(&v, FSCTL_DISMOUNT_VOLUME, &[], &mut [])
        .and_then(|_| ioctl(&v, IOCTL_STORAGE_MEDIA_REMOVAL, &[0u8], &mut []))
        .and_then(|_| ioctl(&v, IOCTL_STORAGE_EJECT_MEDIA, &[], &mut []))
        .map(|_| ())
        .map_err(|e| win_err(&format!("cannot eject {root}"), &e))
}

/// (device type, device number) of the disk or optical drive a volume is on.
fn device_number(h: &H) -> Option<(u32, u32)> {
    let mut out = [0u8; 12];
    ioctl(h, IOCTL_STORAGE_GET_DEVICE_NUMBER, &[], &mut out).ok()?;
    Some((u32::from_le_bytes(out[0..4].try_into().unwrap()), u32::from_le_bytes(out[4..8].try_into().unwrap())))
}

/// The device node of the disk (or optical drive) with this device number.
fn disk_devnode(dev_type: u32, number: u32) -> Option<u32> {
    use windows::Win32::Devices::DeviceAndDriverInstallation::*;
    use windows::Win32::Devices::Properties::DEVPKEY_Device_InstanceId;
    let guid = if dev_type == 2 { windows::Win32::System::Ioctl::GUID_DEVINTERFACE_CDROM } else { windows::Win32::System::Ioctl::GUID_DEVINTERFACE_DISK };
    unsafe {
        let mut len = 0u32;
        if CM_Get_Device_Interface_List_SizeW(&mut len, &guid, PCWSTR::null(), CM_GET_DEVICE_INTERFACE_LIST_PRESENT) != CR_SUCCESS || len < 2 {
            return None;
        }
        let mut buf = vec![0u16; len as usize];
        if CM_Get_Device_Interface_ListW(&guid, PCWSTR::null(), &mut buf, CM_GET_DEVICE_INTERFACE_LIST_PRESENT) != CR_SUCCESS {
            return None;
        }
        for iface in buf.split(|c| *c == 0).filter(|s| !s.is_empty()) {
            let mut z = iface.to_vec();
            z.push(0);
            let Ok(h) = CreateFileW(PCWSTR(z.as_ptr()), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None).map(H) else { continue };
            if device_number(&h) != Some((dev_type, number)) {
                continue;
            }
            let mut ty = windows::Win32::Devices::Properties::DEVPROPTYPE(0);
            let mut id = vec![0u16; 512];
            let mut size = (id.len() * 2) as u32;
            if CM_Get_Device_Interface_PropertyW(PCWSTR(z.as_ptr()), &DEVPKEY_Device_InstanceId, &mut ty, Some(id.as_mut_ptr() as *mut u8), &mut size, 0) != CR_SUCCESS {
                return None;
            }
            let mut node = 0u32;
            return (CM_Locate_DevNodeW(&mut node, PCWSTR(id.as_ptr()), CM_LOCATE_DEVNODE_NORMAL) == CR_SUCCESS).then_some(node);
        }
    }
    None
}

/// The first device from `node` up that Windows marks as removable (the USB device of a USB stick): the one
/// "Safely Remove Hardware" ejects.
fn removable_ancestor(mut node: u32) -> Option<u32> {
    use windows::Win32::Devices::DeviceAndDriverInstallation::*;
    for _ in 0..16 {
        let mut caps = 0u32;
        let mut len = 4u32;
        let ok = unsafe { CM_Get_DevNode_Registry_PropertyW(node, CM_DRP_CAPABILITIES, None, Some(&mut caps as *mut u32 as *mut _), &mut len, 0) } == CR_SUCCESS;
        if ok && caps & CM_DEVCAP_REMOVABLE.0 != 0 {
            return Some(node);
        }
        let mut parent = 0u32;
        if unsafe { CM_Get_Parent(&mut parent, node, 0) } != CR_SUCCESS {
            return None;
        }
        node = parent;
    }
    None
}

/// Safely remove a USB stick / disk: Windows asks every program to let go of it and refuses (veto) when a file is
/// still open. Without a removable device above the disk (an internal card reader), the media is ejected instead.
fn eject_device(l: char, root: &str) -> Res<Eject> {
    use windows::Win32::Devices::DeviceAndDriverInstallation::{CM_Request_Device_EjectW, CR_SUCCESS, PNP_VETO_TYPE};
    let num = {
        let h = open_volume(l, 0).map_err(|e| win_err(&format!("cannot open {root}"), &e))?;
        device_number(&h)
    };
    let Some(target) = num.and_then(|(t, n)| disk_devnode(t, n)).and_then(removable_ancestor) else {
        eject_media(l, root)?;
        return Ok(Eject::Media);
    };
    for i in 0..3 {
        let mut veto = PNP_VETO_TYPE(0);
        let mut name = [0u16; 260];
        // with a veto buffer Windows shows no message of its own
        let cr = unsafe { CM_Request_Device_EjectW(target, Some(&mut veto), Some(&mut name), 0) };
        if cr == CR_SUCCESS && veto.0 == 0 {
            return Ok(Eject::Device);
        }
        if i < 2 {
            std::thread::sleep(Duration::from_millis(500));
        }
    }
    Err(in_use(root))
}

/// Explorer's "Mount" of a disc image (.iso, and .img when Windows mounts those): attach it read-only for good
/// (until it is ejected or Windows restarts) and return its drive ("G:\"). An image already mounted returns its
/// drive. Waits up to 8 seconds for Windows to give the new drive a letter.
pub fn mount_image(path: &Path) -> Res<String> {
    use windows::Win32::Storage::Vhd::{
        ATTACH_VIRTUAL_DISK_FLAG, AttachVirtualDisk, OPEN_VIRTUAL_DISK_FLAG_NONE, OpenVirtualDisk, VIRTUAL_DISK_ACCESS_READ, VIRTUAL_STORAGE_TYPE,
        VIRTUAL_STORAGE_TYPE_DEVICE_ISO, VIRTUAL_STORAGE_TYPE_VENDOR_MICROSOFT,
    };
    let path = abs(path);
    if !path.is_file() {
        return Err(format!("not found: {}", path.display()));
    }
    if let Some(r) = drive_of_image(&path) {
        return Ok(r);
    }
    let w = wide(path.as_os_str());
    let vst = VIRTUAL_STORAGE_TYPE { DeviceId: VIRTUAL_STORAGE_TYPE_DEVICE_ISO, VendorId: VIRTUAL_STORAGE_TYPE_VENDOR_MICROSOFT };
    let mut h = HANDLE::default();
    let r = unsafe { OpenVirtualDisk(&vst, PCWSTR(w.as_ptr()), VIRTUAL_DISK_ACCESS_READ, OPEN_VIRTUAL_DISK_FLAG_NONE, None, &mut h) };
    if r.0 != 0 {
        return Err(win32(&format!("cannot mount {}", path.display()), r.0));
    }
    let h = H(h);
    let before = unsafe { windows::Win32::Storage::FileSystem::GetLogicalDrives() };
    // READ_ONLY | PERMANENT_LIFETIME: stays mounted after this process ends, as Explorer's Mount
    let r = unsafe { AttachVirtualDisk(h.0, None, ATTACH_VIRTUAL_DISK_FLAG(1 | 4), 0, None, None) };
    drop(h);
    if r.0 != 0 {
        return Err(win32(&format!("cannot mount {}", path.display()), r.0));
    }
    let name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    let until = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(r) = drive_of_image(&path) {
            return Ok(r);
        }
        // the image's path as Windows reports it may not read like ours (opened through a subst or network drive
        // letter): a disc image drive that was not there before, mounted from a file with this name, is it
        let now = unsafe { windows::Win32::Storage::FileSystem::GetLogicalDrives() };
        let new = crate::shell::drives_fast().into_iter().filter(|d| letter_of(&d.root).is_ok_and(|l| (now & !before) & (1 << (l as u8 - b'A')) != 0)).find(|d| {
            image_of(&d.root).is_some_and(|p| Path::new(&p).file_name().is_some_and(|n| n.eq_ignore_ascii_case(&name)))
        });
        if let Some(d) = new {
            return Ok(d.root);
        }
        if Instant::now() > until {
            return Err(format!("{} was mounted, but Windows gave it no drive letter", path.display()));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Same file: the same path, or the same file reached through another name (a subst or mapped drive letter).
fn same_file(a: &Path, b: &Path) -> bool {
    if path_eq(a, b) {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => path_eq(&x, &y),
        _ => false,
    }
}

/// The drive a disc image is mounted as ("G:\"), if it is mounted.
pub fn drive_of_image(path: &Path) -> Option<String> {
    let want = abs(path);
    crate::shell::drives_fast().into_iter().filter(|d| matches!(d.kind, "cdrom" | "fixed" | "removable")).find_map(|d| {
        let l = letter_of(&d.root).ok()?;
        let h = open_volume(l, 0).ok()?;
        backing(&h).iter().any(|b| same_file(Path::new(&b.path()), &want)).then(|| d.root.clone())
    })
}

/// True when Windows' own disc image handler opens this file type (its default action is "Mount"): then the
/// file manager mounts it itself and opens the new drive. Another default app (7-Zip, WinRAR ...) keeps it.
pub fn mounts_by_default(ext: &str) -> bool {
    use windows::Win32::UI::Shell::{ASSOCF_INIT_IGNOREUNKNOWN, ASSOCSTR_PROGID, AssocQueryStringW};
    let e = ext.trim_start_matches('.').to_ascii_lowercase();
    if !matches!(e.as_str(), "iso" | "img") {
        return false;
    }
    let w = wide(format!(".{e}"));
    let mut buf = vec![0u16; 256];
    let mut n = buf.len() as u32;
    let hr = unsafe { AssocQueryStringW(ASSOCF_INIT_IGNOREUNKNOWN, ASSOCSTR_PROGID, PCWSTR(w.as_ptr()), PCWSTR::null(), Some(windows::core::PWSTR(buf.as_mut_ptr())), &mut n) };
    hr.is_ok() && from_wide(&buf).eq_ignore_ascii_case("Windows.IsoFile")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters() {
        assert_eq!(letter_of("e"), Ok('E'));
        assert_eq!(letter_of("E:"), Ok('E'));
        assert_eq!(letter_of(r"g:\"), Ok('G'));
        assert!(letter_of("EE").is_err());
        assert!(letter_of(r"\\server\share").is_err());
        assert!(letter_of("").is_err());
    }

    #[test]
    fn system_drive_is_never_ejectable() {
        let sys = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        let root = format!("{}\\", sys.trim_end_matches('\\'));
        assert_eq!(eject_kind(&root, "fixed"), Eject::No);
        assert_eq!(eject_kind(&root, "removable"), Eject::No);
        assert!(eject(&root).is_err());
    }

    #[test]
    fn network_and_unknown_drives_are_not_ejectable() {
        assert_eq!(eject_kind("Z:\\", "network"), Eject::No);
        assert_eq!(eject_kind("Z:\\", "unknown"), Eject::No);
        assert_eq!(eject_kind("not a drive", "removable"), Eject::No);
    }

    #[test]
    fn the_windows_disk_is_found_in_the_device_tree() {
        // the plumbing "Safely remove" uses: volume -> disk number -> disk device node (and its removable parent)
        let l = std::env::var("SystemDrive").ok().and_then(|s| letter_of(&s).ok()).unwrap_or('C');
        let h = open_volume(l, 0).expect("open the Windows volume");
        let (t, n) = device_number(&h).expect("device number");
        assert_eq!(t, 7, "a disk");
        let node = disk_devnode(t, n).expect("its device node");
        // an internal disk has no removable device above it (a Windows-To-Go USB disk would)
        println!("disk {n}: node {node}, removable parent {:?}", removable_ancestor(node));
        assert!(backing(&h).is_empty(), "not a disc image");
    }

    #[test]
    fn iso_type_check() {
        assert!(!mounts_by_default("txt"));
        assert!(!mounts_by_default("vhdx"));
    }
}
