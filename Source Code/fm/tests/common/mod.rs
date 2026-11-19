//! Test helpers. Every test works only inside %TEMP%\wfm-test-* folders, keeps its settings and undo journal in
//! its own data folder (WFM_DATA_DIR), runs wfm hidden, and opens windows only on the never-shown desktop
//! "wfm_hidden".
#![allow(dead_code)]
use serde_json::Value;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn wfm_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_wfm"))
}

/// target\release\examples\savedlg.exe (cargo test builds examples).
pub fn savedlg_exe() -> PathBuf {
    wfm_exe().parent().unwrap().join("examples").join("savedlg.exe")
}

/// A fresh, empty test folder: %TEMP%\wfm-test-<name>-<pid>.
pub fn tdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wfm-test-{name}-{}", std::process::id()));
    assert!(d.file_name().unwrap().to_string_lossy().starts_with("wfm-test-"));
    if d.exists() {
        std::fs::remove_dir_all(&d).unwrap();
    }
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Remove a test folder (only ever a %TEMP%\wfm-test-* folder).
pub fn cleanup(d: &Path) {
    let tmp = std::env::temp_dir();
    assert!(d.starts_with(&tmp) && d.file_name().unwrap().to_string_lossy().starts_with("wfm-test-"), "refusing to remove {}", d.display());
    let _ = std::fs::remove_dir_all(d);
}

pub struct Run {
    pub code: i32,
    pub out: String,
    pub err: String,
}

impl Run {
    /// The JSON document on the last stdout line.
    pub fn json(&self) -> Value {
        let line = self.out.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("null");
        serde_json::from_str(line).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.out))
    }
    /// Every stdout line as JSON.
    pub fn lines(&self) -> Vec<Value> {
        self.out.lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).unwrap()).collect()
    }
}

/// Run wfm hidden with a private data folder.
pub fn wfm(data: &Path, args: &[&str]) -> Run {
    let o = Command::new(wfm_exe())
        .args(args)
        .env("WFM_DATA_DIR", data)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .output()
        .expect("run wfm");
    Run { code: o.status.code().unwrap_or(-1), out: String::from_utf8_lossy(&o.stdout).into_owned(), err: String::from_utf8_lossy(&o.stderr).into_owned() }
}

/// Run wfm and expect exit code 0 and "ok": true; returns the JSON.
pub fn ok(data: &Path, args: &[&str]) -> Value {
    let mut a: Vec<&str> = args.to_vec();
    if !a.contains(&"--json") {
        a.push("--json");
    }
    let r = wfm(data, &a);
    assert_eq!(r.code, 0, "wfm {:?} failed: {} {}", args, r.out, r.err);
    let v = r.json();
    assert_eq!(v["ok"], true, "wfm {:?}: {}", args, r.out);
    v
}

pub fn write(p: &Path, bytes: &[u8]) {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(p, bytes).unwrap();
}

pub fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

pub fn wait_for(limit: Duration, mut f: impl FnMut() -> bool) -> bool {
    let t0 = Instant::now();
    while t0.elapsed() < limit {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// Recycle Bin hygiene: remove this test's own items (originals below its %TEMP%\wfm-test-* folder) from the
/// Recycle Bin. Returns how many were removed.
pub fn purge_bin_under(dir: &Path) -> usize {
    assert!(dir.file_name().unwrap().to_string_lossy().starts_with("wfm-test-"));
    let mut n = 0;
    for r in fmcore::recycle::list_bin(dir) {
        if r.original.starts_with(dir) && fmcore::recycle::purge(&r).is_ok() {
            n += 1;
        }
    }
    n
}

// ---- the never-shown desktop ----

pub mod hidden {
    use std::path::Path;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
    use windows::Win32::System::StationsAndDesktops::{CreateDesktopW, DESKTOP_CONTROL_FLAGS, HDESK};
    use windows::Win32::System::Threading::{
        CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, GetExitCodeProcess, PROCESS_INFORMATION, STARTUPINFOW, TerminateProcess, WaitForSingleObject,
    };
    use windows::core::{PCWSTR, PWSTR};

    pub const DESKTOP: &str = "wfm_hidden";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Create (or open) the hidden desktop. Keep the handle while processes on it run.
    pub fn desktop() -> HDESK {
        let n = wide(DESKTOP);
        unsafe { CreateDesktopW(PCWSTR(n.as_ptr()), PCWSTR::null(), None, DESKTOP_CONTROL_FLAGS(0), 0x1000_0000, None) }.expect("CreateDesktop")
    }

    pub struct Proc {
        pub h: HANDLE,
        pub pid: u32,
    }

    impl Proc {
        /// Wait for exit; Some(exit code), None on timeout.
        pub fn wait(&self, ms: u32) -> Option<u32> {
            unsafe {
                if WaitForSingleObject(self.h, ms) != WAIT_OBJECT_0 {
                    return None;
                }
                let mut c = 0u32;
                GetExitCodeProcess(self.h, &mut c).ok()?;
                Some(c)
            }
        }
        pub fn kill(&self) {
            unsafe {
                let _ = TerminateProcess(self.h, 1);
            }
        }
    }

    impl Drop for Proc {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.h);
            }
        }
    }

    /// Start a process on the hidden desktop (no console window), with extra environment variables.
    pub fn spawn(exe: &Path, args: &str, env: &[(&str, &str)]) -> Proc {
        let mut cmd = wide(&format!("\"{}\" {}", exe.display(), args));
        let mut desk = wide(&format!("WinSta0\\{DESKTOP}"));
        let si = STARTUPINFOW { cb: size_of::<STARTUPINFOW>() as u32, lpDesktop: PWSTR(desk.as_mut_ptr()), ..Default::default() };
        let mut block: Vec<u16> = vec![];
        let mut vars: Vec<(String, String)> = std::env::vars().collect();
        for (k, v) in env {
            vars.retain(|(a, _)| !a.eq_ignore_ascii_case(k));
            vars.push((k.to_string(), v.to_string()));
        }
        vars.sort_by(|a, b| a.0.to_uppercase().cmp(&b.0.to_uppercase()));
        for (k, v) in vars {
            block.extend(format!("{k}={v}").encode_utf16());
            block.push(0);
        }
        block.push(0);
        let mut pi = PROCESS_INFORMATION::default();
        unsafe {
            CreateProcessW(
                PCWSTR::null(),
                Some(PWSTR(cmd.as_mut_ptr())),
                None,
                None,
                false,
                CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
                Some(block.as_ptr() as *const _),
                PCWSTR::null(),
                &si,
                &mut pi,
            )
            .expect("CreateProcess on hidden desktop");
            let _ = CloseHandle(pi.hThread);
        }
        Proc { h: pi.hProcess, pid: pi.dwProcessId }
    }
}
