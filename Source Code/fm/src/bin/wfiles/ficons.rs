//! File-type icons as Direct2D geometry (the mockup's 20 x 20 SVG shapes), drawn at any size.
use crate::gfx::{C, Gfx, P, Rect};
use crate::theme::Theme;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Kind {
    Folder,
    Drive,
    Zip,
    Img,
    Code,
    Nb,
    Pdf,
    Txt,
    Md,
    Exe,
    Video,
    Audio,
    Lnk,
    File,
}

pub fn kind_of(name: &str, is_dir: bool) -> Kind {
    if is_dir {
        return Kind::Folder;
    }
    let ext = match name.rfind('.') {
        Some(i) if i + 1 < name.len() => name[i + 1..].to_ascii_lowercase(),
        _ => return Kind::File,
    };
    match ext.as_str() {
        "zip" | "rar" | "tgz" | "7z" | "xz" | "gz" | "whl" | "tar" | "bz2" | "zst" | "cab" | "iso" | "jar" | "apk" => Kind::Zip,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tif" | "tiff" | "heic" | "avif" | "svg" => Kind::Img,
        "json" | "jsonl" | "rs" | "py" | "yml" | "yaml" | "js" | "ts" | "tsx" | "jsx" | "html" | "htm" | "css" | "c" | "cpp" | "h" | "hpp" | "cs" | "java" | "kt" | "go" | "toml" | "xml" | "sh" | "ps1"
        | "bat" | "cmd" | "sql" | "lua" | "dart" | "swift" | "rb" | "php" | "ini" | "cfg" | "lock" => Kind::Code,
        "ipynb" => Kind::Nb,
        "pdf" => Kind::Pdf,
        "txt" | "log" | "csv" | "tsv" | "doc" | "docx" | "rtf" | "odt" | "srt" | "vtt" => Kind::Txt,
        "md" | "markdown" => Kind::Md,
        "exe" | "msi" | "msix" | "appx" => Kind::Exe,
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "wmv" | "m4v" => Kind::Video,
        "mp3" | "wav" | "flac" | "m4a" | "ogg" | "opus" | "aac" | "wma" => Kind::Audio,
        "lnk" | "url" => Kind::Lnk,
        _ => Kind::File,
    }
}

fn rr(g: &Gfx, ox: f32, oy: f32, k: f32, x: f32, y: f32, w: f32, h: f32, rad: f32, c: C) {
    g.fill_round(Rect::new(ox + x * k, oy + y * k, w * k, h * k), rad * k, c);
}

/// The folder's back (with its tab), in the square `r`.
pub fn folder_back(g: &Gfx, t: &Theme, r: Rect) {
    g.fill_path(
        &[P::M(1.5, 4.2), P::B(1.5, 3.6, 1.9, 3.2, 2.5, 3.2), P::L(7.6, 3.2), P::L(9.5, 5.1), P::L(17.5, 5.1), P::B(18.1, 5.1, 18.5, 5.5, 18.5, 6.1), P::L(18.5, 15.7), P::B(18.5, 16.3, 18.1, 16.7, 17.5, 16.7), P::L(2.5, 16.7), P::B(1.9, 16.7, 1.5, 16.3, 1.5, 15.7), P::Z],
        r.x,
        r.y,
        r.w / 20.0,
        t.folder_back,
    );
}

/// The folder's front, its top edge at `top` (of 20; 6.4 for the plain icon, lower when pictures peek out).
pub fn folder_front(g: &Gfx, t: &Theme, r: Rect, top: f32) {
    g.fill_path(
        &[P::M(1.5, top + 0.9), P::B(1.5, top + 0.4, 1.9, top, 2.4, top), P::L(17.6, top), P::B(18.1, top, 18.5, top + 0.4, 18.5, top + 0.9), P::L(18.5, 15.7), P::B(18.5, 16.3, 18.1, 16.7, 17.5, 16.7), P::L(2.5, 16.7), P::B(1.9, 16.7, 1.5, 16.3, 1.5, 15.7), P::Z],
        r.x,
        r.y,
        r.w / 20.0,
        t.folder,
    );
}

/// Draw the icon in the square `r` (its size is r.w).
pub fn draw(g: &Gfx, t: &Theme, kind: Kind, r: Rect) {
    let k = r.w / 20.0;
    let (ox, oy) = (r.x, r.y);
    match kind {
        Kind::Folder => {
            folder_back(g, t, r);
            folder_front(g, t, r, 6.4);
            return;
        }
        Kind::Drive => {
            rr(g, ox, oy, k, 1.5, 6.5, 17.0, 7.0, 2.0, t.page_stroke);
            rr(g, ox, oy, k, 2.5, 7.5, 15.0, 5.0, 1.4, t.page);
            g.ellipse(ox + 14.5 * k, oy + 10.0 * k, 0.9 * k, t.page_stroke, true, 1.0);
            return;
        }
        _ => {}
    }
    let page = [P::M(4.6, 1.5), P::L(11.7, 1.5), P::L(16.0, 5.8), P::L(16.0, 17.5), P::B(16.0, 18.1, 15.6, 18.5, 15.0, 18.5), P::L(4.6, 18.5), P::B(4.0, 18.5, 3.6, 18.1, 3.6, 17.5), P::L(3.6, 2.5), P::B(3.6, 1.9, 4.0, 1.5, 4.6, 1.5), P::Z];
    g.fill_path(&page, ox, oy, k, t.page);
    g.stroke_path(&page, ox, oy, k, t.page_stroke, k.max(1.0));
    g.stroke_path(&[P::M(11.5, 1.6), P::L(11.5, 4.9), P::B(11.5, 5.5, 11.9, 5.9, 12.5, 5.9), P::L(15.9, 5.9)], ox, oy, k, t.page_stroke, k.max(1.0));
    let lw = |w: f32| (w * k).max(1.0);
    match kind {
        Kind::Zip => {
            let c = C::hex(0xC98A12);
            rr(g, ox, oy, k, 8.6, 2.0, 1.8, 1.6, 0.0, c);
            rr(g, ox, oy, k, 7.0, 3.6, 1.8, 1.6, 0.0, c);
            rr(g, ox, oy, k, 8.6, 5.2, 1.8, 1.6, 0.0, c);
            rr(g, ox, oy, k, 7.0, 6.8, 1.8, 1.6, 0.0, c);
            rr(g, ox, oy, k, 7.0, 9.4, 3.4, 4.4, 0.8, c);
        }
        Kind::Img => {
            g.ellipse(ox + 7.4 * k, oy + 9.2 * k, 1.4 * k, C::hex(0xE0A21B), true, 1.0);
            g.fill_path(&[P::M(5.0, 16.2), P::L(8.6, 11.6), P::L(11.0, 14.4), P::L(12.4, 12.8), P::L(15.0, 16.2), P::Z], ox, oy, k, C::hex(0x7C5CD6));
        }
        Kind::Code => {
            let c = C::hex(0x2E7DD1);
            g.stroke_path(&[P::M(8.0, 9.5), P::L(5.8, 11.8), P::L(8.0, 14.1), P::M(12.0, 9.5), P::L(14.2, 11.8), P::L(12.0, 14.1)], ox, oy, k, c, lw(1.5));
        }
        Kind::Nb => {
            rr(g, ox, oy, k, 5.2, 9.0, 9.6, 6.6, 1.0, C::hex(0xE8742A));
            g.ellipse(ox + 10.0 * k, oy + 12.3 * k, 1.6 * k, C::hex(0xFBE3D2), true, 1.0);
        }
        Kind::Pdf => {
            rr(g, ox, oy, k, 5.2, 10.4, 9.6, 5.0, 1.0, C::hex(0xD13438));
            g.line(ox + 6.8 * k, oy + 12.9 * k, ox + 13.2 * k, oy + 12.9 * k, C::hex(0xFBE0E1), lw(1.2));
        }
        Kind::Txt | Kind::File => {
            let c = C::hex(0x7F8894);
            if kind == Kind::Txt {
                g.stroke_path(&[P::M(6.2, 9.4), P::L(13.8, 9.4), P::M(6.2, 11.8), P::L(13.8, 11.8), P::M(6.2, 14.2), P::L(11.2, 14.2)], ox, oy, k, c, lw(1.1));
            }
        }
        Kind::Md => {
            g.stroke_path(&[P::M(6.0, 14.6), P::L(6.0, 10.0), P::L(8.0, 12.4), P::L(10.0, 10.0), P::L(10.0, 14.6), P::M(12.6, 10.0), P::L(12.6, 14.4), P::M(11.2, 13.2), P::L(12.6, 14.7), P::L(14.0, 13.2)], ox, oy, k, C::hex(0x4E5968), lw(1.1));
        }
        Kind::Exe => {
            rr(g, ox, oy, k, 5.2, 8.6, 9.6, 7.2, 1.2, C::hex(0x2F6FCF));
            g.stroke_path(&[P::M(10.0, 9.9), P::L(10.0, 13.9), P::M(8.2, 12.3), P::L(10.0, 14.1), P::L(11.8, 12.3)], ox, oy, k, C::hex(0xE6F0FF), lw(1.2));
        }
        Kind::Video => {
            rr(g, ox, oy, k, 5.2, 9.0, 9.6, 6.8, 1.2, C::hex(0xC2387A));
            g.fill_path(&[P::M(9.0, 10.7), P::L(9.0, 14.1), P::L(11.8, 12.4), P::Z], ox, oy, k, C::hex(0xFBE4EF));
        }
        Kind::Audio => {
            rr(g, ox, oy, k, 5.2, 9.0, 9.6, 6.8, 1.2, C::hex(0x1F8F84));
            g.stroke_path(&[P::M(9.0, 14.0), P::L(9.0, 10.4), P::L(12.0, 10.0), P::L(12.0, 13.6)], ox, oy, k, C::hex(0xE2F6F3), lw(1.1));
            g.ellipse(ox + 8.4 * k, oy + 14.0 * k, 0.9 * k, C::hex(0xE2F6F3), true, 1.0);
            g.ellipse(ox + 11.4 * k, oy + 13.6 * k, 0.9 * k, C::hex(0xE2F6F3), true, 1.0);
        }
        Kind::Lnk => {
            g.line(ox + 6.4 * k, oy + 11.4 * k, ox + 11.4 * k, oy + 11.4 * k, C::hex(0x7F8894), lw(1.1));
            rr(g, ox, oy, k, 3.2, 12.6, 6.2, 6.2, 1.2, t.page_stroke);
            rr(g, ox, oy, k, 3.7, 13.1, 5.2, 5.2, 0.9, t.page);
            g.stroke_path(&[P::M(5.0, 17.0), P::L(7.6, 14.4), P::M(5.6, 14.4), P::L(7.6, 14.4), P::L(7.6, 16.4)], ox, oy, k, C::hex(0x2E7DD1), lw(1.1));
        }
        _ => {}
    }
}

/// Human type name for the preview header and "What is this file?".
pub fn type_label(name: &str, is_dir: bool) -> String {
    if is_dir {
        return "Folder".into();
    }
    let ext = name.rfind('.').map(|i| name[i + 1..].to_ascii_lowercase()).unwrap_or_default();
    let s = match ext.as_str() {
        "png" => "PNG image",
        "jpg" | "jpeg" => "JPEG image",
        "gif" => "GIF image",
        "webp" => "WebP image",
        "pdf" => "PDF",
        "rs" => "Rust source",
        "py" => "Python source",
        "js" => "JavaScript",
        "ts" => "TypeScript",
        "json" => "JSON",
        "jsonl" => "JSON lines",
        "md" => "Markdown",
        "txt" => "Text",
        "zip" => "ZIP archive",
        "xz" => "XZ archive",
        "tgz" | "gz" => "GZip archive",
        "7z" => "7-Zip archive",
        "rar" => "RAR archive",
        "exe" => "Application",
        "msi" => "Installer",
        "ipynb" => "Jupyter notebook",
        "mp4" => "MP4 video",
        "mkv" => "MKV video",
        "mp3" => "MP3 audio",
        "wav" => "WAV audio",
        "lnk" => "Shortcut",
        "url" => "Internet shortcut",
        "" => "File",
        _ => "",
    };
    if s.is_empty() { format!("{} file", ext.to_uppercase()) } else { s.to_string() }
}
