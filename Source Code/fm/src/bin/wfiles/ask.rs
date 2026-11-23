//! "AI Search" search (Enter in the search box with no result picked): Gemini turns plain words (any language) into
//! up to three Everything queries; they run together inside the same scope, rules and filters as a normal search,
//! and the merged rows come back ordered by how many queries found them, then newest first.
use crate::app::*;
use crate::model::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

impl App {
    pub fn ask_search(&mut self) {
        if !crate::ai::enabled() {
            self.toast("Turn on AI features in Settings to use AI Search", "", false);
            return;
        }
        let here = self.sources(self.pane().dir());
        let sp = self.sp.clone();
        let show_hidden = self.ui.hidden;
        let tab_id = self.tab().id;
        let p = self.tab().active;
        let Some(s) = self.pane_mut().search.as_mut() else { return };
        let words = s.field.text.trim().to_string();
        if words.is_empty() {
            return;
        }
        let seq = s.next_request();
        s.busy = true;
        s.note = "Asking AI\u{2026}".into();
        // the scope, rules and filters of this search, without its words
        let text = std::mem::take(&mut s.field.text);
        let base = s.query_with(&here, &sp);
        s.field.text = text;
        let whole = s.pc || here.is_empty();
        let junk_ok = s.junk;
        self.invalidate();
        self.spawn_search(tab_id, p, move || match run(&words, &base, &sp, whole, show_hidden, junk_ok) {
            Ok((items, note)) => Msg::Found { tab: tab_id, pane: p, seq, res: Ok((items.len() as u32, items.clone())), rows: items, ai: Some(note) },
            Err(e) => Msg::Found { tab: tab_id, pane: p, seq, res: Err(e), rows: vec![], ai: None },
        });
    }
}

fn run(words: &str, base: &str, sp: &SearchPrefs, whole: bool, show_hidden: bool, junk_ok: bool) -> Result<(Vec<Item>, String), String> {
    let (qs, note) = crate::ai::search_queries(words)?;
    let (v, shown) = collect(qs, note, base, sp, whole, show_hidden, junk_ok);
    Ok(pick(words, v, shown, None))
}

/// Step two: Gemini reads the candidates (name, folder, kind, date) and keeps the real matches, best first. On any
/// failure the candidates stay as they are.
pub(crate) fn pick(request: &str, v: Vec<Item>, shown: String, key: Option<&str>) -> (Vec<Item>, String) {
    if v.len() < 2 {
        return (v, shown);
    }
    let top: Vec<&Item> = v.iter().take(60).collect();
    let short = |p: &str| {
        let parts: Vec<&str> = p.split('\\').filter(|s| !s.is_empty()).collect();
        parts[parts.len().saturating_sub(3)..].join("\\")
    };
    let cands: Vec<(String, String, bool, String)> = top.iter().map(|i| (i.name.clone(), short(&i.folder), i.is_dir, crate::fmt::date_ymd(i.modified))).collect();
    match crate::ai::pick_results(request, &cands, key) {
        Ok((best, drop, note)) => {
            // the best first, then every other candidate the AI did not drop (in the found order)
            let mut seen: std::collections::HashSet<usize> = drop.into_iter().collect();
            let mut out: Vec<Item> = best.into_iter().filter(|i| seen.insert(*i)).map(|i| top[i].clone()).collect();
            out.extend((0..top.len()).filter(|i| !seen.contains(i)).map(|i| top[i].clone()));
            out.extend(v.iter().skip(top.len()).cloned());
            (out, if note.is_empty() { shown } else { note })
        }
        Err(_) => (v, shown),
    }
}

/// Run the AI's queries (the last entry is the core name words) and merge them. When the asked thing does not
/// exist (no installer on disk), the nearest real things come instead: the core name words alone, then the longest
/// of them alone.
pub(crate) fn collect(mut qs: Vec<String>, note: String, base: &str, sp: &SearchPrefs, whole: bool, show_hidden: bool, junk_ok: bool) -> (Vec<Item>, String) {
    let core = qs.pop().unwrap_or_default();
    let mut v = merge(&qs, base, sp, whole, show_hidden, junk_ok);
    // a program or game asked for: the biggest .exe files inside the folders found (the game itself, its launcher),
    // since an exe is often named by an abbreviation the words do not match
    if qs.iter().any(|q| q.contains("exe")) {
        // helpers that ship with games and programs are not "the" program
        let helper = |n: &str| {
            let n = n.to_lowercase();
            ["redist", "vcredist", "directx", "dxsetup", "unins", "crash", "report", "helper", "update", "dotnet", "ue4prereq", "prereq"].iter().any(|h| n.contains(h))
        };
        let dirs: Vec<PathBuf> = v.iter().filter(|i| i.is_dir).take(3).map(|i| i.path.clone()).collect();
        for d in dirs {
            // an exe the queries already found inside this folder is enough
            if v.iter().any(|x| !x.is_dir && x.path.starts_with(&d)) {
                continue;
            }
            let q = format!("{} ext:exe", fmcore::evipc::quote(&format!("{}\\", d.to_string_lossy().trim_end_matches('\\'))));
            let found: Vec<Item> = merge_sorted(&[q], base, sp, whole, show_hidden, junk_ok, fmcore::evipc::SORT_SIZE_DESC, 8)
                .into_iter()
                .filter(|e| !helper(&e.name) && !v.iter().any(|x| x.path == e.path))
                .take(2)
                .collect();
            // right after its folder
            let at = v.iter().position(|x| x.path == d).map(|p| p + 1).unwrap_or(v.len());
            for (k, e) in found.into_iter().enumerate() {
                v.insert((at + k).min(v.len()), e);
            }
        }
        // the program itself before its helpers when the queries found both
        v.sort_by_key(|i| !i.is_dir && helper(&i.name));
    }
    let mut shown = if note.is_empty() { qs.join("  \u{00b7}  ") } else { note };
    if v.is_empty() && !core.trim().is_empty() {
        let longest = core.split_whitespace().max_by_key(|w| w.chars().count()).unwrap_or("").to_string();
        for q in [core.clone(), longest] {
            if q.is_empty() {
                continue;
            }
            v = merge(std::slice::from_ref(&q), base, sp, whole, show_hidden, junk_ok);
            if !v.is_empty() {
                shown = format!("nothing exact, the closest: {q}");
                break;
            }
        }
    }
    (v, shown)
}

/// The queries together, merged: found by more queries first, then by the more precise query (earlier in the
/// list), then newest.
fn merge(qs: &[String], base: &str, sp: &SearchPrefs, whole: bool, show_hidden: bool, junk_ok: bool) -> Vec<Item> {
    merge_sorted(qs, base, sp, whole, show_hidden, junk_ok, fmcore::evipc::SORT_DATE_MODIFIED_DESC, 300)
}

#[allow(clippy::too_many_arguments)]
fn merge_sorted(qs: &[String], base: &str, sp: &SearchPrefs, whole: bool, show_hidden: bool, junk_ok: bool, sort: u32, max: u32) -> Vec<Item> {
    // dot names (.vscode) and what is inside them stay out, as in a normal search
    let hid = if show_hidden { "" } else { " !\\." };
    let req = fmcore::evipc::REQ_NAME | fmcore::evipc::REQ_PATH | fmcore::evipc::REQ_SIZE | fmcore::evipc::REQ_MODIFIED | fmcore::evipc::REQ_ATTRS;
    // Everything answers one query at a time: they queue, each is short
    let answers: Vec<Option<fmcore::evipc::Reply>> = std::thread::scope(|sc| {
        let hs: Vec<_> = qs
            .iter()
            .map(|q| {
                let full = format!("{base} {q}{hid}");
                sc.spawn(move || fmcore::evipc::query(&full, max, req, sort, false, Duration::from_secs(4)).ok())
            })
            .collect();
        hs.into_iter().map(|h| h.join().ok().flatten()).collect()
    });
    let mut merged: HashMap<String, (u32, usize, Item)> = HashMap::new();
    let mut cache = HIDDEN_DIRS.lock().unwrap();
    for (qi, r) in answers.into_iter().enumerate() {
        let Some(r) = r else { continue };
        for i in r.items {
            let full = i.full();
            if whole && !sp.keeps(Path::new(&full)) {
                continue;
            }
            if !show_hidden && (i.attrs & 0x2 != 0 || i.name.starts_with('.') || in_hidden_folder(Path::new(&i.path), &mut cache)) {
                continue;
            }
            if !junk_ok && crate::rank::junk(Path::new(&full)) {
                continue;
            }
            let e = merged.entry(full.to_lowercase()).or_insert_with(|| {
                let is_dir = i.is_dir;
                let item = Item {
                    kind: crate::ficons::kind_of(&i.name, is_dir),
                    hl: match_ranges(&i.name, &qs[qi]),
                    size: if is_dir { i.size.filter(|s| *s > 0) } else { i.size },
                    modified: i.modified,
                    attrs: i.attrs,
                    folder: i.path.clone(),
                    is_dir,
                    name: i.name.clone(),
                    path: PathBuf::from(&full),
                    recycled: None,
                    empty: false,
                };
                (0, qi, item)
            });
            e.0 += 1;
            e.1 = e.1.min(qi);
        }
    }
    let mut v: Vec<(u32, usize, Item)> = merged.into_values().collect();
    v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(b.2.modified.cmp(&a.2.modified)));
    v.into_iter().map(|x| x.2).collect()
}

#[cfg(test)]
mod live {
    /// cargo test ask_live -- --ignored --nocapture (needs GEMINI_API_KEY_1 and a route to Gemini, e.g. GEMINI_BASE_URL;
    /// the key is never printed): the whole AI Search path (AI queries, Everything, filters, fallback) for a few
    /// requests, as a This PC search with default settings.
    #[test]
    #[ignore]
    fn ask_live() {
        let key = std::env::var("GEMINI_API_KEY_1").expect("GEMINI_API_KEY_1 not set");
        let sp = crate::model::SearchPrefs::default();
        for req in [
            "the installer of Gears of War",
            "my rust file manager source code",
            "\u{0641}\u{06cc}\u{0644}\u{0645}\u{200c}\u{0647}\u{0627}\u{06cc} \u{062f}\u{0627}\u{0646}\u{0644}\u{0648}\u{062f} \u{0634}\u{062f}\u{0647} \u{0627}\u{06cc}\u{0646} \u{0645}\u{0627}\u{0647}",
            "\u{0646}\u{0635}\u{0628}\u{06cc} \u{0628}\u{0627}\u{0632}\u{06cc} \u{06af}\u{06cc}\u{0631}\u{0632} \u{0627}\u{0641} \u{0648}\u{0627}\u{0631}",
        ] {
            let t0 = std::time::Instant::now();
            match crate::ai::search_queries_with(req, Some(&key)) {
                Ok((qs, note)) => {
                    let ai_ms = t0.elapsed().as_millis();
                    let t1 = std::time::Instant::now();
                    let (items, shown) = super::collect(qs.clone(), note, "", &sp, true, false, false);
                    let (items, shown) = super::pick(req, items, shown, Some(&key));
                    println!("REQ {req}\n  ai {ai_ms} ms, search {} ms, queries {:?}\n  note: {shown}\n  {} rows: {:?}", t1.elapsed().as_millis(), qs, items.len(), items.iter().take(6).map(|i| i.path.display().to_string()).collect::<Vec<_>>());
                }
                Err(e) => println!("REQ {req}\n  AI error after {:?}: {e}", t0.elapsed()),
            }
        }
    }
}
