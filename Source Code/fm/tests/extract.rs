//! "Extract to" / "Extract here" never overwrite, and one undo removes only what that run made.
mod common;
use common::*;

#[test]
fn extract_never_overwrites() {
    let d = tdir("extract");
    // the journal of this test lives in its own data folder
    unsafe { std::env::set_var("WFM_DATA_DIR", d.join("_data")) };
    write(&d.join("src").join("Docs").join("readme.txt"), b"from zip");
    let zip = fmcore::zip::zip(&[d.join("src").join("Docs")], &d.join("z.zip")).unwrap();
    let play = d.join("play");
    std::fs::create_dir_all(&play).unwrap();

    // Extract to: first run makes play\z
    let (p1, moved1) = fmcore::zip::extract_safe(&zip, &play, "z", false).unwrap();
    assert_eq!(p1, play.join("z"));
    assert!(!moved1);
    // the user edits the file and adds one
    write(&p1.join("Docs").join("readme.txt"), b"mine");
    write(&p1.join("Docs").join("mine.txt"), b"keep me");
    // second run: a new folder, the user's files untouched
    let (p2, moved2) = fmcore::zip::extract_safe(&zip, &play, "z", false).unwrap();
    assert_eq!(p2, play.join("z (2)"));
    assert!(moved2);
    assert_eq!(std::fs::read(p1.join("Docs").join("readme.txt")).unwrap(), b"mine");
    assert_eq!(std::fs::read(p2.join("Docs").join("readme.txt")).unwrap(), b"from zip");

    // Extract here with a clash (play\Docs exists after the first "here"): the second set goes to a new folder
    let (h1, c1) = fmcore::zip::extract_safe(&zip, &play, "z", true).unwrap();
    assert_eq!(h1, play);
    assert!(!c1);
    write(&play.join("Docs").join("readme.txt"), b"edited here");
    let (h2, c2) = fmcore::zip::extract_safe(&zip, &play, "z", true).unwrap();
    assert!(c2);
    assert_eq!(h2, play.join("z (3)"));
    assert_eq!(std::fs::read(play.join("Docs").join("readme.txt")).unwrap(), b"edited here");
    // no temporary folder is left behind
    assert!(std::fs::read_dir(&play).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().starts_with(".rfe-extract")));

    // the undo record of the last run names only its own folder
    let ops = fmcore::journal::load();
    let last = ops.last().unwrap();
    assert_eq!(last.kind, "unzip");
    assert_eq!(last.created, vec![play.join("z (3)")]);
    let _ = std::fs::remove_dir_all(&d);
}
