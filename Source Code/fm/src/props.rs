//! File properties through the Windows property system: the same property handlers Explorer's Details pane uses.
//! [`read`] gives a short list for the kind of file (music, picture, video, document) plus type, size and dates;
//! [`set`] changes one writable value (music tags, title, subject, tags, authors, comments) and commits it through
//! the file type's own handler, which writes the file safely (as Explorer does).
use crate::util::*;
use std::path::Path;
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::System::Com::StructuredStorage::{InitPropVariantFromStringVector, PROPVARIANT, PropVariantToStringAlloc};
use windows::Win32::System::Variant::VT_EMPTY;
use windows::Win32::UI::Shell::PropertiesSystem::*;
use windows::core::{Interface, PCWSTR};

/// One property row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Prop {
    /// canonical name ("System.Music.Artist"): what [`set`] takes
    pub key: String,
    pub label: String,
    /// as Explorer shows it ("Artist A; Artist B", "00:03:12", "1920 x 1080")
    pub value: String,
    /// the text to edit (several values joined by "; ")
    pub raw: String,
    pub writable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Music,
    Picture,
    Video,
    Document,
    Other,
}

pub fn group_of(name: &str) -> Group {
    match ext_of(name).as_str() {
        "mp3" | "m4a" | "wma" | "flac" | "wav" | "ogg" | "opus" | "aac" | "aiff" | "aif" => Group::Music,
        "jpg" | "jpeg" | "jfif" | "png" | "gif" | "bmp" | "tif" | "tiff" | "heic" | "heif" | "webp" | "jxr" | "dng" => Group::Picture,
        "mp4" | "m4v" | "mkv" | "mov" | "avi" | "wmv" | "webm" | "3gp" | "ts" => Group::Video,
        "docx" | "doc" | "xlsx" | "xls" | "pptx" | "ppt" | "pdf" | "odt" | "ods" | "odp" | "rtf" | "docm" | "xlsm" | "pptm" => Group::Document,
        _ => Group::Other,
    }
}

/// (canonical name, label) in the order shown; the writable ones are offered for editing.
pub fn keys_for(g: Group) -> &'static [(&'static str, &'static str)] {
    match g {
        Group::Music => &[
            ("System.Title", "Title"),
            ("System.Music.Artist", "Artists"),
            ("System.Music.AlbumTitle", "Album"),
            ("System.Music.AlbumArtist", "Album artist"),
            ("System.Media.Year", "Year"),
            ("System.Music.Genre", "Genre"),
            ("System.Music.TrackNumber", "Track"),
            ("System.Comment", "Comments"),
            ("System.Media.Duration", "Length"),
            ("System.Audio.EncodingBitrate", "Bit rate"),
        ],
        Group::Picture => &[
            ("System.Title", "Title"),
            ("System.Subject", "Subject"),
            ("System.Keywords", "Tags"),
            ("System.Author", "Authors"),
            ("System.Comment", "Comments"),
            ("System.Image.Dimensions", "Dimensions"),
            ("System.Photo.DateTaken", "Date taken"),
            ("System.Photo.CameraModel", "Camera model"),
        ],
        Group::Video => &[
            ("System.Title", "Title"),
            ("System.Media.SubTitle", "Subtitle"),
            ("System.Video.Director", "Directors"),
            ("System.Media.Year", "Year"),
            ("System.Music.Genre", "Genre"),
            ("System.Keywords", "Tags"),
            ("System.Comment", "Comments"),
            ("System.Media.Duration", "Length"),
            ("System.Video.FrameWidth", "Frame width"),
            ("System.Video.FrameHeight", "Frame height"),
        ],
        Group::Document => &[
            ("System.Title", "Title"),
            ("System.Subject", "Subject"),
            ("System.Keywords", "Tags"),
            ("System.Author", "Authors"),
            ("System.Comment", "Comments"),
            ("System.Document.PageCount", "Pages"),
        ],
        Group::Other => &[("System.Title", "Title"), ("System.Keywords", "Tags"), ("System.Author", "Authors"), ("System.Comment", "Comments")],
    }
}

/// Always shown last, never editable.
const COMMON: &[(&str, &str)] = &[("System.ItemTypeText", "Type"), ("System.Size", "Size"), ("System.DateCreated", "Date created"), ("System.DateModified", "Date modified")];

fn key_of(name: &str) -> Res<PROPERTYKEY> {
    let mut k = PROPERTYKEY::default();
    let w = wide(name);
    unsafe { PSGetPropertyKeyFromName(PCWSTR(w.as_ptr()), &mut k) }.map_err(|e| win_err(&format!("unknown property {name}"), &e))?;
    Ok(k)
}

fn description(k: &PROPERTYKEY) -> Option<IPropertyDescription> {
    let mut p = std::ptr::null_mut();
    unsafe { PSGetPropertyDescription(k, &IPropertyDescription::IID, &mut p) }.ok()?;
    (!p.is_null()).then(|| unsafe { IPropertyDescription::from_raw(p) })
}

fn store(path: &Path, flags: GETPROPERTYSTOREFLAGS) -> Res<IPropertyStore> {
    let w = wide(abs(path).as_os_str());
    unsafe { SHGetPropertyStoreFromParsingName::<_, _, IPropertyStore>(PCWSTR(w.as_ptr()), None, flags) }.map_err(|e| win_err(&format!("cannot read the properties of {}", path.display()), &e))
}

fn text_of(pv: &PROPVARIANT) -> String {
    if pv.vt() == VT_EMPTY {
        return String::new();
    }
    unsafe { PropVariantToStringAlloc(pv) }.map(take_pwstr).unwrap_or_default()
}

fn shown(k: &PROPERTYKEY, pv: &PROPVARIANT) -> String {
    if pv.vt() == VT_EMPTY {
        return String::new();
    }
    let s = unsafe { PSFormatForDisplayAlloc(k, pv, PDFF_DEFAULT) }.map(take_pwstr).unwrap_or_else(|_| text_of(pv));
    // Windows wraps dates and sizes in direction marks for its own controls; they only get in the way here
    s.chars().filter(|c| !matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}')).collect()
}

/// The properties of a file to show (a value only when it has one; writable ones also when empty, to fill in).
pub fn read(path: &Path) -> Res<Vec<Prop>> {
    let _c = com();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let is_dir = path.is_dir();
    let st = store(path, GPS_BESTEFFORT)?;
    // which values this file's handler can change (the same check Explorer's pane makes)
    let caps: Option<IPropertyStoreCapabilities> = st.cast().ok();
    let mut out = vec![];
    let list = if is_dir { &[][..] } else { keys_for(group_of(&name)) };
    for (key, label) in list.iter().chain(COMMON.iter()) {
        let Ok(k) = key_of(key) else { continue };
        let pv = unsafe { st.GetValue(&k) }.unwrap_or_default();
        let common = COMMON.iter().any(|c| c.0 == *key);
        let d = description(&k);
        let innate = d.as_ref().and_then(|d| unsafe { d.GetTypeFlags(PDTF_ISINNATE) }.ok()).map(|f| f.0 != 0).unwrap_or(true);
        // only text and whole numbers are typed in (dates and other kinds stay as they are)
        let typed = d.as_ref().and_then(|d| unsafe { d.GetPropertyType() }.ok()).map(|vt| matches!(vt & 0x0FFF, 8 | 31 | 18 | 19 | 21 | 3)).unwrap_or(false);
        let writable = !common && !innate && typed && caps.as_ref().map(|c| unsafe { c.IsPropertyWritable(&k) }.0 == 0).unwrap_or(false);
        let value = shown(&k, &pv);
        if value.is_empty() && !writable {
            continue;
        }
        if *key == "System.Size" && is_dir {
            continue;
        }
        out.push(Prop { key: key.to_string(), label: label.to_string(), raw: text_of(&pv), value, writable });
    }
    Ok(out)
}

/// The value a typed text becomes for this property: empty clears it, several values are split at ";", numbers must be
/// numbers. (Pure; `set` uses it.)
pub fn parse_input(text: &str, multi: bool, number: bool) -> Result<Option<Vec<String>>, String> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(None);
    }
    if number {
        return t.parse::<u32>().map(|n| Some(vec![n.to_string()])).map_err(|_| "needs a whole number".to_string());
    }
    if multi {
        let v: Vec<String> = t.split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        return Ok(if v.is_empty() { None } else { Some(v) });
    }
    Ok(Some(vec![t.to_string()]))
}

/// Change one property (canonical name) to the typed text and save it into the file. Empty text removes the value.
pub fn set(path: &Path, key: &str, text: &str) -> Res<()> {
    let _c = com();
    let k = key_of(key)?;
    let d = description(&k).ok_or_else(|| format!("unknown property {key}"))?;
    let multi = unsafe { d.GetTypeFlags(PDTF_MULTIPLEVALUES) }.map(|f| f.0 != 0).unwrap_or(false);
    let vt = unsafe { d.GetPropertyType() }.unwrap_or(0);
    // VT_UI2 / VT_UI4 / VT_UI8 / VT_I4: years, track numbers
    let number = matches!(vt, 18 | 19 | 21 | 3);
    let mut pv = match parse_input(text, multi, number)? {
        None => PROPVARIANT::default(),
        Some(v) if multi => {
            let w: Vec<Vec<u16>> = v.iter().map(wide).collect();
            let p: Vec<PCWSTR> = w.iter().map(|x| PCWSTR(x.as_ptr())).collect();
            unsafe { InitPropVariantFromStringVector(Some(&p)) }.map_err(|e| win_err("bad value", &e))?
        }
        Some(v) if number => PROPVARIANT::from(v[0].parse::<u32>().unwrap_or(0)),
        Some(v) => PROPVARIANT::from(v[0].as_str()),
    };
    if pv.vt() != VT_EMPTY {
        // the type the property really has (a string becomes VT_LPWSTR, a number VT_UI2 ...)
        unsafe { PSCoerceToCanonicalValue(&k, &mut pv) }.map_err(|e| win_err(&format!("this value does not fit {key}"), &e))?;
    }
    let st = store(path, GPS_READWRITE).map_err(|e| if e.contains("0x80030020") || e.to_lowercase().contains("used by another") { format!("{} is open in another app", path.display()) } else { e })?;
    if let Ok(caps) = st.cast::<IPropertyStoreCapabilities>()
        && unsafe { caps.IsPropertyWritable(&k) }.0 != 0
    {
        return Err(format!("{key} cannot be changed in this file"));
    }
    unsafe { st.SetValue(&k, &pv) }.map_err(|e| win_err("cannot change the value", &e))?;
    unsafe { st.Commit() }.map_err(|e| win_err("cannot save the file", &e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_by_extension() {
        assert_eq!(group_of("song.MP3"), Group::Music);
        assert_eq!(group_of("a.jpeg"), Group::Picture);
        assert_eq!(group_of("clip.mkv"), Group::Video);
        assert_eq!(group_of("report.docx"), Group::Document);
        assert_eq!(group_of("notes"), Group::Other);
        assert!(keys_for(Group::Music).iter().any(|k| k.0 == "System.Music.Artist"));
    }

    #[test]
    fn typed_text_becomes_values() {
        assert_eq!(parse_input("  ", false, false), Ok(None));
        assert_eq!(parse_input(" Hello ", false, false), Ok(Some(vec!["Hello".into()])));
        assert_eq!(parse_input("A; B ;;C", true, false), Ok(Some(vec!["A".into(), "B".into(), "C".into()])));
        assert_eq!(parse_input(" ; ", true, false), Ok(None));
        assert_eq!(parse_input("2024", false, true), Ok(Some(vec!["2024".into()])));
        assert!(parse_input("20x4", false, true).is_err());
    }

    /// Live round trip on a copy (cargo test props_live -- --ignored --nocapture; WFM_PROPS_FILE = an mp3/jpg/docx).
    #[test]
    #[ignore]
    fn props_live_round_trip() {
        let Ok(src) = std::env::var("WFM_PROPS_FILE") else { return };
        let src = std::path::PathBuf::from(src);
        let copy = std::env::temp_dir().join(format!("wfm-props-test.{}", ext_of(&src.to_string_lossy())));
        std::fs::copy(&src, &copy).unwrap();
        let before = read(&copy).unwrap();
        println!("{before:#?}");
        let title = before.iter().find(|p| p.key == "System.Title").expect("title row");
        assert!(title.writable, "title writable");
        set(&copy, "System.Title", "Round trip ✓ تست").unwrap();
        let after = read(&copy).unwrap();
        assert_eq!(after.iter().find(|p| p.key == "System.Title").unwrap().raw, "Round trip ✓ تست");
        set(&copy, "System.Title", "").unwrap();
        assert_eq!(read(&copy).unwrap().iter().find(|p| p.key == "System.Title").map(|p| p.raw.clone()).unwrap_or_default(), "");
        let _ = std::fs::remove_file(&copy);
    }
}
