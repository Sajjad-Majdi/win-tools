//! Tests of the library API the GUI will use directly: progress callbacks, cancel, pause, the background
//! watcher, listing and natural sort. Settings and the journal go to a private %TEMP%\wfm-test-* folder.
mod common;
use common::*;
use fmcore::ops::{self, Conflict, Control, Progress};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

static INIT: Once = Once::new();

fn setup() {
    INIT.call_once(|| {
        let data = std::env::temp_dir().join(format!("wfm-test-libdata-{}", std::process::id()));
        std::fs::create_dir_all(&data).unwrap();
        // SAFETY: set once, before any test reads it (every test calls setup() first).
        unsafe { std::env::set_var("WFM_DATA_DIR", &data) };
    });
}

#[test]
fn copy_progress_and_cancel() {
    setup();
    let d = tdir("libcopy");
    let big: Vec<u8> = vec![42; 64 * 1024 * 1024];
    write(&d.join("big.bin"), &big);
    let out = d.join("out");
    std::fs::create_dir_all(&out).unwrap();

    // Full copy: progress grows to the total.
    let ctl = Control::default();
    let mut events: Vec<Progress> = vec![];
    let rep = ops::copy(&[d.join("big.bin")], &out, Conflict::Fail, &ctl, &mut |p| events.push(p.clone())).unwrap();
    assert!(rep.errors.is_empty() && !rep.cancelled);
    assert_eq!(rep.bytes, big.len() as u64);
    assert!(events.windows(2).all(|w| w[1].bytes_done >= w[0].bytes_done), "progress never goes back");
    let last = events.last().unwrap();
    assert!(last.finished && last.bytes_done == last.bytes_total && last.files_done == 1);
    assert!(rep.journal_id > 0);

    // Cancel in the middle of a file (a 320 MB file is copied unbuffered, so it takes long enough to get progress
    // events): the partial target is removed by Windows.
    {
        use std::io::Write;
        let mut f = std::fs::File::create(d.join("huge.bin")).unwrap();
        let chunk = vec![7u8; 1024 * 1024];
        for _ in 0..320 {
            f.write_all(&chunk).unwrap();
        }
    }
    let ctl = Control::default();
    let c2 = ctl.clone();
    let mut mid = 0u64;
    let rep = ops::copy(&[d.join("huge.bin")], &d.join("cancelled.bin"), Conflict::Fail, &ctl, &mut |p| {
        if p.bytes_done > 0 && !p.finished && mid == 0 {
            mid = p.bytes_done;
            c2.cancel.store(true, Ordering::Relaxed);
        }
    })
    .unwrap();
    assert!(rep.cancelled, "{rep:?}");
    assert!(mid > 0 && mid < 320 * 1024 * 1024, "cancelled in the middle ({mid} bytes)");
    assert!(!d.join("cancelled.bin").exists(), "no half-written file is left");
    std::fs::remove_file(d.join("huge.bin")).unwrap();

    // Pause: a paused copy makes no progress until resumed (the GUI's pause button).
    let ctl = Control::default();
    ctl.pause.store(true, Ordering::Relaxed);
    let c3 = ctl.clone();
    let src = d.join("big.bin");
    let dst = d.join("paused.bin");
    let seen = Arc::new(Mutex::new(0u64));
    let seen2 = seen.clone();
    let t = std::thread::spawn(move || {
        ops::copy(&[src], &dst, Conflict::Fail, &c3, &mut |p| *seen2.lock().unwrap() = p.bytes_done).unwrap()
    });
    std::thread::sleep(Duration::from_millis(600));
    let during = *seen.lock().unwrap();
    assert!(during < big.len() as u64, "paused copy did not finish");
    ctl.pause.store(false, Ordering::Relaxed);
    let rep = t.join().unwrap();
    assert!(rep.errors.is_empty() && rep.bytes == big.len() as u64);
    assert_eq!(std::fs::metadata(d.join("paused.bin")).unwrap().len(), big.len() as u64);
    purge_bin_under(&d);
    cleanup(&d);
}

#[test]
fn background_watcher() {
    setup();
    let d = tdir("libwatch");
    let got = Arc::new(Mutex::new(vec![]));
    let g2 = got.clone();
    let w = fmcore::watch::Watcher::start(&d, true, move |c| g2.lock().unwrap().push((c.kind, c.path.clone()))).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    std::fs::create_dir_all(d.join("sub")).unwrap();
    write(&d.join("sub").join("deep.txt"), b"x");
    assert!(wait_for(Duration::from_secs(5), || got.lock().unwrap().iter().any(|(k, p)| *k == fmcore::watch::ChangeKind::Added && p.ends_with("deep.txt"))));
    let t0 = Instant::now();
    w.stop();
    assert!(t0.elapsed() < Duration::from_secs(1), "stop is immediate");
    cleanup(&d);
}

#[test]
fn listing_and_natural_sort() {
    setup();
    let d = tdir("liblist");
    for n in ["File10.txt", "file2.txt", "File1.txt", "a.TXT"] {
        write(&d.join(n), b"x");
    }
    let mut v = fmcore::list::list_dir(&d, false).unwrap();
    fmcore::list::sort_entries(&mut v, fmcore::list::SortKey::Name, false, true);
    let names: Vec<&str> = v.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["a.TXT", "File1.txt", "file2.txt", "File10.txt"]);
    assert_eq!(v[0].ext(), "txt");
    assert!(fmcore::list::list_dir(&d.join("missing"), false).is_err());
    assert_eq!(fmcore::list::natural_cmp("x9", "x10"), std::cmp::Ordering::Less);
    assert_eq!(fmcore::list::natural_cmp("x010", "x10"), std::cmp::Ordering::Less, "zero-padded first, like Explorer");
    cleanup(&d);
}

#[test]
fn search_query_args() {
    let q = fmcore::search::Query { words: vec!["invoice".into()], exts: vec!["pdf".into(), ".docx".into()], size: Some(">10mb".into()), modified: Some("7d".into()), folders: Some(false), top: 5, ..Default::default() };
    let a = fmcore::search::es_args(&q);
    for want in ["invoice", "ext:pdf;docx", "size:>10mb", "dm:last7days", "file:"] {
        assert!(a.contains(&want.to_string()), "{want} missing in {a:?}");
    }
    assert!(a.windows(2).any(|w| w[0] == "-n" && w[1] == "5"));
}
