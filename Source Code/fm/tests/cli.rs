//! End-to-end tests of the wfm CLI (and through it, the fmcore library). Each test uses its own
//! %TEMP%\wfm-test-* folder and its own settings/journal folder.
mod common;
use common::*;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn names(v: &Value) -> Vec<String> {
    v["items"].as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap().to_string()).collect()
}

#[test]
fn ls_and_sort() {
    let d = tdir("ls");
    let data = d.join("_data");
    let w = d.join("w");
    write(&w.join("a10.txt"), &[0; 1000]);
    write(&w.join("a2.txt"), &[0; 100]);
    write(&w.join("a1.txt"), &[0; 10]);
    write(&w.join("b.log"), &[0; 500]);
    std::fs::create_dir(w.join("zdir")).unwrap();
    write(&w.join("hidden.txt"), b"h");
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("attrib").arg("+h").arg(w.join("hidden.txt")).creation_flags(CREATE_NO_WINDOW).status().unwrap();
    }

    let v = ok(&data, &["ls", &s(&w)]);
    assert_eq!(names(&v), ["a1.txt", "a2.txt", "a10.txt", "b.log", "zdir"], "natural name order, hidden left out");
    assert_eq!(v["files"], 4);
    assert_eq!(v["folders"], 1);
    let first = &v["items"][0];
    assert_eq!(first["size"], 10);
    assert_eq!(first["type"], "txt");
    assert!(first["modified"].as_i64().unwrap() > 1_600_000_000);

    let v = ok(&data, &["ls", &s(&w), "--all"]);
    assert!(names(&v).contains(&"hidden.txt".to_string()));
    let hid = v["items"].as_array().unwrap().iter().find(|i| i["name"] == "hidden.txt").unwrap().clone();
    assert!(hid["attributes"].as_str().unwrap().contains('H'));

    let v = ok(&data, &["ls", &s(&w), "--sort", "size", "--desc"]);
    assert_eq!(names(&v)[..4], ["a10.txt", "b.log", "a2.txt", "a1.txt"]);
    let v = ok(&data, &["ls", &s(&w), "--sort", "type", "--dirs-first"]);
    assert_eq!(names(&v)[0], "zdir");
    assert_eq!(names(&v)[1], "b.log");
    let v = ok(&data, &["ls", &s(&w), "--top", "2"]);
    assert_eq!(names(&v).len(), 2);
    assert_eq!(v["count"], 5);

    // Readable table and errors.
    let r = wfm(&data, &["ls", &s(&w)]);
    assert!(r.out.contains("a10.txt") && r.out.contains("<DIR>"));
    let r = wfm(&data, &["ls", &s(&w.join("nope"))]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("not found"));
    let r = wfm(&data, &["ls", &s(&w), "--sort", "bogus"]);
    assert_eq!(r.code, 2);
    let r = wfm(&data, &["frobnicate"]);
    assert_eq!(r.code, 2);
    cleanup(&d);
}

#[test]
fn ls_10k_files_is_fast() {
    let d = tdir("ls10k");
    let data = d.join("_data");
    let w = d.join("many");
    std::fs::create_dir_all(&w).unwrap();
    for i in 0..10_000 {
        std::fs::File::create(w.join(format!("file_{i:05}.dat"))).unwrap();
    }
    let t0 = Instant::now();
    let v = ok(&data, &["ls", &s(&w), "--sort", "name"]);
    let wall = t0.elapsed();
    assert_eq!(v["count"], 10_000);
    let ms = v["ms"].as_f64().unwrap();
    println!("ls of 10,000 files: {ms:.1} ms inside wfm, {} ms wall (process start + JSON output)", wall.as_millis());
    assert!(ms < 200.0, "listing + sorting 10k files took {ms} ms");
    cleanup(&d);
}

#[test]
fn copy_move_conflicts_progress_undo() {
    let d = tdir("copy");
    let data = d.join("_data");
    let src = d.join("src");
    let dst = d.join("dst");
    let big: Vec<u8> = (0..40 * 1024 * 1024u32).map(|i| (i % 251) as u8).collect();
    write(&src.join("big.bin"), &big);
    write(&src.join("tree").join("a.txt"), b"A");
    write(&src.join("tree").join("sub").join("b.txt"), b"BB");
    std::fs::create_dir_all(&dst).unwrap();

    // Copy with JSON progress lines.
    let r = wfm(&data, &["copy", &s(&src.join("big.bin")), &s(&src.join("tree")), &s(&dst), "--json"]);
    assert_eq!(r.code, 0, "{} {}", r.out, r.err);
    let lines = r.lines();
    let prog: Vec<&Value> = lines.iter().filter(|l| l["event"] == "progress").collect();
    assert!(prog.len() >= 2, "expected progress events, got {}", r.out);
    let last = prog.last().unwrap();
    assert_eq!(last["finished"], true);
    assert_eq!(last["bytes_done"], last["bytes_total"]);
    assert_eq!(last["bytes_total"], 40 * 1024 * 1024 + 3);
    assert_eq!(last["files_total"], 3);
    let done = lines.last().unwrap();
    assert_eq!(done["event"], "done");
    assert_eq!(done["ok"], true);
    assert_eq!(std::fs::read(dst.join("big.bin")).unwrap(), big);
    assert_eq!(std::fs::read(dst.join("tree").join("sub").join("b.txt")).unwrap(), b"BB");

    // Conflicts. Default: fail.
    let r = wfm(&data, &["copy", &s(&src.join("big.bin")), &s(&dst), "--json"]);
    assert_eq!(r.code, 1, "{}", r.out);
    // skip
    write(&src.join("note.txt"), b"new");
    write(&dst.join("note.txt"), b"old");
    let v = ok(&data, &["copy", &s(&src.join("note.txt")), &s(&dst), "--conflict", "skip"]);
    assert_eq!(v["skipped"].as_array().unwrap().len(), 1);
    assert_eq!(std::fs::read(dst.join("note.txt")).unwrap(), b"old");
    // keep-both
    let v = ok(&data, &["copy", &s(&src.join("note.txt")), &s(&dst), "--conflict", "keep-both"]);
    assert!(v["done"][0]["to"].as_str().unwrap().ends_with("note (2).txt"), "{v}");
    assert_eq!(std::fs::read(dst.join("note (2).txt")).unwrap(), b"new");
    // replace (the old one goes to the Recycle Bin; undo brings it back)
    ok(&data, &["copy", &s(&src.join("note.txt")), &s(&dst), "--conflict", "replace"]);
    assert_eq!(std::fs::read(dst.join("note.txt")).unwrap(), b"new");
    let u = ok(&data, &["undo"]);
    assert_eq!(u["undone"]["kind"], "copy");
    assert_eq!(std::fs::read(dst.join("note.txt")).unwrap(), b"old", "undo restored the replaced file");
    // folder merge with skip: existing files kept, missing ones added
    std::fs::remove_file(dst.join("tree").join("a.txt")).unwrap();
    write(&dst.join("tree").join("sub").join("b.txt"), b"keep");
    ok(&data, &["copy", &s(&src.join("tree")), &s(&dst), "--conflict", "skip"]);
    assert_eq!(std::fs::read(dst.join("tree").join("a.txt")).unwrap(), b"A");
    assert_eq!(std::fs::read(dst.join("tree").join("sub").join("b.txt")).unwrap(), b"keep");

    // Move inside one volume (a rename), then undo.
    let mv = d.join("moved");
    std::fs::create_dir_all(&mv).unwrap();
    ok(&data, &["move", &s(&src.join("tree")), &s(&mv)]);
    assert!(mv.join("tree").join("sub").join("b.txt").exists() && !src.join("tree").exists());
    let u = ok(&data, &["undo"]);
    assert_eq!(u["undone"]["kind"], "move");
    assert!(src.join("tree").join("sub").join("b.txt").exists() && !mv.join("tree").exists());

    // Move "across drives": the same temp folder through \\localhost\C$ is another device for Windows, so this
    // runs the copy-with-progress + remove-source path without leaving the test folder.
    let unc = to_unc(&mv);
    if let Some(unc) = unc.filter(|u| u.exists()) {
        let r = wfm(&data, &["move", &s(&src.join("big.bin")), &s(&src.join("tree")), &s(&unc), "--json"]);
        assert_eq!(r.code, 0, "{} {}", r.out, r.err);
        let lines = r.lines();
        assert!(lines.iter().filter(|l| l["event"] == "progress").count() >= 2);
        assert_eq!(lines.last().unwrap()["ok"], true, "{}", r.out);
        assert!(!src.join("big.bin").exists() && !src.join("tree").exists(), "sources removed after the cross-device move");
        assert_eq!(std::fs::read(mv.join("big.bin")).unwrap().len(), big.len());
        assert_eq!(std::fs::read(mv.join("tree").join("a.txt")).unwrap(), b"A");
        ok(&data, &["undo"]);
        assert!(src.join("big.bin").exists() && src.join("tree").join("sub").join("b.txt").exists() && !mv.join("big.bin").exists());
    } else {
        println!("NOTE: \\\\localhost\\C$ is not reachable; cross-device move not tested");
    }

    // Copy undo: the copy goes to the Recycle Bin.
    ok(&data, &["copy", &s(&src.join("note.txt")), &s(&d.join("copy-of-note.txt"))]);
    assert!(d.join("copy-of-note.txt").exists());
    ok(&data, &["undo"]);
    assert!(!d.join("copy-of-note.txt").exists());

    // Errors: missing source; folder into itself.
    assert_eq!(wfm(&data, &["copy", &s(&d.join("nope")), &s(&dst)]).code, 1);
    let r = wfm(&data, &["copy", &s(&src.join("tree")), &s(&src.join("tree").join("sub")), "--json"]);
    assert_eq!(r.code, 1, "{}", r.out);
    purge_bin_under(&d);
    cleanup(&d);
}

/// C:\x\y -> \\localhost\C$\x\y
fn to_unc(p: &Path) -> Option<PathBuf> {
    let st = p.to_string_lossy();
    let b = st.as_bytes();
    if b.len() > 2 && b[1] == b':' { Some(PathBuf::from(format!("\\\\localhost\\{}$\\{}", &st[..1], &st[3..]))) } else { None }
}

#[test]
fn rename_and_batch_rename() {
    let d = tdir("rename");
    let data = d.join("_data");
    let w = d.join("w");
    write(&w.join("report.txt"), b"r");
    let v = ok(&data, &["rename", &s(&w.join("report.txt")), "final report.txt"]);
    assert!(v["to"].as_str().unwrap().ends_with("final report.txt"));
    assert!(w.join("final report.txt").exists() && !w.join("report.txt").exists());
    // bad names and taken names
    assert_eq!(wfm(&data, &["rename", &s(&w.join("final report.txt")), "a:b.txt"]).code, 1);
    write(&w.join("other.txt"), b"o");
    assert_eq!(wfm(&data, &["rename", &s(&w.join("final report.txt")), "other.txt"]).code, 1);
    // case-only rename is allowed
    ok(&data, &["rename", &s(&w.join("other.txt")), "OTHER.txt"]);
    assert!(std::fs::read_dir(&w).unwrap().any(|e| e.unwrap().file_name() == "OTHER.txt"));
    // undo the last rename, then the first
    ok(&data, &["undo"]);
    assert!(std::fs::read_dir(&w).unwrap().any(|e| e.unwrap().file_name() == "other.txt"));
    ok(&data, &["undo"]);
    assert!(w.join("report.txt").exists());

    // Batch rename.
    let b = d.join("photos");
    for n in ["IMG_0001.JPG", "IMG_0002.JPG", "IMG_0010.JPG", "notes.txt"] {
        write(&b.join(n), n.as_bytes());
    }
    let v = ok(&data, &["batch-rename", &s(&b), "--pattern", r"^IMG_(\d+)\.jpg$", "--to", "Trip {n:2} ($1).jpg", "--dry-run"]);
    assert_eq!(v["dry_run"], true);
    let items: Vec<(String, String)> = v["items"].as_array().unwrap().iter().map(|i| (i["from"].as_str().unwrap().into(), i["to"].as_str().unwrap().into())).collect();
    assert_eq!(
        items,
        [
            ("IMG_0001.JPG".to_string(), "Trip 01 (0001).jpg".to_string()),
            ("IMG_0002.JPG".into(), "Trip 02 (0002).jpg".into()),
            ("IMG_0010.JPG".into(), "Trip 03 (0010).jpg".into())
        ]
    );
    assert_eq!(v["unchanged"], 1);
    assert!(b.join("IMG_0001.JPG").exists(), "dry run changes nothing");
    let r = wfm(&data, &["batch-rename", &s(&b), "--pattern", r"^IMG_(\d+)\.jpg$", "--to", "Trip {n:2} ($1).jpg", "--dry-run"]);
    assert!(r.out.contains("IMG_0001.JPG -> Trip 01 (0001).jpg"), "{}", r.out);

    ok(&data, &["batch-rename", &s(&b), "--pattern", r"^IMG_(\d+)\.jpg$", "--to", "Trip {n:2} ($1).jpg"]);
    assert!(b.join("Trip 03 (0010).jpg").exists() && !b.join("IMG_0010.JPG").exists());
    assert_eq!(std::fs::read(b.join("Trip 01 (0001).jpg")).unwrap(), b"IMG_0001.JPG");
    // --keep-ext: change only the name part
    ok(&data, &["batch-rename", &s(&b), "--pattern", "^notes$", "--to", "{parent}-notes", "--keep-ext"]);
    assert!(b.join("photos-notes.txt").exists());
    ok(&data, &["undo"]);
    assert!(b.join("notes.txt").exists());
    ok(&data, &["undo"]);
    assert!(b.join("IMG_0002.JPG").exists() && !b.join("Trip 02 (0002).jpg").exists());

    // Swap two names (needs temporary names), then undo the swap.
    let sw = d.join("swap");
    write(&sw.join("a.txt"), b"A");
    write(&sw.join("b.txt"), b"B");
    let r = wfm(&data, &["batch-rename", &s(&sw), "--pattern", "^(a|b)$", "--to", "x", "--keep-ext", "--dry-run", "--json"]);
    assert_eq!(r.json()["ok"], false, "the dry run shows the problem: {}", r.out);
    assert!(r.json()["problems"][0].as_str().unwrap().contains("two items"));
    let r = wfm(&data, &["batch-rename", &s(&sw), "--pattern", "^(a|b)$", "--to", "x", "--keep-ext", "--json"]);
    assert_eq!(r.code, 1, "two items to one name must be refused: {}", r.out);
    ok(&data, &["batch-rename", &s(&sw), "--pattern", "^a$", "--to", "c", "--keep-ext"]);
    ok(&data, &["batch-rename", &s(&sw), "--pattern", "^b$", "--to", "a", "--keep-ext"]);
    ok(&data, &["batch-rename", &s(&sw), "--pattern", "^c$", "--to", "b", "--keep-ext"]);
    assert_eq!(std::fs::read(sw.join("a.txt")).unwrap(), b"B");
    assert_eq!(std::fs::read(sw.join("b.txt")).unwrap(), b"A");
    // a real one-step swap through the library
    let plan = fmcore::ops::Plan { items: vec![(sw.join("a.txt"), sw.join("b.txt")), (sw.join("b.txt"), sw.join("a.txt"))], unchanged: 0, problems: vec![] };
    fmcore::ops::rename_pairs(&plan.items).unwrap();
    assert_eq!(std::fs::read(sw.join("a.txt")).unwrap(), b"A");
    assert_eq!(std::fs::read(sw.join("b.txt")).unwrap(), b"B");
    // bad pattern = usage-free error
    let r = wfm(&data, &["batch-rename", &s(&sw), "--pattern", "(a", "--to", "x"]);
    assert_eq!(r.code, 1);
    cleanup(&d);
}

#[test]
fn delete_to_recycle_bin_and_undo() {
    let d = tdir("delete");
    let data = d.join("_data");
    let f = d.join("w").join("to delete.txt");
    write(&f, b"precious");
    write(&d.join("w").join("folder").join("inside.txt"), b"in");
    let v = ok(&data, &["delete", &s(&f), &s(&d.join("w").join("folder"))]);
    assert_eq!(v["permanent"], false);
    assert!(!f.exists() && !d.join("w").join("folder").exists());
    // It is in the Recycle Bin, with a record we can read.
    let bin = fmcore::recycle::list_bin(&d);
    assert!(bin.iter().any(|r| r.original == f), "the file is in the Recycle Bin");
    let j = ok(&data, &["undo", "--list"]);
    assert_eq!(j["ops"][0]["kind"], "delete");
    ok(&data, &["undo"]);
    assert_eq!(std::fs::read(&f).unwrap(), b"precious");
    assert_eq!(std::fs::read(d.join("w").join("folder").join("inside.txt")).unwrap(), b"in");
    assert!(!fmcore::recycle::list_bin(&d).iter().any(|r| r.original == f), "restored, so no longer in the bin");
    // Permanent delete (no journal entry).
    ok(&data, &["delete", &s(&f), "--permanent"]);
    assert!(!f.exists());
    assert!(!fmcore::recycle::list_bin(&d).iter().any(|r| r.original == f));
    assert_eq!(wfm(&data, &["undo"]).code, 1, "nothing left to undo");
    assert_eq!(wfm(&data, &["delete", &s(&f)]).code, 1, "missing file");
    purge_bin_under(&d);
    cleanup(&d);
}

#[test]
fn mkdir_new_file_and_undo() {
    let d = tdir("mkdir");
    let data = d.join("_data");
    let deep = d.join("a").join("b").join("c");
    ok(&data, &["mkdir", &s(&deep)]);
    assert!(deep.is_dir());
    assert_eq!(wfm(&data, &["mkdir", &s(&deep)]).code, 1, "already exists");
    let nf = deep.join("new.txt");
    ok(&data, &["new-file", &s(&nf)]);
    assert_eq!(std::fs::metadata(&nf).unwrap().len(), 0);
    assert_eq!(wfm(&data, &["new-file", &s(&nf)]).code, 1, "already exists");
    assert_eq!(wfm(&data, &["new-file", &s(&deep.join("bad|name.txt"))]).code, 1);
    ok(&data, &["undo"]);
    assert!(!nf.exists());
    ok(&data, &["undo"]);
    assert!(!d.join("a").exists(), "undo removes the top-most new folder");
    purge_bin_under(&d);
    cleanup(&d);
}

#[test]
fn zip_and_unzip() {
    let d = tdir("zip");
    let data = d.join("_data");
    let w = d.join("w");
    write(&w.join("docs").join("readme.txt"), b"hello zip");
    write(&w.join("docs").join("گزارش.txt"), "سلام".as_bytes());
    write(&w.join("single.bin"), &[7u8; 5000]);
    let z = d.join("out.zip");
    let v = ok(&data, &["zip", &s(&w.join("docs")), &s(&w.join("single.bin")), &s(&z)]);
    assert!(v["size"].as_u64().unwrap() > 100);
    assert_eq!(&std::fs::read(&z).unwrap()[..2], b"PK", "a real zip file");
    assert_eq!(wfm(&data, &["zip", &s(&w.join("docs")), &s(&z)]).code, 1, "refuses to overwrite");
    let x = d.join("x");
    ok(&data, &["unzip", &s(&z), &s(&x)]);
    assert_eq!(std::fs::read(x.join("docs").join("readme.txt")).unwrap(), b"hello zip");
    assert_eq!(std::fs::read(x.join("single.bin")).unwrap().len(), 5000);
    assert_eq!(std::fs::read_to_string(x.join("docs").join("گزارش.txt")).unwrap(), "سلام", "non-English names survive");
    ok(&data, &["undo"]);
    assert!(!x.exists(), "undo of unzip removes what it made");
    purge_bin_under(&d);
    cleanup(&d);
}

#[test]
fn pins() {
    let d = tdir("pins");
    let data = d.join("_data");
    let a = d.join("Projects");
    std::fs::create_dir_all(&a).unwrap();
    assert_eq!(ok(&data, &["pins"])["pins"].as_array().unwrap().len(), 0);
    let v = ok(&data, &["pin", &s(&a), "--name", "Work"]);
    assert_eq!(v["pins"][0]["name"], "Work");
    ok(&data, &["pin", &s(&d)]);
    ok(&data, &["pin", &s(&a)]); // no duplicate
    let v = ok(&data, &["pins"]);
    assert_eq!(v["pins"].as_array().unwrap().len(), 2);
    assert!(data.join("fm.json").exists(), "pins live in fm.json in the data folder");
    let v = ok(&data, &["unpin", &s(&a)]);
    assert_eq!(v["pins"].as_array().unwrap().len(), 1);
    assert_eq!(wfm(&data, &["unpin", &s(&a)]).code, 1);
    assert_eq!(wfm(&data, &["pin", &s(&d.join("missing"))]).code, 1);
    cleanup(&d);
}

#[test]
fn info_details() {
    let d = tdir("info");
    let data = d.join("_data");
    let w = d.join("w");
    write(&w.join("a.txt"), &[1; 1234]);
    write(&w.join("sub").join("b.bin"), &[2; 4321]);
    std::fs::create_dir_all(w.join("empty")).unwrap();
    let v = ok(&data, &["info", &s(&w.join("a.txt"))]);
    assert_eq!(v["size"], 1234);
    assert_eq!(v["dir"], false);
    assert!(!v["type"].as_str().unwrap().is_empty());
    // folder size by walking (exact)
    let v = ok(&data, &["info", &s(&w), "--no-index"]);
    assert_eq!(v["folder"]["bytes"], 1234 + 4321);
    assert_eq!(v["folder"]["files"], 2);
    assert_eq!(v["folder"]["folders"], 2);
    assert_eq!(v["folder"]["source"], "walk");
    assert_eq!(v["folder"]["complete"], true);
    // exe version info (read only)
    let v = ok(&data, &["info", r"C:\Windows\System32\notepad.exe"]);
    assert!(!v["version"]["file_version"].as_str().unwrap().is_empty());
    assert!(v["version"]["company"].as_str().unwrap().contains("Microsoft"));
    // shortcut target
    let lnk = w.join("to a.lnk");
    {
        let _c = fmcore::util::com();
        fmcore::info::create_shortcut(&lnk, &w.join("a.txt"), "").unwrap();
    }
    let v = ok(&data, &["info", &s(&lnk)]);
    assert_eq!(v["shortcut"]["target"].as_str().unwrap().to_lowercase(), s(&w.join("a.txt")).to_lowercase());
    // recent files come from .lnk files like this one
    let recent = fmcore::pins::recent_in(&w, 10).unwrap();
    assert_eq!(recent.len(), 1);
    assert!(recent[0].exists && !recent[0].is_dir);
    // big folder from the Everything index (or a walk with a time limit when Everything is not running)
    let v = ok(&data, &["info", r"C:\Windows\System32", "--limit", "500"]);
    assert!(v["folder"]["bytes"].as_u64().unwrap() > 0);
    println!("C:\\Windows\\System32 size from {}: {} bytes in {} ms", v["folder"]["source"], v["folder"]["bytes"], v["folder"]["ms"]);
    assert_eq!(wfm(&data, &["info", &s(&w.join("missing"))]).code, 1);
    cleanup(&d);
}

#[test]
fn find_with_everything() {
    let d = tdir("find");
    let data = d.join("_data");
    if !fmcore::search::everything_ready() {
        println!("SKIPPED find: Everything is not running (es.exe cannot reach it)");
        cleanup(&d);
        return;
    }
    let unique = format!("wfmfind{}x{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis());
    write(&d.join(format!("{unique}.txt")), &[0; 2048]);
    write(&d.join(format!("{unique}.log")), b"x");
    std::fs::create_dir_all(d.join(format!("{unique}_dir"))).unwrap();
    // Everything sees new files within a moment.
    let mut v = Value::Null;
    let seen = wait_for(Duration::from_secs(10), || {
        v = ok(&data, &["find", &unique, "--in", &s(&d)]);
        v["count"].as_u64().unwrap() >= 3
    });
    assert!(seen, "Everything did not index the new files: {v}");
    // Best of 3 (other tests create 10,000 files at the same time, which keeps Everything busy for a moment).
    let mut wall = u128::MAX;
    let mut v = Value::Null;
    for _ in 0..3 {
        let t0 = Instant::now();
        v = ok(&data, &["find", &unique, "--ext", "txt"]);
        wall = wall.min(t0.elapsed().as_millis());
    }
    assert_eq!(v["count"], 1);
    assert_eq!(v["results"][0]["size"], 2048);
    assert!(v["results"][0]["modified"].as_i64().unwrap() > 1_600_000_000);
    println!("find (one word, whole PC): {} ms inside wfm, {wall} ms wall", v["ms"]);
    assert!(wall < 1000, "find took {wall} ms");
    assert_eq!(ok(&data, &["find", &unique, "--folders"])["count"], 1);
    assert_eq!(ok(&data, &["find", &unique, "--files", "--size", ">1kb"])["count"], 1);
    assert_eq!(ok(&data, &["find", &unique, "--files", "--modified", "1d"])["count"], 2);
    assert_eq!(ok(&data, &["find", &unique, "--top", "1"])["count"], 1);
    let r = wfm(&data, &["find", &unique]);
    assert!(r.out.contains(&unique) && r.out.contains("results"));
    assert_eq!(wfm(&data, &["find"]).code, 2);
    cleanup(&d);
}

#[test]
fn drives_and_open_with() {
    let d = tdir("drives");
    let data = d.join("_data");
    let v = ok(&data, &["drives"]);
    let c = v["drives"].as_array().unwrap().iter().find(|x| x["root"] == "C:\\").unwrap().clone();
    assert_eq!(c["type"], "fixed");
    assert!(c["total"].as_u64().unwrap() > c["free"].as_u64().unwrap());
    write(&d.join("x.txt"), b"x");
    let v = ok(&data, &["open-with", &s(&d.join("x.txt"))]);
    assert!(!v["apps"].as_array().unwrap().is_empty(), "some app opens .txt");
    cleanup(&d);
}

#[test]
fn thumbnails() {
    let d = tdir("thumb");
    let data = d.join("_data");
    // A 64x48 24-bit BMP with a gradient.
    let (w, h) = (64u32, 48u32);
    let row = (w * 3).div_ceil(4) * 4;
    let mut bmp = vec![];
    bmp.extend(b"BM");
    bmp.extend((54 + row * h).to_le_bytes());
    bmp.extend([0u8; 4]);
    bmp.extend(54u32.to_le_bytes());
    bmp.extend(40u32.to_le_bytes());
    bmp.extend((w as i32).to_le_bytes());
    bmp.extend((h as i32).to_le_bytes());
    bmp.extend(1u16.to_le_bytes());
    bmp.extend(24u16.to_le_bytes());
    bmp.extend([0u8; 24]);
    for y in 0..h {
        for x in 0..w {
            bmp.extend([(x * 4) as u8, (y * 5) as u8, 200]);
        }
        bmp.extend(vec![0u8; (row - w * 3) as usize]);
    }
    let img = d.join("pic.bmp");
    write(&img, &bmp);
    let out = d.join("pic-thumb.png");
    let v = ok(&data, &["thumb", &s(&img), "--out", &s(&out), "--size", "256"]);
    let png = std::fs::read(&out).unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert!(v["width"].as_u64().unwrap() > 0 && v["width"].as_u64().unwrap() <= 256);
    // An icon for a plain text file.
    write(&d.join("t.txt"), b"x");
    let v = ok(&data, &["thumb", &s(&d.join("t.txt")), "--out", &s(&d.join("t.png")), "--size", "48", "--icon"]);
    assert!(v["width"].as_u64().unwrap() <= 48);
    // Library: pixels straight for the GUI, with real (not all-zero) alpha.
    let im = fmcore::thumb::thumbnail(&img, 96, fmcore::thumb::ThumbKind::Auto).unwrap();
    assert_eq!(im.bgra.len() as u32, im.width * im.height * 4);
    assert!(im.bgra.chunks(4).any(|p| p[3] == 255));
    cleanup(&d);
}

#[test]
fn watch_streams_changes() {
    use std::io::{BufRead, BufReader};
    use std::os::windows::process::CommandExt;
    let d = tdir("watch");
    let data = d.join("_data");
    let w = d.join("w");
    std::fs::create_dir_all(&w).unwrap();
    let mut child = std::process::Command::new(wfm_exe())
        .args(["watch", &s(&w), "--timeout", "15", "--json"])
        .env("WFM_DATA_DIR", &data)
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut rd = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    rd.read_line(&mut line).unwrap();
    assert!(line.contains("watching"), "{line}");
    std::thread::sleep(Duration::from_millis(200));
    write(&w.join("new.txt"), b"1");
    std::fs::rename(w.join("new.txt"), w.join("renamed.txt")).unwrap();
    std::fs::remove_file(w.join("renamed.txt")).unwrap();
    let mut events = vec![];
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(10) {
        line.clear();
        if rd.read_line(&mut line).unwrap() == 0 {
            break;
        }
        let v: Value = serde_json::from_str(line.trim()).unwrap();
        events.push(v.clone());
        if v["event"] == "removed" {
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    let kinds: Vec<&str> = events.iter().map(|e| e["event"].as_str().unwrap()).collect();
    assert!(kinds.contains(&"added"), "{kinds:?}");
    let ren = events.iter().find(|e| e["event"] == "renamed").expect("renamed event");
    assert!(ren["path"].as_str().unwrap().ends_with("renamed.txt") && ren["old_path"].as_str().unwrap().ends_with("new.txt"));
    assert!(kinds.contains(&"removed"));
    // --count ends it by itself
    let mut child = std::process::Command::new(wfm_exe())
        .args(["watch", &s(&w), "--count", "1", "--timeout", "10", "--json"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(400));
    write(&w.join("one.txt"), b"1");
    let st = child.wait().unwrap();
    assert!(st.success());
    cleanup(&d);
}

#[test]
fn open_runs_default_app_on_hidden_desktop() {
    let d = tdir("open");
    let exe = savedlg_exe();
    assert!(exe.exists(), "examples are built by cargo test");
    let marker = d.join("opened.txt");
    let _desk = hidden::desktop();
    // wfm itself runs on the hidden desktop, so even a stray window could not reach the screen.
    let p = hidden::spawn(&wfm_exe(), &format!("open \"{}\"", exe.display()), &[("WFM_TEST_MARKER", &s(&marker)), ("WFM_DATA_DIR", &s(&d.join("_data")))]);
    assert_eq!(p.wait(15_000), Some(0), "wfm open exit code");
    assert!(wait_for(Duration::from_secs(10), || marker.exists()), "the opened program ran");
    // A missing file is an error, not a dialog.
    let p = hidden::spawn(&wfm_exe(), &format!("open \"{}\"", d.join("missing.xyz").display()), &[]);
    assert_eq!(p.wait(15_000), Some(1));
    cleanup(&d);
}

#[test]
fn dialog_save_on_hidden_desktop() {
    let d = tdir("dialog");
    let data = d.join("_data");
    let exe = savedlg_exe();
    let _desk = hidden::desktop();

    // 1. A Save dialog in our test app; wfm fills the path and presses Save.
    let result = d.join("result1.txt");
    let app = hidden::spawn(&exe, &format!("\"{}\" \"{}\"", result.display(), d.display()), &[]);
    let mut list = Value::Null;
    let found = wait_for(Duration::from_secs(20), || {
        list = ok(&data, &["dialog", "list", "--desktop", hidden::DESKTOP]);
        list["dialogs"].as_array().unwrap().iter().any(|x| x["pid"] == app.pid)
    });
    assert!(found, "dialog not found: {list}");
    let dlg = list["dialogs"].as_array().unwrap().iter().find(|x| x["pid"] == app.pid).unwrap().clone();
    assert_eq!(dlg["kind"], "save");
    assert_eq!(dlg["process"], "savedlg.exe");
    let target = d.join("saved by agent.txt");
    let v = ok(&data, &["dialog", "save", "--path", &s(&target), "--hwnd", &dlg["hwnd"].to_string(), "--desktop", hidden::DESKTOP]);
    assert_eq!(v["closed"], true);
    assert_eq!(app.wait(10_000), Some(0));
    assert_eq!(std::fs::read_to_string(&result).unwrap(), format!("OK {}", target.display()));

    // 2. The file exists: without --overwrite wfm reports the app's question and leaves the dialog open.
    write(&target, b"old");
    let result2 = d.join("result2.txt");
    let app2 = hidden::spawn(&exe, &format!("\"{}\" \"{}\"", result2.display(), d.display()), &[]);
    let mut h2 = String::new();
    assert!(wait_for(Duration::from_secs(20), || {
        let l = ok(&data, &["dialog", "list", "--desktop", hidden::DESKTOP]);
        match l["dialogs"].as_array().unwrap().iter().find(|x| x["pid"] == app2.pid) {
            Some(x) => {
                h2 = x["hwnd"].to_string();
                true
            }
            None => false,
        }
    }));
    let r = wfm(&data, &["dialog", "save", "--path", &s(&target), "--hwnd", &h2, "--desktop", hidden::DESKTOP, "--json"]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.out.contains("asked"), "the confirm question is reported: {}", r.out);
    assert_eq!(app2.wait(500), None, "still open");
    app2.kill();

    // 3. Same with --overwrite: wfm answers Yes.
    let result3 = d.join("result3.txt");
    let app3 = hidden::spawn(&exe, &format!("\"{}\" \"{}\"", result3.display(), d.display()), &[]);
    let mut h3 = String::new();
    assert!(wait_for(Duration::from_secs(20), || {
        let l = ok(&data, &["dialog", "list", "--desktop", hidden::DESKTOP]);
        match l["dialogs"].as_array().unwrap().iter().find(|x| x["pid"] == app3.pid) {
            Some(x) => {
                h3 = x["hwnd"].to_string();
                true
            }
            None => false,
        }
    }));
    let v = ok(&data, &["dialog", "save", "--path", &s(&target), "--hwnd", &h3, "--overwrite", "--desktop", hidden::DESKTOP]);
    assert_eq!(v["closed"], true, "{v}");
    assert_eq!(v["confirmed"], true);
    assert_eq!(app3.wait(10_000), Some(0));
    assert_eq!(std::fs::read_to_string(&result3).unwrap(), format!("OK {}", target.display()));

    // Errors: relative path is a usage error; no dialog left is an error.
    assert_eq!(wfm(&data, &["dialog", "save", "--path", "relative.txt", "--desktop", hidden::DESKTOP]).code, 2);
    cleanup(&d);
}
