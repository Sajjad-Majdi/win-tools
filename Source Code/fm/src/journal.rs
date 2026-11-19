//! Undo journal: a short list of our own file operations in %LOCALAPPDATA%\WinTools\fm-journal.json.
//! `undo_last` reverses the newest one: copies go to the Recycle Bin, moves and renames go back, recycled and
//! replaced items come back from the Recycle Bin.
use crate::recycle::Recycled;
use crate::util::*;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const KEEP: usize = 100;

pub fn journal_path() -> PathBuf {
    data_dir().join("fm-journal.json")
}

/// One recorded operation.
#[derive(Clone, Debug, Default)]
pub struct Op {
    pub id: u64,
    /// Unix seconds.
    pub time: i64,
    /// copy | move | rename | batch-rename | delete | mkdir | new-file
    pub kind: String,
    /// Paths this operation created (undo sends them to the Recycle Bin).
    pub created: Vec<PathBuf>,
    /// (from, to) pairs this operation moved or renamed (undo moves them back, newest first).
    pub moved: Vec<(PathBuf, PathBuf)>,
    /// Items this operation sent to the Recycle Bin, including replaced targets (undo restores them).
    pub recycled: Vec<Recycled>,
}

impl Op {
    pub fn new(kind: &str) -> Op {
        Op { kind: kind.into(), ..Default::default() }
    }
    pub fn is_empty(&self) -> bool {
        self.created.is_empty() && self.moved.is_empty() && self.recycled.is_empty()
    }
    /// One-line description ("move 3 items", ...).
    pub fn summary(&self) -> String {
        let n = self.created.len().max(self.moved.len()).max(self.recycled.len());
        let first = self
            .moved
            .first()
            .map(|m| m.0.clone())
            .or_else(|| self.created.first().cloned())
            .or_else(|| self.recycled.first().map(|r| r.original.clone()))
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        if n <= 1 { format!("{} {}", self.kind, first) } else { format!("{} {} items ({} ...)", self.kind, n, first) }
    }
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id, "time": self.time, "kind": self.kind,
            "created": self.created.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
            "moved": self.moved.iter().map(|(a, b)| json!({"from": a.to_string_lossy(), "to": b.to_string_lossy()})).collect::<Vec<_>>(),
            "recycled": self.recycled.iter().map(|r| json!({"original": r.original.to_string_lossy(), "data": r.data.to_string_lossy(), "record": r.record.to_string_lossy(), "deleted": r.deleted})).collect::<Vec<_>>(),
        })
    }
    pub fn from_json(v: &Value) -> Op {
        let s = |x: &Value| PathBuf::from(x.as_str().unwrap_or(""));
        Op {
            id: v["id"].as_u64().unwrap_or(0),
            time: v["time"].as_i64().unwrap_or(0),
            kind: v["kind"].as_str().unwrap_or("").into(),
            created: v["created"].as_array().map(|a| a.iter().map(s).collect()).unwrap_or_default(),
            moved: v["moved"].as_array().map(|a| a.iter().map(|m| (s(&m["from"]), s(&m["to"]))).collect()).unwrap_or_default(),
            recycled: v["recycled"]
                .as_array()
                .map(|a| a.iter().map(|r| Recycled { original: s(&r["original"]), data: s(&r["data"]), record: s(&r["record"]), deleted: r["deleted"].as_u64().unwrap_or(0) }).collect())
                .unwrap_or_default(),
        }
    }
}

/// All recorded operations, oldest first.
pub fn load() -> Vec<Op> {
    let Ok(s) = std::fs::read_to_string(journal_path()) else { return vec![] };
    let v: Value = serde_json::from_str(&s).unwrap_or(Value::Null);
    v["ops"].as_array().map(|a| a.iter().map(Op::from_json).collect()).unwrap_or_default()
}

fn save(ops: &[Op]) -> Res<()> {
    let p = journal_path();
    std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| format!("cannot create {}: {e}", data_dir().display()))?;
    let start = ops.len().saturating_sub(KEEP);
    let v = json!({"version": 1, "ops": ops[start..].iter().map(|o| o.to_json()).collect::<Vec<_>>()});
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string(&v).unwrap()).map_err(|e| format!("cannot write journal: {e}"))?;
    std::fs::rename(&tmp, &p).map_err(|e| format!("cannot write journal: {e}"))
}

/// Append an operation (ignored when it changed nothing). Returns its id.
pub fn record(mut op: Op) -> Res<u64> {
    if op.is_empty() {
        return Ok(0);
    }
    let mut ops = load();
    op.id = ops.last().map(|o| o.id + 1).unwrap_or(1);
    op.time = ft_unix(now_ft());
    let id = op.id;
    ops.push(op);
    save(&ops)?;
    Ok(id)
}

/// Reverse the newest operation. On a partial failure the parts that could not be undone stay in the journal.
pub fn undo_last() -> Res<Op> {
    let mut ops = load();
    let op = ops.pop().ok_or("nothing to undo")?;
    let mut left = Op { id: op.id, time: op.time, kind: op.kind.clone(), ..Default::default() };
    let mut errors: Vec<String> = vec![];

    // 1. Things we created: to the Recycle Bin (or deleted when the drive has none).
    let created: Vec<PathBuf> = op.created.iter().rev().filter(|p| p.exists()).cloned().collect();
    let (bin, nobin): (Vec<PathBuf>, Vec<PathBuf>) = created.into_iter().partition(|p| crate::recycle::has_recycle_bin(p));
    if !bin.is_empty() {
        if let Err(e) = crate::recycle::recycle(&bin) {
            errors.push(e);
            left.created.extend(bin);
        }
    }
    for p in nobin {
        if let Err(e) = crate::recycle::delete_permanently(&p) {
            errors.push(e);
            left.created.push(p);
        }
    }
    // 2. Moves and renames go back, newest first. A batch rename goes back as one batch (safe for swaps).
    if op.kind == "batch-rename" && !op.moved.is_empty() {
        let back: Vec<(PathBuf, PathBuf)> = op.moved.iter().map(|(a, b)| (b.clone(), a.clone())).collect();
        if let Err(e) = crate::ops::rename_pairs(&back) {
            errors.push(e);
            left.moved = op.moved.clone();
        }
    }
    for (from, to) in op.moved.iter().rev().filter(|_| op.kind != "batch-rename") {
        if let Err(e) = crate::ops::move_back(to, from) {
            errors.push(e);
            left.moved.insert(0, (from.clone(), to.clone()));
        }
    }
    // 3. Recycled or replaced items come back.
    for r in op.recycled.iter().rev() {
        // The recorded $R may be gone when the item was restored and recycled again later (e.g. `bin restore`
        // then undo); then the newest Recycle Bin record of the same original path is the one to bring back.
        let cur = if r.data.exists() {
            r.clone()
        } else {
            crate::recycle::list_bin(&r.original).into_iter().find(|x| path_eq(&x.original, &r.original)).unwrap_or_else(|| r.clone())
        };
        if let Err(e) = crate::recycle::restore(&cur) {
            errors.push(e);
            left.recycled.insert(0, r.clone());
        }
    }
    if !left.is_empty() {
        ops.push(left);
    }
    save(&ops)?;
    if errors.is_empty() { Ok(op) } else { Err(format!("undo was only partly done: {}", errors.join("; "))) }
}

/// Helper for callers: record when the op has content; errors writing the journal do not fail the file operation.
pub fn record_quiet(op: Op) -> u64 {
    record(op).unwrap_or(0)
}

/// Does a path exist (file or folder)?
pub fn exists(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok()
}
