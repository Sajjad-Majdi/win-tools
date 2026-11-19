//! "What is this file?" without AI: a built-in table of file types (short description + the usual app) and the
//! raw local facts (type, size, dates, full path, publisher of programs, archive entry count). Small offline
//! models invent facts, so the AI is asked only for types this table does not know, and its text never replaces
//! the facts.
use std::path::Path;

/// (extensions, description, usual app)
const TABLE: &[(&[&str], &str, &str)] = &[
    (&["pdf"], "A PDF document: pages laid out for reading and printing.", "a PDF reader or any web browser"),
    (&["doc", "docx", "odt", "rtf"], "A word-processor document (text with formatting).", "Microsoft Word or LibreOffice Writer"),
    (&["xls", "xlsx", "ods"], "A spreadsheet: tables of numbers and formulas.", "Microsoft Excel or LibreOffice Calc"),
    (&["ppt", "pptx", "odp"], "A slide presentation.", "Microsoft PowerPoint or LibreOffice Impress"),
    (&["txt", "log"], "Plain text with no formatting.", "Notepad or any text editor"),
    (&["md", "markdown"], "Markdown: plain text with simple marks for headings, lists and links.", "a code editor or any text editor"),
    (&["json"], "JSON data: structured settings or data as text.", "a code editor"),
    (&["jsonl", "ndjson"], "JSON lines: one JSON record per line, often a dataset or a log.", "a code editor"),
    (&["csv", "tsv"], "A table as plain text, one row per line.", "Excel, LibreOffice Calc or a text editor"),
    (&["py"], "Python source code.", "Python (to run it) or a code editor"),
    (&["rs"], "Rust source code.", "a code editor; built with cargo"),
    (&["js", "mjs", "cjs"], "JavaScript source code.", "a code editor; runs in a browser or Node.js"),
    (&["ts", "tsx"], "TypeScript source code.", "a code editor"),
    (&["html", "htm"], "A web page.", "any web browser"),
    (&["css"], "Styles for web pages.", "a code editor"),
    (&["ipynb"], "A Jupyter notebook: code, results and notes in one file.", "Jupyter, VS Code or Google Colab"),
    (&["gguf"], "A language-model weights file for llama.cpp and similar local AI tools.", "llama.cpp, LM Studio or Ollama"),
    (&["onnx"], "A machine-learning model in the ONNX format.", "ONNX Runtime"),
    (&["safetensors"], "Machine-learning model weights (safe tensors format).", "PyTorch / Hugging Face tools"),
    (&["pt", "pth", "ckpt"], "PyTorch model weights or a training checkpoint.", "PyTorch"),
    (&["zip"], "A ZIP archive: files packed and compressed into one.", "Windows itself, 7-Zip"),
    (&["7z"], "A 7-Zip archive: files packed and compressed into one.", "7-Zip"),
    (&["rar"], "A RAR archive: files packed and compressed into one.", "WinRAR or 7-Zip"),
    (&["tar"], "A tar archive: files packed into one (not compressed).", "7-Zip or tar"),
    (&["gz", "tgz"], "A gzip-compressed file, often a packed folder (.tar.gz).", "7-Zip or tar"),
    (&["xz", "txz"], "An xz-compressed file, often a packed folder (.tar.xz).", "7-Zip or tar"),
    (&["bz2"], "A bzip2-compressed file.", "7-Zip or tar"),
    (&["whl"], "A Python wheel: a ready-built Python package (a ZIP inside).", "pip install <file>"),
    (&["iso"], "A disc image: the full contents of a CD, DVD or USB installer.", "Windows (double-click mounts it)"),
    (&["exe"], "A Windows program. Running it starts the app (or its installer).", "Windows itself"),
    (&["msi"], "A Windows Installer package: installs a program.", "Windows Installer"),
    (&["msix", "appx"], "A Windows app package.", "App Installer"),
    (&["dll"], "A program library used by other programs; not opened on its own.", "(used by programs)"),
    (&["sys"], "A Windows driver or system file.", "(used by Windows)"),
    (&["lnk"], "A shortcut that points to another file, folder or program.", "Windows itself"),
    (&["url"], "An internet shortcut (a saved web link).", "your web browser"),
    (&["bat", "cmd"], "A Windows batch script: commands that run in a console.", "Command Prompt"),
    (&["ps1"], "A PowerShell script.", "PowerShell"),
    (&["sh"], "A shell script for Linux, macOS or Git Bash.", "bash"),
    (&["reg"], "Windows registry settings; opening it can change system settings.", "Registry Editor"),
    (&["png"], "A PNG image (lossless, may have transparency).", "Photos or any image viewer"),
    (&["jpg", "jpeg"], "A JPEG photo or image.", "Photos or any image viewer"),
    (&["gif"], "A GIF image, often a short animation.", "Photos or a web browser"),
    (&["webp"], "A WebP image, common on the web.", "Photos or a web browser"),
    (&["heic", "heif"], "A HEIC photo (common from iPhones).", "Photos (with the HEIF extension)"),
    (&["svg"], "A vector image (drawn from shapes, sharp at any size).", "a web browser or Inkscape"),
    (&["ico"], "A Windows icon image.", "an image viewer"),
    (&["bmp"], "A bitmap image (uncompressed).", "Photos or Paint"),
    (&["psd"], "A Photoshop image with layers.", "Adobe Photoshop or GIMP"),
    (&["mp4", "m4v"], "An MP4 video.", "Media Player, VLC"),
    (&["mkv"], "A Matroska video (often films with several audio or subtitle tracks).", "VLC, Media Player"),
    (&["mov"], "A QuickTime video.", "VLC, Media Player"),
    (&["avi", "wmv", "webm"], "A video file.", "VLC, Media Player"),
    (&["mp3"], "An MP3 audio file.", "Media Player, VLC"),
    (&["wav"], "An uncompressed audio recording.", "Media Player, VLC, Audacity"),
    (&["flac"], "Lossless compressed audio.", "VLC, Media Player"),
    (&["ogg", "opus"], "Compressed audio (Ogg / Opus).", "VLC, a web browser"),
    (&["m4a", "aac"], "Compressed audio (AAC).", "Media Player, VLC"),
    (&["srt", "vtt"], "Subtitles: timed text lines for a video.", "a video player (next to the video) or a text editor"),
    (&["apk"], "An Android app package.", "an Android phone (to install)"),
    (&["jar"], "A Java program or library.", "Java"),
    (&["db", "sqlite", "sqlite3"], "A database file.", "DB Browser for SQLite"),
    (&["toml", "yml", "yaml", "ini", "cfg", "conf"], "A settings file as plain text.", "a code editor"),
    (&["xml"], "XML data as text.", "a code editor"),
    (&["ttf", "otf", "woff", "woff2"], "A font.", "Windows Fonts (double-click to preview or install)"),
    (&["torrent"], "A torrent file: describes files to download with a torrent app.", "a torrent client"),
    (&["part", "crdownload", "tmp"], "An unfinished download or a temporary file.", "(usually safe to delete when the download is done)"),
    (&["bin"], "Raw binary data; what it holds depends on the program that made it.", "(depends on the program)"),
    (&["c", "h", "cpp", "hpp", "cc"], "C / C++ source code.", "a code editor"),
    (&["cs"], "C# source code.", "Visual Studio or a code editor"),
    (&["java", "kt"], "Java / Kotlin source code.", "an IDE or code editor"),
    (&["go"], "Go source code.", "a code editor"),
    (&["sql"], "SQL database commands.", "a database tool or code editor"),
    (&["stl", "obj", "fbx", "glb", "gltf", "blend"], "A 3D model.", "3D Viewer, Blender"),
];

/// (description, usual app) for a file name, or None when the type is unknown.
pub fn describe(name: &str, is_dir: bool) -> Option<(String, String)> {
    if is_dir {
        return Some(("A folder.".into(), "Rust File Explorer".into()));
    }
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tar.xz") || lower.ends_with(".tar.bz2") {
        return Some(("A packed folder (tar) that is also compressed. Extract it to use the files inside.".into(), "7-Zip or tar".into()));
    }
    let ext = lower.rsplit_once('.').map(|x| x.1.to_string())?;
    TABLE.iter().find(|r| r.0.contains(&ext.as_str())).map(|r| (r.1.to_string(), r.2.to_string()))
}

/// Number of entries in a ZIP (or .whl / .jar / .apk) from its end record: cheap, reads only the tail.
pub fn zip_entries(p: &Path) -> Option<u64> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(p).ok()?;
    let len = f.metadata().ok()?.len();
    let tail = len.min(66 * 1024);
    f.seek(SeekFrom::Start(len - tail)).ok()?;
    let mut b = vec![0u8; tail as usize];
    f.read_exact(&mut b).ok()?;
    let i = (0..b.len().saturating_sub(21)).rev().find(|&i| b[i..i + 4] == [0x50, 0x4B, 0x05, 0x06])?;
    let n = u16::from_le_bytes([b[i + 10], b[i + 11]]) as u64;
    if n == 0xFFFF {
        // ZIP64: the count is in the zip64 end record
        let j = (0..i.saturating_sub(55)).rev().find(|&j| b[j..j + 4] == [0x50, 0x4B, 0x06, 0x06])?;
        return Some(u64::from_le_bytes(b[j + 32..j + 40].try_into().ok()?));
    }
    Some(n)
}

/// The raw facts shown in the panel: label, value.
pub fn facts(path: &Path, is_dir: bool) -> Vec<(String, String)> {
    let mut v = vec![];
    let md = std::fs::metadata(path).ok();
    let ft = |t: Option<std::time::SystemTime>| t.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| fmcore::util::unix_ft(d.as_secs() as i64)).unwrap_or(0);
    if let Some(m) = &md {
        v.push(("Modified".into(), crate::fmt::date_long(ft(m.modified().ok()))));
        v.push(("Created".into(), crate::fmt::date_long(ft(m.created().ok()))));
        if !is_dir {
            v.push(("Size".into(), format!("{} ({} bytes)", crate::fmt::size(m.len()), crate::fmt::thousands(m.len()))));
        }
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = fmcore::util::ext_of(&name);
    if is_dir {
        if let Ok(rd) = std::fs::read_dir(path) {
            let (mut d, mut f) = (0, 0);
            for e in rd.flatten().take(100_000) {
                if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    d += 1
                } else {
                    f += 1
                }
            }
            v.push(("Inside".into(), format!("{d} folders, {f} files")));
        }
    } else if matches!(ext.as_str(), "zip" | "whl" | "jar" | "apk" | "nupkg" | "docx" | "xlsx" | "pptx")
        && let Some(n) = zip_entries(path)
    {
        v.push(("Inside".into(), format!("{} entries", crate::fmt::thousands(n))));
    }
    if matches!(ext.as_str(), "exe" | "dll" | "msi" | "sys")
        && let Some(vi) = fmcore::info::version_info(path)
    {
        if !vi.company.is_empty() {
            v.push(("Publisher".into(), vi.company.clone()));
        }
        if !vi.description.is_empty() {
            v.push(("Says it is".into(), vi.description.clone()));
        }
        if !vi.file_version.is_empty() {
            v.push(("Version".into(), vi.file_version.clone()));
        }
    }
    if ext == "lnk"
        && let Some(l) = fmcore::info::read_shortcut(path)
    {
        v.push(("Points to".into(), l.target));
    }
    if let Some((_, app)) = describe(&name, is_dir) {
        v.push(("Opens with".into(), app));
    }
    v.push(("Path".into(), path.display().to_string()));
    v
}
