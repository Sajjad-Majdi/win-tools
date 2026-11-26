//! Tests for the "default file manager" additions: Recycle Bin, search without Everything, network paths,
//! eject refusal, phones (shell namespace), the right-click menu, `open --select` and drag-and-drop data.
mod common;
use common::*;
use serde_json::Value;
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[test]
fn recycle_bin_list_and_restore() {
    let d = tdir("bin");
    let data = d.join("_data");
    let f = d.join("w").join("bin me.txt");
    let g = d.join("w").join("bin me too.txt");
    write(&f, b"one");
    write(&g, b"two");
    ok(&data, &["delete", &s(&f), &s(&g)]);
    let v = ok(&data, &["bin"]);
    let items = v["items"].as_array().unwrap();
    let it = items.iter().find(|i| i["original"] == s(&f)).expect("our file is listed").clone();
    assert_eq!(it["size"], 3);
    assert!(it["deleted"].as_i64().unwrap() > 1_600_000_000);
    assert!(it["id"].as_str().unwrap().starts_with("C:$I"));
    // restore by id
    let r = ok(&data, &["bin", "restore", it["id"].as_str().unwrap()]);
    assert_eq!(r["restored"], s(&f));
    assert_eq!(std::fs::read(&f).unwrap(), b"one");
    // restore by original path
    ok(&data, &["bin", "restore", &s(&g)]);
    assert_eq!(std::fs::read(&g).unwrap(), b"two");
    assert_eq!(wfm(&data, &["bin", "restore", &s(&g)]).code, 1, "no longer in the bin");
    // Undo the two restores (both go back to the bin), then undo the delete: it must find the new bin records.
    ok(&data, &["undo"]);
    ok(&data, &["undo"]);
    assert!(!f.exists() && !g.exists());
    ok(&data, &["undo"]);
    assert_eq!(std::fs::read(&f).unwrap(), b"one");
    assert_eq!(std::fs::read(&g).unwrap(), b"two");
    // emptying needs --yes (never run for real in tests)
    let r = wfm(&data, &["bin", "empty"]);
    assert_eq!(r.code, 2);
    assert!(r.err.contains("--yes"));
    purge_bin_under(&d);
    cleanup(&d);
}

#[test]
fn search_without_everything() {
    let d = tdir("walk");
    let data = d.join("_data");
    write(&d.join("docs").join("Invoice 2026.pdf"), &[0; 3000]);
    write(&d.join("docs").join("invoice-old.txt"), b"x");
    write(&d.join("docs").join("deep").join("more").join("invoice-deep.pdf"), b"y");
    std::fs::create_dir_all(d.join("invoices")).unwrap();
    let dir = s(&d);
    let run = |args: &[&str]| -> Value {
        let mut a = vec!["find"];
        a.extend_from_slice(args);
        a.extend(["--in", dir.as_str(), "--json"]);
        use std::os::windows::process::CommandExt;
        let o = std::process::Command::new(wfm_exe())
            .args(&a)
            .creation_flags(CREATE_NO_WINDOW)
            .env("WFM_DATA_DIR", &data)
            .env("WFM_NO_EVERYTHING", "1")
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
        serde_json::from_slice(&o.stdout).unwrap()
    };
    let v = run(&["invoice"]);
    assert_eq!(v["engine"], "walk", "the CLI says which engine answered");
    assert!(v["note"].as_str().unwrap().contains("not running"));
    assert_eq!(v["count"], 4);
    assert_eq!(v["complete"], true);
    assert_eq!(run(&["invoice", "--ext", "pdf"])["count"], 2);
    assert_eq!(run(&["invoice", "--files", "--size", ">1kb"])["count"], 1);
    assert_eq!(run(&["invoice", "--folders"])["count"], 1);
    assert_eq!(run(&["inv*.pdf"])["count"], 2);
    assert_eq!(run(&["invoice", "!pdf"])["count"], 2);
    assert_eq!(run(&["invoice", "--modified", "1d", "--files"])["count"], 3);
    assert_eq!(run(&["invoice", "--top", "1"])["complete"], false);
    // Forced walk with Everything running reports the walk too; table output names the engine.
    let r = wfm(&data, &["find", "invoice", "--in", &s(&d), "--engine", "walk"]);
    assert!(r.out.contains("engine: folder walk"), "{}", r.out);
    if fmcore::search::everything_ready() {
        let r = wfm(&data, &["find", "invoice", "--in", &s(&d), "--engine", "everything"]);
        assert_eq!(r.code, 0);
        assert!(r.out.contains("engine: Everything"), "{}", r.out);
    }
    cleanup(&d);
}

#[test]
fn network_paths_and_drives() {
    let d = tdir("unc");
    let data = d.join("_data");
    write(&d.join("over the network.txt"), b"net");
    let st = s(&d);
    let unc = format!("\\\\localhost\\{}$\\{}", &st[..1], &st[3..]);
    if std::path::Path::new(&unc).exists() {
        let v = ok(&data, &["ls", &unc]);
        assert!(v["items"].as_array().unwrap().iter().any(|i| i["name"] == "over the network.txt"));
        let v = ok(&data, &["ls", r"\\localhost", "--all"]);
        assert!(v["shares"].as_array().unwrap().iter().any(|x| x["name"] == "C$"), "{v}");
    } else {
        println!("NOTE: \\\\localhost\\C$ not reachable; UNC listing not tested");
    }
    let t0 = Instant::now();
    let v = ok(&data, &["drives", "--timeout", "1500"]);
    assert!(t0.elapsed() < Duration::from_secs(5));
    for x in v["drives"].as_array().unwrap() {
        assert!(x.get("remote").is_some() && x.get("timed_out").is_some());
    }
    // A 0 ms limit must still return at once (every drive marked as not answering in time).
    let t0 = Instant::now();
    let v = ok(&data, &["drives", "--timeout", "0"]);
    assert!(t0.elapsed() < Duration::from_secs(3));
    assert!(!v["drives"].as_array().unwrap().is_empty());
    cleanup(&d);
}

#[test]
fn eject_refuses_fixed_drive() {
    let d = tdir("eject");
    let data = d.join("_data");
    let r = wfm(&data, &["eject", "C:", "--json"]);
    assert_eq!(r.code, 1);
    assert!(r.json()["error"].as_str().unwrap().contains("not removable"), "{}", r.out);
    assert_eq!(wfm(&data, &["eject", "C:\\"]).code, 1);
    assert_eq!(wfm(&data, &["eject", "nonsense"]).code, 1);
    cleanup(&d);
}

#[test]
fn phones_and_shell_namespace() {
    let d = tdir("mtp");
    let data = d.join("_data");
    let v = ok(&data, &["devices"]);
    println!("portable devices found: {}", v["count"]);
    assert!(v["items"].is_array());
    let pc = fmcore::namespace::this_pc().unwrap();
    assert!(pc.iter().any(|e| e.parsing.eq_ignore_ascii_case("C:\\")), "This PC lists drive C:");
    // Browse and copy through the shell (the same calls a phone uses), on temp folders.
    write(&d.join("src").join("photo.jpg"), &[1; 777]);
    std::fs::create_dir_all(d.join("dst")).unwrap();
    let v = ok(&data, &["shell-ls", &s(&d.join("src"))]);
    assert_eq!(v["items"][0]["name"].as_str().unwrap().trim_end_matches(".jpg"), "photo");
    let parsing = v["items"][0]["parsing"].as_str().unwrap().to_string();
    ok(&data, &["shell-copy", &parsing, &s(&d.join("dst"))]);
    assert_eq!(std::fs::read(d.join("dst").join("photo.jpg")).unwrap().len(), 777);
    cleanup(&d);
}

#[test]
fn context_menu_list_and_invoke() {
    let d = tdir("menu");
    let data = d.join("_data");
    write(&d.join("victim.txt"), b"menu");
    write(&d.join("other.txt"), b"o");
    let v = ok(&data, &["menu", &s(&d.join("victim.txt"))]);
    let verbs: Vec<String> = v["items"].as_array().unwrap().iter().map(|i| i["verb"].as_str().unwrap().to_lowercase()).collect();
    assert!(verbs.contains(&"delete".to_string()) && verbs.contains(&"properties".to_string()), "{verbs:?}");
    // Two items of one folder share one menu.
    ok(&data, &["menu", &s(&d.join("victim.txt")), &s(&d.join("other.txt"))]);
    assert_eq!(wfm(&data, &["menu", &s(&d.join("victim.txt")), r"C:\Windows\notepad.exe"]).code, 1, "different folders");
    // Invoke the harmless "delete" verb (to the Recycle Bin, no window) with wfm itself on the hidden desktop, so
    // even an unexpected confirmation could never reach the screen.
    let _desk = hidden::desktop();
    let p = hidden::spawn(&wfm_exe(), &format!("menu \"{}\" --invoke delete", d.join("victim.txt").display()), &[("WFM_DATA_DIR", &s(&data))]);
    match p.wait(20_000) {
        Some(0) => {
            assert!(wait_for(Duration::from_secs(5), || !d.join("victim.txt").exists()), "the delete verb ran");
            assert!(fmcore::recycle::list_bin(&d).iter().any(|r| r.original == d.join("victim.txt")));
        }
        other => {
            p.kill();
            panic!("menu --invoke delete did not finish cleanly: {other:?}");
        }
    }
    assert_eq!(wfm(&data, &["menu", &s(&d.join("other.txt")), "--invoke", "no-such-verb"]).code, 1);
    purge_bin_under(&d);
    cleanup(&d);
}

#[test]
fn open_select_target() {
    let d = tdir("select");
    let data = d.join("_data");
    write(&d.join("pick me.txt"), b"x");
    let v = ok(&data, &["open", "--select", &s(&d.join("pick me.txt"))]);
    assert_eq!(v["folder"], s(&d));
    assert_eq!(v["select"], "pick me.txt");
    // explorer-style ",path" works too
    let v = ok(&data, &["open", "--select", &format!(",{}", s(&d.join("pick me.txt")))]);
    assert_eq!(v["select"], "pick me.txt");
    assert_eq!(wfm(&data, &["open", "--select", &s(&d.join("missing.txt"))]).code, 1);
    cleanup(&d);
}

#[test]
fn drag_drop_data_round_trip() {
    let d = tdir("dnd");
    let a = d.join("a.txt");
    let b = d.join("گزارش.txt");
    write(&a, b"a");
    write(&b, b"b");
    let paths: Vec<PathBuf> = vec![a.clone(), b.clone()];
    // In memory: CF_HDROP bytes.
    assert_eq!(fmcore::dnd::paths_from_hdrop(&fmcore::dnd::hdrop_bytes(&paths)), paths);
    // The shell's own data object (what other apps receive when files are dragged out), read back.
    let _c = fmcore::util::com();
    let obj = fmcore::dnd::data_object(&paths).unwrap();
    let back = fmcore::dnd::paths_from_data_object(&obj).unwrap();
    assert_eq!(back, paths);
    cleanup(&d);
}

/// `wfm default set|status|restore` on the test root: the explore verb, the class Folder (with Explorer's
/// openinexplorer hand-back) and the right-click menu parts are written, reported and removed again.
#[test]
fn default_set_on_test_root_has_explore_folder_and_menu() {
    let d = tdir("defaultroot");
    let data = d.join("_data");
    let root = format!(r"HKCU\Software\RustFileExplorerTest\Cli-{}\Classes", std::process::id());
    let exe = wfm_exe().with_file_name("RustFileExplorer.exe");
    let v = ok(&data, &["default", "set", "--yes", "--root", &root, "--exe", &s(&exe)]);
    for part in ["folders", "drives", "explore", "folder_class", "menu", "win_e", "this_pc"] {
        assert_eq!(v["parts"][part], true, "{part}: {v}");
    }
    assert_eq!(v["all"], true);
    let v = ok(&data, &["default", "restore", "--yes", "--root", &root]);
    for part in ["folders", "explore", "folder_class", "menu"] {
        assert_eq!(v["parts"][part], false, "{part} after restore: {v}");
    }
    // a root that is not a test key is refused
    assert_eq!(wfm(&data, &["default", "set", "--yes", "--root", r"HKCU\Software\Classes"]).code, 2);
    let _ = std::process::Command::new("reg").args(["delete", &format!(r"HKCU\Software\RustFileExplorerTest\Cli-{}", std::process::id()), "/f"]).output();
    cleanup(&d);
}
