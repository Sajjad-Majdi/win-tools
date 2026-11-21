//! Real shell icons for files (as Explorer shows them), made off the UI thread and cached for the life of the window.
//! Programs, shortcuts and icon files get their own icon (keyed by path); every other file shares its type's icon
//! (keyed by extension). Folders and drives keep the vector icons (they are instant). Until an icon is ready the
//! vector icon of the type is drawn, so nothing waits.
use std::collections::{HashMap, HashSet};

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

/// Premultiplied BGRA pixels.
pub type Img = (u32, u32, Vec<u8>);

pub type Queue = Arc<(Mutex<Vec<(String, u32, PathBuf)>>, Condvar)>;

#[derive(Default)]
pub struct Icons {
    pub q: Queue,
    pub done: HashMap<(String, u32), Option<Img>>,
    pub asked: HashSet<(String, u32)>,
}

/// Files whose icon is their own (not their type's).
fn own_icon(ext: &str) -> bool {
    matches!(ext, "exe" | "lnk" | "url" | "ico" | "cur" | "ani" | "msc" | "cpl" | "scr" | "appref-ms" | "website" | "theme" | "deskthemepack")
}

/// The cache key of an item's icon, or None when the vector icon is used (folders, drives).
pub fn key_of(path: &Path, name: &str, is_dir: bool, attrs: u32, modified: u64) -> Option<String> {
    if is_dir {
        // a folder can carry its own icon (desktop.ini IconResource) only with the read-only or system attribute:
        // those get the shell's icon, per folder and per change time; plain folders keep the shared vector icon
        return (attrs & 0x5 != 0).then(|| format!("d|{}|{modified}", path.display()));
    }
    let ext = fmcore::util::ext_of(name);
    Some(if own_icon(&ext) { format!("p|{}", path.display()) } else { format!("e|{ext}") })
}

/// Pixel sizes icons are made at (fewer sizes = more cache hits).
pub fn bucket(px: f32) -> u32 {
    for b in [16u32, 20, 24, 32, 40, 48, 64, 96, 128, 256] {
        if px <= b as f32 + 0.5 {
            return b;
        }
    }
    256
}

/// New menu icons use a separate type cache so every row has its Shell icon on first paint.
pub fn new_item_icon(name: &'static str, px: u32, folder: bool) -> Option<Arc<fmcore::menu::Icon>> {
    type Cache = HashMap<(&'static str, u32), Arc<fmcore::menu::Icon>>;
    static ICONS: OnceLock<Mutex<Cache>> = OnceLock::new();
    let mut icons = ICONS.get_or_init(Mutex::default).lock().unwrap();
    if let Some(icon) = icons.get(&(name, px)) {
        return Some(icon.clone());
    }
    let icon = Arc::new(fmcore::menu::shgetfileinfo_type_icon(Path::new(name), px, folder)?);
    icons.insert((name, px), icon.clone());
    Some(icon)
}

/// Start the icon worker (one thread, its own COM apartment). `done` posts each result to the window.
pub fn start(q: Queue, done: impl Fn(String, u32, Option<Img>) + Send + 'static) {
    std::thread::spawn(move || {
        let _c = fmcore::util::com();
        loop {
            let (key, px, path) = {
                let (m, cv) = &*q;
                let mut g = m.lock().unwrap();
                while g.is_empty() {
                    g = cv.wait(g).unwrap();
                }
                // newest first: the rows on screen now
                g.pop().unwrap()
            };
            done(key, px, load(&path, px));
        }
    });
}

/// The icon of a file at `px` pixels (shortcuts with the arrow, as Explorer draws them).
fn load(path: &Path, px: u32) -> Option<Img> {
    // list sizes (up to 32 px): SHGetFileInfo, the system image list (Explorer's own icons, shortcut arrow
    // included); it is lighter than the image factory (about 4 MB less in memory). Bigger: the image factory.
    let i = if px <= 32 {
        fmcore::menu::shgetfileinfo_icon(path, px).or_else(|| fmcore::menu::list_icon(path, px))?
    } else {
        fmcore::menu::list_icon(path, px)?
    };
    Some((i.width, i.height, i.bgra))
}
