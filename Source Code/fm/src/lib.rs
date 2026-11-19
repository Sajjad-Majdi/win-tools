//! fmcore: the engine of Rust File Explorer, a fast replacement for Windows File Explorer.
//!
//! The GUI and the `wfm` CLI are thin layers over this library. Every function is synchronous and returns
//! readable `String` errors; long operations take a progress callback and a `Control` (cancel / pause) so a GUI
//! can run them on a worker thread.
//!
//! | Need | API |
//! |---|---|
//! | List a folder, sort it | [`list::list_dir`], [`list::sort_entries`], [`list::SortKey`] |
//! | Details, version info, shortcut target, folder size | [`info::info`], [`info::folder_size`] |
//! | Instant search (Everything) | [`search::search`], [`search::Query`] |
//! | Copy / move with progress, cancel, pause, conflicts | [`ops::copy`], [`ops::move_items`], [`ops::Control`], [`ops::Conflict`] |
//! | Rename, batch rename with preview, new folder / file, delete | [`ops::rename`], [`ops::batch_plan`] + [`ops::batch_apply`], [`ops::mkdir`], [`ops::new_file`], [`ops::delete`] |
//! | Undo (Ctrl+Z) | [`journal::undo_last`], [`journal::load`] |
//! | Recycle Bin records and restore | [`recycle`] |
//! | Watch a folder for changes | [`watch::Watcher`], [`watch::watch_blocking`] |
//! | Thumbnails / icons (BGRA or PNG) | [`thumb::thumbnail`], [`thumb::thumbnail_png`] |
//! | Pins and recent files | [`pins`] |
//! | Open, open with, reveal, drives | [`shell`] |
//! | Zip / unzip (Windows tar.exe) | [`zip`] |
//! | Fill a Save/Open dialog for agents | [`dialog`] |
//! | Recycle Bin list / restore / empty | [`recycle::list_all`], [`recycle::find_item`], [`recycle::restore`], [`recycle::empty_all`] |
//! | Search without Everything (folder walk) | [`search::walk_search`], [`search::Engine`] |
//! | Network: shares of a server, mapped drives with a timeout | [`shell::server_shares`], [`shell::drives_within`] |
//! | Eject a USB stick / card / DVD | [`shell::eject`] |
//! | Phones and cameras (MTP) through the Shell namespace | [`namespace::portable_devices`], [`namespace::children`], [`namespace::copy`] |
//! | Right-click menu with app extensions | [`menu::list`], [`menu::invoke`] |
//! | `explorer /select,` requests | [`shell::select_target`] |
//! | Everything IPC (no es.exe): type-ahead search, all folder sizes of a folder | [`evipc::query`], [`evipc::folder_sizes`] |
//! | Control pipe of the window (`wfm gui ...`) | [`pipe::send`], [`pipe::serve`] |
//! | Default file manager (per user) | [`defaultfm::set`], [`defaultfm::restore`], [`defaultfm::is_default`] |
//! | Drag and drop / clipboard data | [`dnd::data_object`], [`dnd::paths_from_data_object`], [`dnd::hdrop_bytes`] |
//!
//! Settings and the undo journal live in `%LOCALAPPDATA%\WinTools\` (`fm.json`, `fm-journal.json`); set the
//! `WFM_DATA_DIR` environment variable to use another folder (the tests do).
pub mod defaultfm;
pub mod dialog;
pub mod dnd;
pub mod evipc;
pub mod info;
pub mod journal;
pub mod list;
pub mod menu;
pub mod namespace;
pub mod ops;
pub mod pins;
pub mod pipe;
pub mod recycle;
pub mod rx;
pub mod search;
pub mod shell;
pub mod thumb;
pub mod util;
pub mod watch;
pub mod zip;
