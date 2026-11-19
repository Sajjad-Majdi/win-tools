//! Adds the app icon and the version resource (ProductName, CompanyName, FileDescription, version) to wintask.exe and
//! wtm.exe with windres (GNU toolchain). The GNU preprocessor breaks on paths with spaces, so the inputs are staged in
//! a space-free temp folder first. On other toolchains the exes build without resources.
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/wintask.ico");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("gnu") {
        println!("cargo:warning=no icon/version resource (needs the GNU toolchain with windres)");
        return;
    }
    let stage = PathBuf::from(std::env::var("TEMP").or_else(|_| std::env::var("TMP")).unwrap_or_else(|_| ".".into())).join("wintools-res-app");
    std::fs::create_dir_all(&stage).unwrap();
    std::fs::copy("assets/wintask.ico", stage.join("app.ico")).unwrap();
    let v = std::env::var("CARGO_PKG_VERSION").unwrap();
    let n: Vec<&str> = v.split('.').collect();
    let rc = format!(
        "1 ICON \"app.ico\"
1 VERSIONINFO
FILEVERSION {0},{1},{2},0
PRODUCTVERSION {0},{1},{2},0
BEGIN
 BLOCK \"StringFileInfo\"
 BEGIN
  BLOCK \"040904B0\"
  BEGIN
   VALUE \"CompanyName\", \"Sajjad Majdi\"
   VALUE \"FileDescription\", \"Win Tools Task Manager\"
   VALUE \"FileVersion\", \"{3}\"
   VALUE \"ProductName\", \"Win Tools Task Manager\"
   VALUE \"ProductVersion\", \"{3}\"
   VALUE \"LegalCopyright\", \"Copyright (c) 2026 Sajjad Majdi. MIT License.\"
  END
 END
 BLOCK \"VarFileInfo\"
 BEGIN
  VALUE \"Translation\", 0x409, 1200
 END
END
",
        n[0], n[1], n[2], v
    );
    std::fs::write(stage.join("app.rc"), rc).unwrap();
    let ok = Command::new("windres").current_dir(&stage).args(["-O", "coff", "-i", "app.rc", "-o", "app_res.o"]).status().map(|s| s.success()).unwrap_or(false);
    if !ok {
        println!("cargo:warning=windres failed: exes built without icon/version resource");
        return;
    }
    println!("cargo:rustc-link-arg-bins={}", stage.join("app_res.o").display());
}
