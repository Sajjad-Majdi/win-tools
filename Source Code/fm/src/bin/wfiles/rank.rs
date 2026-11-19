//! Search relevance: Everything finds the matches, this ranks them the way a person looks for things.
//! Name match (exact > starts with > word start > contains), then where it lives (the user's own places up;
//! temp, caches, AppData, node_modules, .git, Windows, Program Files, the Recycle Bin far down), then how recent it
//! is. Runs of the same name in look-alike folders (remotion-webpack-bundle-1234\public\lumen ...) fold into one
//! "N more" row, and junk places are hidden behind one hint row unless "Include system & temp" is on.
use crate::model::Item;
use std::collections::HashMap;
use std::path::PathBuf;

/// Path prefix of the rows that are not files: "N more in ..." and "N hidden in temp / system folders".
pub const MORE: &str = "::more::";
pub const HIDDEN: &str = "::hidden::";

/// System, temp and cache places (lower case, with backslashes around): hidden by default, ranked last.
const JUNK: &[&str] = &[
    "\\appdata\\local\\temp\\", "\\windows\\temp\\", "\\appdata\\", "\\node_modules\\", "\\.git\\", "\\$recycle.bin\\", "\\programdata\\",
    "\\program files\\", "\\program files (x86)\\", "\\.cache\\", "\\cache\\", "\\caches\\", "\\__pycache__\\", "\\.venv\\", "\\venv\\",
    "\\site-packages\\", "\\target\\debug\\", "\\target\\release\\", "\\.gradle\\", "\\.m2\\", "\\.cargo\\registry\\", "\\.npm\\",
    "\\system volume information\\", "\\winsxs\\", "\\.vs\\", "\\build\\tmp\\", "\\build\\intermediates\\", "\\build\\generated\\",
    "\\obj\\debug\\", "\\obj\\release\\", "\\.next\\", "\\.dart_tool\\",
];
/// Places that are noisy but real (logs, session dumps): ranked down, never hidden.
const NOISY: &[&str] = &["\\sessions\\", "\\logs\\", "\\log\\", "\\_work\\", "\\.claude\\", "\\backup\\", "\\backups\\", "\\build\\", "\\src\\", "\\java\\", "\\res\\"];

/// The nearest folder above the item whose name holds a typed word (the item sits inside a better match: a
/// project "Lumen note keeper" holds dozens of "lumen" package folders), as (its path, lower case).
fn match_ancestor(it: &Item, words: &[String]) -> Option<String> {
    let mut p = it.path.parent();
    while let Some(d) = p {
        let name = d.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        if !name.is_empty() && words.iter().all(|w| name.contains(w.as_str())) {
            return Some(d.to_string_lossy().to_lowercase());
        }
        p = d.parent();
    }
    None
}
/// Places where people keep their own work: ranked up.
const OWN: &[&str] = &["\\codebase\\", "\\projects\\", "\\source\\repos\\", "\\dev\\", "\\code\\", "\\work\\"];

fn low_path(p: &std::path::Path) -> String {
    let mut s = p.to_string_lossy().to_lowercase();
    if !s.ends_with('\\') {
        s.push('\\');
    }
    s
}

/// Is a path in a system / temp / cache place?
pub fn junk(p: &std::path::Path) -> bool {
    let s = low_path(p);
    if s.starts_with("c:\\windows\\") {
        return true;
    }
    JUNK.iter().any(|j| s.contains(j))
}

/// How well the name matches the typed words (0 when a word is not in the name: the match is on the path).
fn name_score(name: &str, words: &[String]) -> f32 {
    let n = name.to_lowercase();
    let stem = n.rsplit_once('.').map(|x| x.0.to_string()).unwrap_or_else(|| n.clone());
    let all = words.join(" ");
    if n == all || stem == all {
        return 100.0;
    }
    let mut total = 0.0;
    for w in words {
        let s = if n.starts_with(w.as_str()) {
            70.0
        } else if let Some(i) = n.find(w.as_str()) {
            let before = n[..i].chars().last().unwrap_or(' ');
            if !before.is_alphanumeric() { 50.0 } else { 25.0 }
        } else {
            0.0
        };
        total += s;
    }
    total / words.len().max(1) as f32
}

/// The user's own places (lower case, ending in a backslash): Desktop, Documents, Downloads, pins, recent folders.
pub fn score(it: &Item, words: &[String], own: &[String], now_ft: u64) -> f32 {
    let mut s = name_score(&it.name, words);
    if it.is_dir {
        s += 6.0;
    }
    let parent = low_path(it.path.parent().unwrap_or(&it.path));
    if junk(&it.path) {
        s -= 80.0;
    } else if NOISY.iter().any(|j| parent.contains(j)) {
        s -= 25.0;
    }
    if own.iter().any(|o| parent.starts_with(o.as_str())) || OWN.iter().any(|o| parent.contains(o)) {
        s += 20.0;
    }
    // inside a folder that already matches: that folder is the answer, its contents come later
    if match_ancestor(it, words).is_some() {
        s -= 45.0;
    }
    // shallow paths (a drive root, the user's folders) before deep ones
    let depth = parent.matches('\\').count() as f32;
    s += ((8.0 - depth) * 2.0).clamp(-20.0, 10.0);
    // recently changed
    if it.modified > 0 && now_ft > it.modified {
        let days = (now_ft - it.modified) as f64 / 864_000_000_000.0;
        s += if days < 1.0 { 15.0 } else if days < 7.0 { 10.0 } else if days < 30.0 { 5.0 } else { 0.0 };
    }
    s
}

/// A folder path with its run-specific parts made generic ("remotion-webpack-bundle-8f3a91c2" -> "remotion-webpack-bundle-*"),
/// so items in look-alike folders fold together.
pub fn pattern(parent: &str) -> String {
    parent
        .split('\\')
        .map(|c| {
            let digits = c.chars().filter(|x| x.is_ascii_digit()).count();
            let hexrun = c.split(|x: char| !x.is_ascii_hexdigit()).map(|r| r.len()).max().unwrap_or(0);
            if digits >= 4 || hexrun >= 8 {
                // keep the readable head, star the varying tail
                let head: String = c.chars().take_while(|x| !x.is_ascii_digit()).collect();
                let head = head.trim_end_matches(|x: char| x.is_ascii_hexdigit() && x.is_ascii_lowercase());
                format!("{head}*")
            } else {
                c.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\\")
}

fn short_place(pat: &str) -> String {
    // the last two folders are enough to recognise the place
    let parts: Vec<&str> = pat.split('\\').filter(|p| !p.is_empty()).collect();
    let n = parts.len();
    parts[n.saturating_sub(3)..].join("\\")
}

/// Rank `all`, fold runs of the same name in look-alike folders (unless `open` has their key), and hide junk places
/// (unless `with_junk`). Returns the rows to show; the hidden and folded rows are reachable through their marker rows.
pub fn arrange(all: &[Item], words_text: &str, own: &[String], with_junk: bool, open: &std::collections::HashSet<String>, now_ft: u64) -> Vec<Item> {
    let words: Vec<String> = words_text.split_whitespace().filter(|w| !w.contains(':')).map(|w| w.to_lowercase().trim_matches('*').to_string()).filter(|w| !w.is_empty()).collect();
    let mut scored: Vec<(f32, &Item)> = all.iter().map(|i| (score(i, &words, own, now_ft), i)).collect();
    // best first; the same score keeps Everything's order (newest first)
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut hidden = 0usize;
    let mut groups: HashMap<String, Vec<&Item>> = HashMap::new();
    let mut order: Vec<String> = vec![];
    for (_, it) in &scored {
        if !with_junk && junk(&it.path) {
            hidden += 1;
            continue;
        }
        let parent = it.path.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        // same name in look-alike folders, or anywhere inside the same matching folder, folds into one row
        let place = match_ancestor(it, &words).map(|a| format!("in:{a}")).unwrap_or_else(|| pattern(&parent).to_lowercase());
        let key = format!("{}|{}", it.name.to_lowercase(), place);
        if !groups.contains_key(&key) {
            order.push(key.clone());
        }
        groups.entry(key).or_default().push(*it);
    }
    let mut out = vec![];
    for key in order {
        let g = &groups[&key];
        out.push(g[0].clone());
        if g.len() > 2 && !open.contains(&key) {
            let parent = g[1].path.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            let place = match key.split_once("|in:") {
                Some((_, anc)) => {
                    // the matching folder's own name, as written on disk
                    let n = g[1].path.ancestors().find(|a| a.to_string_lossy().to_lowercase() == anc).and_then(|a| a.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    format!("inside {n}")
                }
                None => format!("in {}", short_place(&pattern(&parent))),
            };
            let mut m = marker(&format!("{MORE}{key}"), &format!("{}: {} more {place}", g[0].name, g.len() - 1));
            m.folder = "Show them".into();
            out.push(m);
        } else {
            out.extend(g[1..].iter().map(|i| (*i).clone()));
        }
    }
    if hidden > 0 {
        let mut m = marker(HIDDEN, &format!("{hidden} hidden in temp / system folders"));
        m.folder = "Show them for this search".into();
        out.push(m);
    }
    out
}

fn marker(path: &str, label: &str) -> Item {
    Item {
        name: label.into(),
        path: PathBuf::from(path),
        is_dir: false,
        size: None,
        modified: 0,
        attrs: 0,
        kind: crate::ficons::Kind::File,
        folder: String::new(),
        hl: vec![],
        recycled: None,
        empty: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn it(path: &str, dir: bool, modified: u64) -> Item {
        let p = PathBuf::from(path);
        Item { name: p.file_name().unwrap().to_string_lossy().into_owned(), path: p, is_dir: dir, size: None, modified, attrs: 0, kind: crate::ficons::Kind::Folder, folder: String::new(), hl: vec![], recycled: None, empty: false }
    }

    #[test]
    fn real_project_first_junk_hidden_runs_folded() {
        let mut all = vec![];
        for i in 0..40 {
            all.push(it(&format!(r"C:\Users\S\AppData\Local\Temp\remotion-webpack-bundle-{}a9f\public\lumen", 1000 + i), true, 0));
        }
        for i in 0..20 {
            all.push(it(&format!(r"E:\Claude Bot\_work\sessions\2026-10-{:02}_lumen-notes.md", i + 1), false, 0));
        }
        all.push(it(r"E:\CodeBase\My Apps\Lumen note keeper - Opus 5.5", true, 0));
        all.push(it(r"C:\Users\S\Desktop\Lumen Panel.url", false, 0));
        let own = vec![r"c:\users\s\desktop\".to_string()];
        let rows = arrange(&all, "lumen", &own, false, &Default::default(), 0);
        assert!(rows[0].name.starts_with("Lumen"), "first: {}", rows[0].name);
        assert!(rows[1].name.starts_with("Lumen"), "second: {}", rows[1].name);
        let last = rows.last().unwrap();
        assert_eq!(last.path, PathBuf::from(HIDDEN));
        assert!(last.name.starts_with("40 hidden"));
        // with junk on, the 40 bundle folders fold into one row
        let rows = arrange(&all, "lumen", &own, true, &Default::default(), 0);
        let more: Vec<&Item> = rows.iter().filter(|r| r.path.to_string_lossy().starts_with(MORE)).collect();
        assert_eq!(more.len(), 1, "{:?}", rows.iter().map(|r| r.name.clone()).collect::<Vec<_>>());
        assert!(more[0].name.contains("39 more in"), "{}", more[0].name);
    }

    #[test]
    fn pattern_stars_run_ids() {
        assert_eq!(pattern(r"C:\T\remotion-webpack-bundle-1234567\public"), r"C:\T\remotion-webpack-bundle-*\public");
        assert_eq!(pattern(r"E:\CodeBase\My Apps"), r"E:\CodeBase\My Apps");
    }
}
