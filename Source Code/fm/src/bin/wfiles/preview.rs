//! Space = quick preview. Loaded off the UI thread: images and PDF first pages through the shell thumbnail
//! (no decoder in the exe), text and code with simple syntax colours.
use crate::ficons::{Kind, kind_of};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tok {
    Plain,
    Keyword,
    Type,
    Str,
    Comment,
    Number,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PvKind {
    Image,
    Pdf,
    Text,
    Other,
}

#[derive(Clone, Debug)]
pub struct Data {
    pub path: PathBuf,
    pub kind: PvKind,
    /// premultiplied BGRA
    pub img: Option<(u32, u32, Vec<u8>)>,
    pub lines: Vec<Vec<(String, Tok)>>,
    pub meta: String,
    pub pages: u32,
    pub page: u32,
    pub code: bool,
}

/// Straight-alpha BGRA (from fmcore::thumb) to premultiplied.
pub fn premultiply(px: &mut [u8]) {
    for p in px.chunks_exact_mut(4) {
        let a = p[3] as u32;
        if a < 255 {
            p[0] = (p[0] as u32 * a / 255) as u8;
            p[1] = (p[1] as u32 * a / 255) as u8;
            p[2] = (p[2] as u32 * a / 255) as u8;
        }
    }
}

/// Width and height from the image header (PNG, JPEG, GIF, BMP, WebP VP8X).
pub fn image_size(path: &Path) -> Option<(u32, u32)> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut b = vec![0u8; 64 * 1024];
    let n = f.read(&mut b).ok()?;
    let b = &b[..n];
    if b.starts_with(b"\x89PNG") && n >= 24 {
        return Some((u32::from_be_bytes(b[16..20].try_into().ok()?), u32::from_be_bytes(b[20..24].try_into().ok()?)));
    }
    if b.starts_with(b"GIF") && n >= 10 {
        return Some((u16::from_le_bytes([b[6], b[7]]) as u32, u16::from_le_bytes([b[8], b[9]]) as u32));
    }
    if b.starts_with(b"BM") && n >= 26 {
        return Some((i32::from_le_bytes(b[18..22].try_into().ok()?) as u32, i32::from_le_bytes(b[22..26].try_into().ok()?).unsigned_abs()));
    }
    if b.starts_with(b"RIFF") && n >= 30 && &b[8..12] == b"WEBP" && &b[12..16] == b"VP8X" {
        let w = 1 + (b[24] as u32 | (b[25] as u32) << 8 | (b[26] as u32) << 16);
        let h = 1 + (b[27] as u32 | (b[28] as u32) << 8 | (b[29] as u32) << 16);
        return Some((w, h));
    }
    if b.starts_with(&[0xFF, 0xD8]) {
        let mut i = 2;
        while i + 9 < n {
            if b[i] != 0xFF {
                i += 1;
                continue;
            }
            let m = b[i + 1];
            let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
            if (0xC0..=0xCF).contains(&m) && m != 0xC4 && m != 0xC8 && m != 0xCC {
                return Some((u16::from_be_bytes([b[i + 7], b[i + 8]]) as u32, u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32));
            }
            i += 2 + len;
        }
    }
    None
}

fn pdf_pages(path: &Path) -> u32 {
    let Ok(b) = std::fs::read(path) else { return 0 };
    // the biggest "/Count n" is the page tree root
    let mut best = 0u32;
    let pat = b"/Count";
    let mut i = 0;
    while i + pat.len() < b.len() {
        if &b[i..i + pat.len()] == pat {
            let mut j = i + pat.len();
            while j < b.len() && b[j] == b' ' {
                j += 1;
            }
            let mut v = 0u32;
            while j < b.len() && b[j].is_ascii_digit() {
                v = v.saturating_mul(10).saturating_add((b[j] - b'0') as u32);
                j += 1;
            }
            best = best.max(v);
            i = j;
        } else {
            i += 1;
        }
    }
    best
}

const KEYWORDS: &[&str] = &[
    "fn", "pub", "struct", "enum", "impl", "let", "mut", "match", "if", "else", "for", "while", "loop", "return", "use", "mod", "self", "Self", "crate", "const", "static", "trait", "where", "as", "in", "ref", "move", "async",
    "await", "true", "false", "None", "Some", "Ok", "Err", "def", "class", "import", "from", "with", "try", "except", "finally", "raise", "lambda", "yield", "not", "and", "or", "is", "pass", "function", "var",
    "new", "this", "export", "default", "switch", "case", "break", "continue", "null", "undefined", "typeof", "interface", "type", "public", "private", "void", "int", "float", "double", "bool", "string",
    "package", "func", "go", "defer", "range", "elif", "global", "nonlocal", "del", "assert", "unsafe", "dyn", "extern",
];

fn is_code(ext: &str) -> bool {
    matches!(ext, "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "c" | "cpp" | "h" | "hpp" | "cs" | "java" | "kt" | "go" | "swift" | "rb" | "php" | "lua" | "dart" | "sh" | "ps1" | "json" | "jsonl" | "toml" | "yml" | "yaml" | "css" | "html" | "xml" | "sql" | "ipynb" | "bat" | "cmd")
}

/// Split one line into coloured tokens. `in_block` carries a /* ... */ comment across lines.
pub fn tokenize(line: &str, ext: &str, in_block: &mut bool) -> Vec<(String, Tok)> {
    let hash_comment = matches!(ext, "py" | "sh" | "ps1" | "toml" | "yml" | "yaml" | "rb");
    let ch: Vec<char> = line.chars().collect();
    let mut out: Vec<(String, Tok)> = vec![];
    let push = |out: &mut Vec<(String, Tok)>, s: String, t: Tok| {
        if let Some(last) = out.last_mut()
            && last.1 == t
        {
            last.0.push_str(&s);
            return;
        }
        out.push((s, t));
    };
    let mut i = 0;
    while i < ch.len() {
        if *in_block {
            let start = i;
            while i < ch.len() && !(ch[i] == '*' && i + 1 < ch.len() && ch[i + 1] == '/') {
                i += 1;
            }
            if i < ch.len() {
                i += 2;
                *in_block = false;
            }
            push(&mut out, ch[start..i.min(ch.len())].iter().collect(), Tok::Comment);
            continue;
        }
        let c = ch[i];
        if (c == '/' && i + 1 < ch.len() && ch[i + 1] == '/') || (hash_comment && c == '#') {
            push(&mut out, ch[i..].iter().collect(), Tok::Comment);
            break;
        }
        if c == '/' && i + 1 < ch.len() && ch[i + 1] == '*' {
            *in_block = true;
            continue;
        }
        if c == '"' || c == '\'' || c == '`' {
            // a Rust lifetime like 'a is not a string
            if c == '\'' && ext == "rs" && i + 2 < ch.len() && ch[i + 2] != '\'' && ch[i + 1].is_alphabetic() {
                push(&mut out, c.to_string(), Tok::Plain);
                i += 1;
                continue;
            }
            let start = i;
            i += 1;
            while i < ch.len() && ch[i] != c {
                if ch[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(ch.len());
            push(&mut out, ch[start..i].iter().collect(), Tok::Str);
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            while i < ch.len() && (ch[i].is_ascii_alphanumeric() || ch[i] == '.' || ch[i] == '_') {
                i += 1;
            }
            push(&mut out, ch[start..i].iter().collect(), Tok::Number);
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < ch.len() && (ch[i].is_alphanumeric() || ch[i] == '_') {
                i += 1;
            }
            let w: String = ch[start..i].iter().collect();
            let t = if KEYWORDS.contains(&w.as_str()) {
                Tok::Keyword
            } else if w.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) || matches!(w.as_str(), "u8" | "u16" | "u32" | "u64" | "i32" | "i64" | "f32" | "f64" | "usize" | "isize" | "str") {
                Tok::Type
            } else {
                Tok::Plain
            };
            push(&mut out, w, t);
            continue;
        }
        push(&mut out, c.to_string(), Tok::Plain);
        i += 1;
    }
    out
}

/// One PDF page with Windows' own PDF renderer (works without any PDF app): premultiplied BGRA + page count.
pub fn pdf_render(path: &Path, page: u32, width: u32) -> windows::core::Result<(u32, u32, Vec<u8>, u32)> {
    use windows::Data::Pdf::*;
    use windows::Graphics::Imaging::*;
    use windows::Storage::Streams::InMemoryRandomAccessStream;
    use windows::Storage::StorageFile;
    let file = StorageFile::GetFileFromPathAsync(&windows::core::HSTRING::from(path.as_os_str()))?.get()?;
    let doc = PdfDocument::LoadFromFileAsync(&file)?.get()?;
    let count = doc.PageCount()?;
    let pg = doc.GetPage(page.min(count.saturating_sub(1)))?;
    let stream = InMemoryRandomAccessStream::new()?;
    let opts = PdfPageRenderOptions::new()?;
    opts.SetDestinationWidth(width)?;
    pg.RenderWithOptionsToStreamAsync(&stream, &opts)?.get()?;
    let dec = BitmapDecoder::CreateAsync(&stream)?.get()?;
    let (w, h) = (dec.PixelWidth()?, dec.PixelHeight()?);
    let pd = dec.GetPixelDataTransformedAsync(BitmapPixelFormat::Bgra8, BitmapAlphaMode::Premultiplied, &BitmapTransform::new()?, ExifOrientationMode::IgnoreExifOrientation, ColorManagementMode::DoNotColorManage)?.get()?;
    let px = pd.DetachPixelData()?;
    Ok((w, h, px.to_vec(), count))
}

pub fn load_page(path: &Path, page: u32) -> Data {
    let mut d = load(path, false);
    if let Ok((w, h, px, n)) = pdf_render(path, page, 1100) {
        d.img = Some((w, h, px));
        d.pages = n;
        d.page = page.min(n.saturating_sub(1));
        d.kind = PvKind::Pdf;
    }
    d
}

pub fn load(path: &Path, is_dir: bool) -> Data {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let kind = kind_of(&name, is_dir);
    let ext = fmcore::util::ext_of(&name);
    let md = std::fs::metadata(path).ok();
    let size = md.as_ref().map(|m| m.len()).unwrap_or(0);
    let mtime = md.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| fmcore::util::unix_ft(d.as_secs() as i64)).unwrap_or(0);
    let label = crate::ficons::type_label(&name, is_dir);
    let when = crate::fmt::date(mtime, false);
    let mut d = Data { path: path.to_path_buf(), kind: PvKind::Other, img: None, lines: vec![], meta: String::new(), pages: 0, page: 0, code: false };
    let thumb = |px: u32, k: fmcore::thumb::ThumbKind| -> Option<(u32, u32, Vec<u8>)> {
        let mut img = fmcore::thumb::thumbnail(path, px, k).ok()?;
        premultiply(&mut img.bgra);
        Some((img.width, img.height, img.bgra))
    };
    match kind {
        Kind::Img => {
            d.kind = PvKind::Image;
            d.img = thumb(1024, fmcore::thumb::ThumbKind::Auto);
            let dims = image_size(path).map(|(w, h)| format!(" · {w} × {h}")).unwrap_or_default();
            d.meta = format!("{label}{dims} · {} · {when}", crate::fmt::size(size));
        }
        Kind::Pdf => {
            d.kind = PvKind::Pdf;
            match pdf_render(path, 0, 1100) {
                Ok((w, h, px, n)) => {
                    d.img = Some((w, h, px));
                    d.pages = n;
                }
                Err(_) => {
                    d.img = thumb(1024, fmcore::thumb::ThumbKind::ThumbnailOnly);
                    d.pages = pdf_pages(path);
                }
            }
            let pages = if d.pages > 0 { format!(" · {} page{}", d.pages, if d.pages == 1 { "" } else { "s" }) } else { String::new() };
            d.meta = format!("PDF{pages} · {} · {when}", crate::fmt::size(size));
            if d.img.is_none() {
                d.kind = PvKind::Other;
                d.img = thumb(256, fmcore::thumb::ThumbKind::IconOnly);
            }
        }
        _ if !is_dir && (matches!(kind, Kind::Code | Kind::Txt | Kind::Md | Kind::Nb) || is_code(&ext)) && !matches!(ext.as_str(), "doc" | "docx" | "rtf" | "odt") => {
            use std::io::Read;
            d.kind = PvKind::Text;
            d.code = is_code(&ext);
            let mut buf = vec![0u8; 96 * 1024];
            let n = std::fs::File::open(path).and_then(|mut f| f.read(&mut buf)).unwrap_or(0);
            let text = if buf.starts_with(&[0xFF, 0xFE]) {
                let w: Vec<u16> = buf[2..n].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                String::from_utf16_lossy(&w)
            } else {
                String::from_utf8_lossy(&buf[..n]).into_owned()
            };
            let total_lines = if n < buf.len() { text.lines().count() } else { 0 };
            let mut block = false;
            for l in text.lines().take(600) {
                let l = l.replace('\t', "    ");
                d.lines.push(if d.code { tokenize(&l, &ext, &mut block) } else { vec![(l, Tok::Plain)] });
            }
            let lines = if total_lines > 0 { format!(" · {total_lines} lines") } else { String::new() };
            d.meta = format!("{label} · {}{lines} · {when}", crate::fmt::size(size));
        }
        _ => {
            d.img = thumb(256, if matches!(kind, Kind::Video) { fmcore::thumb::ThumbKind::Auto } else { fmcore::thumb::ThumbKind::IconOnly });
            if matches!(kind, Kind::Video) && d.img.as_ref().map(|i| i.0 > 200).unwrap_or(false) {
                d.kind = PvKind::Image;
            }
            d.meta = if is_dir { format!("Folder · {when}") } else { format!("{label} · {} · {when}", crate::fmt::size(size)) };
        }
    }
    d
}
