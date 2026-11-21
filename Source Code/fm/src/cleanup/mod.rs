//! Cleanup candidates from Everything. Run these synchronous calls on a worker thread.
mod classify;
mod duplicates;
mod packages;
mod protect;

pub use classify::{Classification, REVIEW_BATCH, Verdict, classify, review};
mod move_items;
pub use move_items::{BinInfo, MoveReport, Skip, bin_info, bin_limits, move_items};

use crate::evipc;
use crate::util::*;
use protect::{Facts, Rules, key, known, under};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use windows::Win32::UI::Shell::*;
use windows::core::PCWSTR;

pub const INSTALLER_AGE_DAYS: u64 = 30;
pub const LARGE_FILE_AGE_DAYS: u64 = 180;
pub const LARGE_FILE_MIN_BYTES: u64 = 500 * 1024 * 1024;
pub const PROJECT_IDLE_DAYS: u64 = 90;
pub const DEVELOPER_CACHE_REASON: &str = "Old build output, and package downloads that tools fetch again";
const PACKAGE_DOWNLOAD_REASON: &str = "Downloaded package; fetched again when needed";
const TEMP_IDLE_DAYS: u64 = 7;

const DAY: u64 = 24 * 60 * 60 * 10_000_000;
const FILE_CANDIDATES: u32 = 8192;
const FOLDER_CANDIDATES: u32 = 4096;
const PROJECT_ENTRIES: usize = 20_000;
const QUERY_TIMEOUT: Duration = Duration::from_millis(1500);
const LARGE_GROUP_BUDGET: Duration = Duration::from_millis(220);
const SMALL_GROUP_BUDGET: Duration = Duration::from_millis(80);
const BUILD_GROUP_BUDGET: Duration = Duration::from_millis(350);
const CACHE_NAMES: &[&str] = &["cache", "caches", "code cache", "gpucache", "shadercache", "d3dscache", "cachestorage", "inetcache"];
const BUILD_NAMES: &[&str] = &["node_modules", "target", "__pycache__", ".gradle", "build"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupId {
    Temp,
    AppCaches,
    CrashDumps,
    Installers,
    LargeFiles,
    BuildCaches,
    DuplicateFiles,
    RecycleBin,
}

impl GroupId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Temp => "temp",
            Self::AppCaches => "app-caches",
            Self::CrashDumps => "crash-dumps",
            Self::Installers => "installers",
            Self::LargeFiles => "large-files",
            Self::BuildCaches => "build-caches",
            Self::DuplicateFiles => "duplicate-files",
            Self::RecycleBin => "recycle-bin",
        }
    }

    fn group(self, title: &'static str, reason: &'static str) -> Group {
        Group { id: self, title, reason, total_size: 0, items: vec![], limited: false }
    }
}

#[derive(Clone, Debug)]
pub struct Item {
    pub path: PathBuf,
    pub size: u64,
    /// Last modified, as a Windows FILETIME.
    pub modified: u64,
    pub reason: &'static str,
    pub copy_of: Option<PathBuf>,
}

impl Item {
    pub fn is_package_download(&self) -> bool {
        self.reason == PACKAGE_DOWNLOAD_REASON
    }
}

#[derive(Clone, Debug)]
pub struct Group {
    pub id: GroupId,
    pub title: &'static str,
    pub reason: &'static str,
    pub total_size: u64,
    pub items: Vec<Item>,
    /// A candidate, time or project limit was reached, or a bin size could not be read.
    pub limited: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Progress {
    pub group: GroupId,
    pub checked: usize,
    pub candidates: usize,
}

struct Scan {
    rules: Rules,
    temps: Vec<String>,
    app_data: Vec<String>,
    packages: Vec<String>,
    downloads: Option<String>,
    now: u64,
    parents: HashMap<PathBuf, bool>,
    seen: HashSet<String>,
    trees: HashSet<String>,
    duplicate_parents: HashSet<String>,
    projects: HashMap<PathBuf, bool>,
    ignore: HashSet<String>,
}

impl Scan {
    fn new() -> Res<Self> {
        let rules = Rules::load()?;
        let mut temps = vec![rules.win_temp.clone()];
        if let Ok(temp) = std::env::var("TEMP")
            && let Some(dir) = key(Path::new(temp.trim_end_matches(['\\', '/'])))
            && !rules.blocks(&dir)
        {
            temps.push(dir);
        }
        let folder = |id| known(id).and_then(|p| key(&p));
        let local = folder(&FOLDERID_LocalAppData);
        let roaming = folder(&FOLDERID_RoamingAppData);
        let packages = packages::roots(&rules.profile, local.as_deref(), roaming.as_deref());
        Ok(Self {
            rules, temps, app_data: local.into_iter().chain(folder(&FOLDERID_LocalAppDataLow)).chain(roaming).collect(), packages,
            downloads: folder(&FOLDERID_Downloads), now: now_ft(), parents: HashMap::new(),
            seen: HashSet::new(), trees: HashSet::new(), duplicate_parents: HashSet::new(), projects: HashMap::new(), ignore: HashSet::new(),
        })
    }

    fn query(&self, group: GroupId) -> Option<String> {
        use GroupId::*;
        let term = match group {
            Temp => format!("file: {} {}", within(&self.temps), date_query(self.now, TEMP_IDLE_DAYS)),
            AppCaches if !self.app_data.is_empty() => format!("file: {} {}", within(&self.app_data), in_named_dirs(CACHE_NAMES)),
            AppCaches => return None,
            CrashDumps => format!("file: <ext:dmp|{}>", in_named_dirs(&["crashdumps"])),
            Installers => format!("file: ext:exe;msi;zip {} {}", within(std::slice::from_ref(self.downloads.as_ref()?)), date_query(self.now, INSTALLER_AGE_DAYS)),
            LargeFiles => format!("file: size:>{LARGE_FILE_MIN_BYTES} {}", date_query(self.now, LARGE_FILE_AGE_DAYS)),
            BuildCaches => {
                let names = BUILD_NAMES.iter().map(|name| format!("wfn:{name}")).collect::<Vec<_>>().join("|");
                format!("folder: <{names}> {} !{}", date_query(self.now, PROJECT_IDLE_DAYS), in_named_dirs(BUILD_NAMES))
            }
            DuplicateFiles => format!("file: size:>={} sizedupe:", duplicates::MIN_BYTES),
            RecycleBin => return None,
        };
        let term = if self.packages.is_empty() {
            term
        } else {
            format!("{term} !{}", within(&self.packages))
        };
        self.scoped_query(term)
    }

    fn scoped_query(&self, term: String) -> Option<String> {
        if self.rules.fixed.is_empty() {
            return None;
        }
        let roots = self.rules.fixed.iter().map(|p| evipc::quote(p)).collect::<Vec<_>>().join("|");
        let mut blocked: Vec<_> = self.rules.blocked_roots.iter().map(|p| format!("{}\\", p)).collect();
        for drive in &self.rules.fixed {
            blocked.extend(["$recycle.bin", "system volume information", "recovery"].map(|p| format!("{drive}{p}\\")));
        }
        let exclusion = blocked.iter().map(|p| evipc::quote(p)).collect::<Vec<_>>().join("|");
        let win_temp = within(std::slice::from_ref(&self.rules.win_temp));
        Some(format!("{term} <{roots}> <!<{exclusion}>|{win_temp}>"))
    }

    fn collect(&mut self, group: &mut Group, cancel: &AtomicBool, progress: &dyn Fn(Progress)) -> Res<()> {
        self.collect_matches(group, false, cancel, progress)?;
        if group.id == GroupId::BuildCaches && !self.packages.is_empty() {
            self.collect_matches(group, true, cancel, progress)?;
        }
        Ok(())
    }

    fn collect_matches(&mut self, group: &mut Group, downloads: bool, cancel: &AtomicBool, progress: &dyn Fn(Progress)) -> Res<()> {
        let query = if downloads {
            self.scoped_query(format!("file: {}", within(&self.packages)))
        } else {
            self.query(group.id)
        };
        let Some(query) = query else {
            group.limited = true;
            return Ok(());
        };
        let folders = group.id == GroupId::BuildCaches && !downloads;
        let duplicates = group.id == GroupId::DuplicateFiles;
        let max = if folders { FOLDER_CANDIDATES } else { FILE_CANDIDATES };
        let sort = if folders { evipc::SORT_DATE_MODIFIED_DESC } else if duplicates { evipc::SORT_SIZE_ASC } else { evipc::SORT_SIZE_DESC };
        let fields = evipc::REQ_NAME | evipc::REQ_PATH | if duplicates { evipc::REQ_SIZE } else { 0 };
        progress(Progress { group: group.id, checked: 0, candidates: 0 });
        check_cancel(cancel)?;
        let reply = evipc::query(
            &query, max, fields, sort, false, QUERY_TIMEOUT,
        ).map_err(|e| format!("{}: {e}", group.title))?;
        let budget = match group.id {
            GroupId::BuildCaches => BUILD_GROUP_BUDGET,
            GroupId::Temp | GroupId::AppCaches | GroupId::DuplicateFiles => LARGE_GROUP_BUDGET,
            _ => SMALL_GROUP_BUDGET,
        };
        let deadline = Instant::now() + budget;
        let candidates = reply.items.len();
        group.limited |= reply.total as usize > candidates;
        if duplicates {
            return duplicates::collect(self, group, reply.items, cancel, deadline, progress);
        }
        let mut checked = 0;
        for hit in reply.items {
            check_cancel(cancel)?;
            if Instant::now() >= deadline {
                group.limited = true;
                break;
            }
            if checked % 128 == 0 {
                progress(Progress { group: group.id, checked, candidates });
            }
            checked += 1;
            let path = PathBuf::from(hit.full());
            let Some(name) = key(&path) else { continue };
            if self.ignore.contains(&name) || self.seen.contains(&name) || ancestors(&name).any(|p| self.trees.contains(p)) {
                continue;
            }
            let Some(facts) = self.rules.check(&path, &name, &mut self.parents) else { continue };
            let (size, reason) = match facts.is_dir() {
                true if folders => {
                    if self.duplicate_parents.contains(&name) || self.ignore.iter().any(|p| under(p, &name))
                        || packages::contains(&self.packages, &name) || self.packages.iter().any(|p| under(p, &name))
                    {
                        continue;
                    }
                    let cutoff = self.now.saturating_sub(PROJECT_IDLE_DAYS * DAY);
                    if !at_or_before(facts.modified, cutoff) {
                        continue;
                    }
                    let Some(root) = project_root(&path, cutoff) else { continue };
                    if !self.projects.contains_key(&root) {
                        let stale = walk(&root, cancel, Some(deadline), |_, md| old_metadata(md, cutoff));
                        group.limited |= matches!(stale, Walk::Limit);
                        self.projects.insert(root.clone(), matches!(stale, Walk::Done));
                    }
                    if !self.projects[&root] {
                        continue;
                    }
                    let (total, status) = tree_size(&self.rules, &path, &mut self.parents, cancel, Some(deadline));
                    group.limited |= matches!(status, Walk::Limit);
                    if !matches!(status, Walk::Done) || total == 0 && is_empty(&path) {
                        continue;
                    }
                    self.trees.insert(name.clone());
                    (total, "Generated folder in a project untouched for at least 90 days")
                }
                false if facts.links == 1 && downloads && packages::contains(&self.packages, &name) => {
                    (facts.size, PACKAGE_DOWNLOAD_REASON)
                }
                false if facts.links == 1 && !downloads && self.file_matches(group.id, &path, &name, &facts) => {
                    (facts.size, group.reason)
                }
                _ => continue,
            };
            self.seen.insert(name);
            group.total_size += size;
            group.items.push(Item { path, size, modified: facts.modified, reason, copy_of: None });
        }
        progress(Progress { group: group.id, checked, candidates });
        check_cancel(cancel)
    }

    fn file_matches(&self, group: GroupId, path: &Path, name: &str, facts: &Facts) -> bool {
        if packages::contains(&self.packages, name) {
            return false;
        }
        match group {
            GroupId::Temp => self.temps.iter().any(|p| under(name, p))
                && at_or_before(facts.modified, self.now.saturating_sub(TEMP_IDLE_DAYS * DAY)),
            GroupId::AppCaches => self.app_data.iter().any(|p| under(name, p) && in_named_dir(&name[p.len() + 1..], CACHE_NAMES)),
            GroupId::CrashDumps => ext(path) == "dmp" || in_named_dir(name, &["crashdumps"]),
            GroupId::Installers => self.downloads.as_ref().is_some_and(|p| under(name, p))
                && matches!(ext(path).as_str(), "exe" | "msi" | "zip")
                && facts.modified != 0 && facts.modified < self.now.saturating_sub(INSTALLER_AGE_DAYS * DAY),
            GroupId::LargeFiles => facts.size > LARGE_FILE_MIN_BYTES
                && at_or_before(facts.modified, self.now.saturating_sub(LARGE_FILE_AGE_DAYS * DAY)),
            _ => false,
        }
    }
}

/// Find candidates through Everything and verify each against the live filesystem. The bin row is size-only.
/// Cancellation is checked between queries and during validation. A cancelled scan returns an error.
pub fn scan(cancel: &AtomicBool, progress: &dyn Fn(Progress)) -> Res<Vec<Group>> {
    scan_in(None, cancel, progress, &mut |_| {})
}

/// Scope the index queries to the chosen drives. Publish each verified group as it finishes.
pub fn scan_in(scope: Option<&[String]>, cancel: &AtomicBool, progress: &dyn Fn(Progress), ready: &mut dyn FnMut(&Group)) -> Res<Vec<Group>> {
    use GroupId::*;
    check_cancel(cancel)?;
    let mut scan = Scan::new()?;
    if let Some(scope) = scope {
        scan.rules.fixed.retain(|root| scope.iter().any(|d| d.eq_ignore_ascii_case(root)));
    }
    scan.ignore = ignored().iter().filter_map(|p| key(p)).collect();
    let mut groups = vec![
        Temp.group("Temporary files", "Temp file unchanged for at least 7 days"),
        AppCaches.group("App caches", "File in a known app cache folder"),
        CrashDumps.group("Crash dumps", "Crash dump file or file in a CrashDumps folder"),
        Installers.group("Old installers", "EXE, MSI or ZIP in Downloads, modified more than 30 days ago"),
        LargeFiles.group("Large old files", "Larger than 500 MB and unchanged for at least 180 days"),
        BuildCaches.group("Developer caches", DEVELOPER_CACHE_REASON),
        DuplicateFiles.group("Duplicate files", "Same size and content as an older copy; at least 10 MB"),
        RecycleBin.group("Recycle Bin", "Files already in the Recycle Bin"),
    ];
    // Reserve both copies and keepers before other checks can select them.
    for i in [6, 5, 2, 0, 1, 3, 4] {
        check_cancel(cancel)?;
        scan.collect(&mut groups[i], cancel, progress)?;
        if i == 1 {
            for g in [2, 1, 5] {
                ready(&groups[g]);
            }
        } else if !matches!(i, 2 | 5) {
            ready(&groups[i]);
        }
    }
    let bin = groups.last_mut().unwrap();
    for (i, drive) in scan.rules.fixed.iter().enumerate() {
        check_cancel(cancel)?;
        progress(Progress { group: RecycleBin, checked: i, candidates: scan.rules.fixed.len() });
        let root = wide(drive);
        let mut info = SHQUERYRBINFO { cbSize: size_of::<SHQUERYRBINFO>() as u32, ..Default::default() };
        if unsafe { SHQueryRecycleBinW(PCWSTR(root.as_ptr()), &mut info) }.is_ok() {
            bin.total_size += info.i64Size.max(0) as u64;
        } else {
            bin.limited = true;
        }
    }
    let drives = scan.rules.fixed.len();
    progress(Progress { group: RecycleBin, checked: drives, candidates: drives });
    check_cancel(cancel)?;
    Ok(groups)
}

/// Paths hidden by the user. Preserve the original spelling for Settings.
pub fn ignored() -> Vec<PathBuf> {
    ignore_paths(&crate::pins::config())
}

fn ignore_paths(config: &serde_json::Value) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    config["cleanup"]["ignore"].as_array().into_iter().flatten()
        .filter_map(|v| v.as_str().map(PathBuf::from))
        .filter(|p| key(p).is_some_and(|k| seen.insert(k))).collect()
}

fn edit_ignore(config: &mut serde_json::Value, path: &Path, hide: bool) -> Res<()> {
    let name = key(path).ok_or("invalid Cleanup path")?;
    let mut paths = ignore_paths(config);
    paths.retain(|p| key(p).as_deref() != Some(&name));
    if hide {
        paths.push(path.to_path_buf());
    }
    if !config["cleanup"].is_object() {
        config["cleanup"] = serde_json::json!({});
    }
    config["cleanup"]["ignore"] = serde_json::json!(paths);
    Ok(())
}

pub fn set_ignored(path: &Path, hide: bool) -> Res<()> {
    let mut config = crate::pins::config();
    edit_ignore(&mut config, path, hide)?;
    crate::pins::save_config(&config)
}

fn check_cancel(cancel: &AtomicBool) -> Res<()> {
    if cancel.load(Ordering::Relaxed) { Err("cleanup scan cancelled".into()) } else { Ok(()) }
}

fn within(paths: &[String]) -> String {
    let terms = paths.iter().map(|p| evipc::quote(&format!("{p}\\"))).collect::<Vec<_>>().join("|");
    format!("<{terms}>")
}

fn in_named_dirs(names: &[&str]) -> String {
    let terms = names.iter().map(|name| evipc::quote(&format!("\\{name}\\"))).collect::<Vec<_>>().join("|");
    format!("<{terms}>")
}

fn in_named_dir(path: &str, names: &[&str]) -> bool {
    path.rsplit_once('\\').is_some_and(|(parent, _)| parent.split('\\').any(|p| names.contains(&p)))
}

fn ancestors(path: &str) -> impl Iterator<Item = &str> {
    path.match_indices('\\').map(|(i, _)| &path[..i])
}

fn date_query(now: u64, days: u64) -> String {
    // Include the entire cutoff day in the index query. Exact age is checked with live FILETIMEs.
    let date = fmt_time(now.saturating_sub((days - 1) * DAY));
    date.get(..10).map(|d| format!("dm:<{d}")).unwrap_or_default()
}

fn at_or_before(modified: u64, cutoff: u64) -> bool {
    modified != 0 && modified <= cutoff
}

fn ext(path: &Path) -> String {
    path.extension().and_then(|s| s.to_str()).unwrap_or_default().to_ascii_lowercase()
}

fn is_empty(path: &Path) -> bool {
    std::fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_none())
}

fn project_root(path: &Path, cutoff: u64) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    let markers: &[&str] = match name.as_str() {
        "node_modules" => &["package.json"],
        "target" => &["Cargo.toml", "pom.xml"],
        "__pycache__" => &["pyproject.toml", "setup.py", "requirements.txt", ".git"],
        ".gradle" => &["settings.gradle", "settings.gradle.kts", "build.gradle", "build.gradle.kts"],
        "build" => &["CMakeLists.txt", "meson.build", "build.gradle", "build.gradle.kts", "pyproject.toml", "setup.py", "Makefile"],
        _ => return None,
    };
    let mut parent = path.parent()?;
    loop {
        if markers.iter().any(|m| std::fs::symlink_metadata(parent.join(m)).is_ok()) {
            return std::fs::symlink_metadata(parent).ok().filter(|md| old_metadata(md, cutoff)).map(|_| parent.to_path_buf());
        }
        if name != "__pycache__" {
            return None;
        }
        parent = parent.parent()?;
    }
}

fn old_metadata(md: &std::fs::Metadata, cutoff: u64) -> bool {
    use std::os::windows::fs::MetadataExt;
    md.file_attributes() & (ATTR_REPARSE | ATTR_OFFLINE | ATTR_RECALL_ON_DATA_ACCESS) == 0
        && at_or_before(md.last_write_time(), cutoff)
}

enum Walk {
    Done,
    Refused,
    Limit,
}

fn walk(root: &Path, cancel: &AtomicBool, deadline: Option<Instant>, mut visit: impl FnMut(&Path, &std::fs::Metadata) -> bool) -> Walk {
    use std::os::windows::fs::MetadataExt;
    let mut left = PROJECT_ENTRIES;
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { return Walk::Refused };
        for entry in entries {
            if cancel.load(Ordering::Relaxed) {
                return Walk::Refused;
            }
            if left == 0 || deadline.is_some_and(|t| Instant::now() >= t) {
                return Walk::Limit;
            }
            left -= 1;
            let Ok(entry) = entry else { return Walk::Refused };
            let Ok(md) = entry.metadata() else { return Walk::Refused };
            let path = entry.path();
            if md.file_attributes() & (ATTR_REPARSE | ATTR_OFFLINE | ATTR_RECALL_ON_DATA_ACCESS) != 0 || !visit(&path, &md) {
                return Walk::Refused;
            }
            if md.is_dir() {
                pending.push(path);
            }
        }
    }
    Walk::Done
}

fn tree_size(rules: &Rules, root: &Path, parents: &mut HashMap<PathBuf, bool>, cancel: &AtomicBool, deadline: Option<Instant>) -> (u64, Walk) {
    let mut size = 0;
    let status = walk(root, cancel, deadline, |path, md| {
        let Some(name) = key(path) else { return false };
        let Some(facts) = rules.check(path, &name, parents) else { return false };
        if facts.is_dir() != md.is_dir() {
            return false;
        }
        if !facts.is_dir() && facts.links == 1 {
            size += facts.size;
        }
        true
    });
    (size, status)
}

#[derive(Debug)]
pub struct RecycleResult {
    pub path: PathBuf,
    pub result: Res<()>,
}

/// Recheck each item immediately before asking the Shell to send it to the Recycle Bin.
/// A folder must still meet the build-cache checks. One result is returned per path.
pub fn recycle(paths: &[PathBuf]) -> Vec<RecycleResult> {
    let rules = Rules::load();
    let _com = com();
    paths.iter().map(|path| {
        let result = match &rules {
            Ok(rules) if let Some(size) = can_recycle(rules, path) => {
                let item = Item { path: path.clone(), size, modified: 0, reason: "", copy_of: None };
                if bin_limits(&[item]).is_empty() {
                    recycle_one(path).map_err(|e| win_err("cannot move to the Recycle Bin", &e))
                } else {
                    Err("item cannot fit in the Recycle Bin or the drive has no bin".into())
                }
            }
            Ok(_) => Err("protected, in use, unavailable, or no longer a cleanup folder".into()),
            Err(e) => Err(e.clone()),
        };
        RecycleResult { path: path.clone(), result }
    }).collect()
}

fn can_recycle(rules: &Rules, path: &Path) -> Option<u64> {
    let mut parents = HashMap::new();
    let name = key(path)?;
    let facts = rules.check(path, &name, &mut parents)?;
    if !facts.is_dir() {
        return (facts.links == 1).then_some(facts.size);
    }
    if is_empty(path) { return None; }
    let cutoff = now_ft().saturating_sub(PROJECT_IDLE_DAYS * DAY);
    let project = project_root(path, cutoff)?;
    let cancel = AtomicBool::new(false);
    if !matches!(walk(&project, &cancel, None, |_, md| old_metadata(md, cutoff)), Walk::Done) {
        return None;
    }
    let (size, status) = tree_size(rules, path, &mut parents, &cancel, None);
    matches!(status, Walk::Done).then_some(size)
}

fn recycle_one(path: &Path) -> windows::core::Result<()> {
    let p = wide(path);
    unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(p.as_ptr()), None)?;
        let resolved = take_pwstr(item.GetDisplayName(SIGDN_FILESYSPATH)?);
        if key(Path::new(&resolved)) != key(path) {
            return Err(windows::core::Error::from_hresult(windows::Win32::Foundation::E_ACCESSDENIED));
        }
        let bin: IShellItem = SHGetKnownFolderItem(&FOLDERID_RecycleBinFolder, KF_FLAG_DEFAULT, None)?;
        let parent = item.GetParent()?;
        let transfer: ITransferSource = parent.BindToHandler(None, &BHID_Transfer)?;
        transfer.RecycleItem(&item, &bin, TSF_NORMAL.0 as u32).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan_rules() -> Scan {
        Scan {
            rules: Rules {
                fixed: vec!["c:\\".into(), "e:\\".into()],
                blocked_roots: vec![r"c:\windows".into(), r"c:\program files".into()],
                win_temp: r"c:\windows\temp".into(),
                ..Default::default()
            },
            temps: vec![r"c:\windows\temp".into(), r"c:\users\me\temp".into()],
            app_data: vec![r"c:\users\me\appdata\local".into()],
            packages: vec![r"c:\users\me\appdata\local\uv\cache".into()],
            downloads: Some(r"e:\my downloads".into()),
            now: unix_ft(1_800_000_000),
            parents: HashMap::new(),
            seen: HashSet::new(),
            trees: HashSet::new(),
            duplicate_parents: HashSet::new(),
            projects: HashMap::new(),
            ignore: HashSet::new(),
        }
    }

    #[test]
    fn file_group_rules_and_thresholds() {
        use GroupId::*;
        let scan = scan_rules();
        let mut facts = Facts { attrs: 0, size: LARGE_FILE_MIN_BYTES, modified: scan.now - INSTALLER_AGE_DAYS * DAY, created: 0, links: 1 };
        let installer = Path::new(r"E:\My Downloads\Sub\setup.MSI");
        let name = key(installer).unwrap();
        assert!(!scan.file_matches(Installers, installer, &name, &facts));
        facts.modified -= 1;
        assert!(scan.file_matches(Installers, installer, &name, &facts));
        assert!(!scan.file_matches(Installers, installer, r"e:\my downloads backup\setup.msi", &facts));
        facts.modified = scan.now - LARGE_FILE_AGE_DAYS * DAY - 1;
        assert!(!scan.file_matches(LargeFiles, installer, &name, &facts));
        facts.size += 1;
        assert!(scan.file_matches(LargeFiles, installer, &name, &facts));
        facts.modified += 1;
        assert!(scan.file_matches(LargeFiles, installer, &name, &facts));
        facts.modified += 1;
        assert!(!scan.file_matches(LargeFiles, installer, &name, &facts));
        for (group, path, matches) in [
            (Temp, r"c:\windows\temp\nested\anything.txt", true),
            (Temp, r"c:\users\me\temp2\anything.txt", false),
            (AppCaches, r"c:\users\me\appdata\local\browser\gpucache\a", true),
            (AppCaches, r"c:\users\me\appdata\local\uv\cache\archive-v0\a", false),
            (AppCaches, r"c:\users\me\appdata\local\browser\cached data\a", false),
            (CrashDumps, r"c:\work\file.dmp", true),
            (CrashDumps, r"c:\work\crashdumps\file", true),
        ] {
            assert_eq!(scan.file_matches(group, Path::new(path), path, &facts), matches, "{path}");
        }
    }

    #[test]
    fn hard_links_contribute_no_reclaimable_bytes() {
        let base = std::env::var_os("WFM_DATA_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let root = base.join(format!("cleanup-hard-links-{}", std::process::id()));
        let target = root.join("target");
        std::fs::create_dir_all(&target).unwrap();
        let shared = root.join("shared.dll");
        let linked = target.join("linked.dll");
        let single = target.join("single.dll");
        std::fs::write(&shared, b"shared bytes").unwrap();
        std::fs::hard_link(&shared, &linked).unwrap();
        std::fs::write(&single, b"single").unwrap();
        let rules = Rules::load().unwrap();
        let (size, status) = tree_size(&rules, &target, &mut HashMap::new(), &AtomicBool::new(false), None);
        let shared_size = can_recycle(&rules, &linked);
        let single_size = can_recycle(&rules, &single);
        std::fs::remove_dir_all(&root).unwrap();
        assert!(matches!(status, Walk::Done));
        assert_eq!(size, 6);
        assert_eq!(shared_size, None);
        assert_eq!(single_size, Some(6));
    }

    #[test]
    fn everything_queries_use_scopes_types_and_cutoffs() {
        let scan = scan_rules();
        let temp = scan.query(GroupId::Temp).unwrap();
        assert!(temp.starts_with(r#"file: <"c:\windows\temp\"|"c:\users\me\temp\"> dm:<"#), "{temp}");
        assert!(temp.contains(r#"<"c:\"|"e:\">"#), "{temp}");
        assert!(temp.contains(r#""c:\program files\""#), "{temp}");
        let installers = scan.query(GroupId::Installers).unwrap();
        assert!(installers.starts_with(r#"file: ext:exe;msi;zip <"e:\my downloads\"> dm:<"#), "{installers}");
        assert!(scan.query(GroupId::LargeFiles).unwrap().starts_with(&format!("file: size:>{LARGE_FILE_MIN_BYTES} dm:<")));
        assert!(scan.query(GroupId::BuildCaches).unwrap().starts_with("folder: <wfn:node_modules|wfn:target|wfn:__pycache__|wfn:.gradle|wfn:build> dm:<"));
        assert!(scan.query(GroupId::DuplicateFiles).unwrap().starts_with("file: size:>=10485760 sizedupe:"));
        assert!(scan.query(GroupId::RecycleBin).is_none());
    }

    #[test]
    fn cancelled_scan_never_queries_everything() {
        assert_eq!(scan(&AtomicBool::new(true), &|_| panic!("scan started")).unwrap_err(), "cleanup scan cancelled");
    }

    #[test]
    fn cleanup_ignore_keeps_one_path_and_can_be_undone() {
        let mut cfg = serde_json::json!({"ui": {"theme": 1}, "cleanup": {"ignore": [7, "C:\\Temp\\a", "c:/temp/A"], "custom": true}});
        assert_eq!(ignore_paths(&cfg), [PathBuf::from(r"C:\Temp\a")]);
        edit_ignore(&mut cfg, Path::new(r"C:\Temp\b"), true).unwrap();
        edit_ignore(&mut cfg, Path::new("c:/TEMP/A"), false).unwrap();
        assert_eq!(ignore_paths(&cfg), [PathBuf::from(r"C:\Temp\b")]);
        assert_eq!(cfg["ui"]["theme"], 1);
        assert_eq!(cfg["cleanup"]["custom"], true);
    }

    #[test]
    fn recycle_refuses_protected_paths() {
        let mut paths = vec![PathBuf::from(r"C:\Windows"), r"C:\Program Files".into(), r"C:\ProgramData".into(), r"\\server\share\file".into()];
        if let Some(profile) = known(&FOLDERID_Profile) {
            paths.push(profile);
        }
        let results = recycle(&paths);
        assert_eq!(results.len(), paths.len());
        assert!(results.iter().all(|r| r.result.is_err()));
    }
}
