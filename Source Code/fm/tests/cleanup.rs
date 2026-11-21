//! Cleanup recycling checks on files created in the test's own temp folder.
mod common;
use common::*;
use fmcore::cleanup;
use std::fs::{FileTimes, OpenOptions};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::time::{Duration, SystemTime};
use windows::Win32::Storage::FileSystem::{FILE_FLAG_BACKUP_SEMANTICS, FILE_WRITE_ATTRIBUTES};

#[test]
fn cleanup_recycles_a_fixture_and_refuses_an_open_file() {
    let dir = tdir("cleanup");
    let file = dir.join("move to bin.txt");
    let locked = dir.join("open file.txt");
    write(&file, b"test recycle");
    write(&locked, b"test lock");
    let handle = std::fs::File::open(&locked).unwrap();
    let results = cleanup::recycle(&[file.clone(), locked.clone()]);
    let record = fmcore::recycle::list_bin(&dir).into_iter().find(|r| r.original == file);
    let recycled = !file.exists() && record.is_some();
    let kept = locked.is_file();
    drop(handle);
    let removed = purge_bin_under(&dir);
    cleanup(&dir);
    assert!(recycled, "{results:?}");
    assert_eq!(removed, 1, "only the recycled fixture was removed from the bin");
    assert!(results[0].result.is_ok(), "{results:?}");
    assert!(kept && results[1].result.is_err(), "{results:?}");
}

#[test]
fn cleanup_move_reports_in_use_and_undo_restores_its_own_batch() {
    let dir = tdir("cleanup-undo");
    let file = dir.join("move.txt");
    let locked = dir.join("locked.txt");
    write(&file, b"move");
    write(&locked, b"locked");
    let entry = |path: &Path| {
        let md = path.metadata().unwrap();
        cleanup::Item { path: path.to_path_buf(), size: md.len(), modified: md.last_write_time(), reason: "Fixture" }
    };
    // Each integration-test binary has its own journal.
    unsafe { std::env::set_var("WFM_DATA_DIR", dir.join("data")) };
    let handle = std::fs::File::open(&locked).unwrap();
    let report = cleanup::move_items(vec![entry(&file), entry(&locked)]);
    let moved = !file.exists() && report.records.len() == 1 && report.moved.len() == 1;
    let mut later = fmcore::journal::Op::new("new-file");
    later.created.push(locked.clone());
    let later_id = fmcore::journal::record(later).unwrap();
    let skipped = locked.is_file() && report.skipped == [(locked, cleanup::Skip::InUse)];
    assert!(fmcore::journal::undo_cleanup(report.journal, &[]).is_err());
    let undo = fmcore::journal::undo_cleanup(report.journal, &report.records);
    let restored = std::fs::read(file).ok().as_deref() == Some(b"move");
    let later_kept = fmcore::journal::load().last().is_some_and(|op| op.id == later_id && op.created[0].exists());
    drop(handle);
    let left = purge_bin_under(&dir);
    cleanup(&dir);
    assert!(moved, "{:?}", report.warning);
    assert!(skipped, "{:?}", report.skipped);
    assert!(undo.is_ok() && restored && later_kept, "{undo:?}");
    assert_eq!(left, 0);
}

fn age(path: &Path, days: u64) {
    let old = SystemTime::now() - Duration::from_secs(days * 86_400);
    let file = OpenOptions::new().access_mode(FILE_WRITE_ATTRIBUTES.0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0).open(path).unwrap();
    file.set_times(FileTimes::new().set_modified(old)).unwrap();
}

#[test]
fn cleanup_rechecks_the_project_and_leaves_a_nonempty_folder_alone() {
    let dir = tdir("cleanup-project");
    let project = dir.join("project");
    let marker = project.join("Cargo.toml");
    let target = project.join("target");
    let artifact = target.join("build.bin");
    let ordinary = dir.join("ordinary");
    write(&marker, b"[package]");
    write(&artifact, b"build");
    write(&ordinary.join("keep.txt"), b"keep");
    // The old cache has a recently edited project file.
    age(&artifact, cleanup::PROJECT_IDLE_DAYS + 1);
    age(&target, cleanup::PROJECT_IDLE_DAYS + 1);
    age(&project, cleanup::PROJECT_IDLE_DAYS + 1);
    let refused = cleanup::recycle(&[target.clone(), ordinary.clone()]);
    let both_kept = artifact.is_file() && ordinary.join("keep.txt").is_file();
    age(&marker, cleanup::PROJECT_IDLE_DAYS + 1);
    let allowed = cleanup::recycle(std::slice::from_ref(&target));
    let recycled = !target.exists() && fmcore::recycle::list_bin(&dir).iter().any(|r| r.original == target);
    let removed = purge_bin_under(&dir);
    cleanup(&dir);
    assert!(both_kept && refused.iter().all(|r| r.result.is_err()), "{refused:?}");
    assert!(recycled && allowed[0].result.is_ok(), "{allowed:?}");
    assert_eq!(removed, 1);
}
