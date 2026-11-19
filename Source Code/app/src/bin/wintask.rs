#![windows_subsystem = "windows"]
#![allow(non_snake_case)]
//! RustTaskManager.exe: the Rust Task Manager window.
//! Args: `--software` (CPU renderer), `--theme light|dark`, `--no-mica`, `--test` (safe mode for automated tests).
//! `--set-default` / `--restore-default` run the elevated helper only (the window starts it with UAC when the user
//! clicks the button in Settings) and exit.
//! A crash writes %TEMP%\wintask-crash.log (there is no console to print to).
fn main() {
    std::panic::set_hook(Box::new(|info| {
        let p = std::env::temp_dir().join("wintask-crash.log");
        let _ = std::fs::write(p, format!("{info}\n{}", std::backtrace::Backtrace::force_capture()));
    }));
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--set-default") {
        std::process::exit(wt::gui::prefs::run_helper(true));
    }
    if args.iter().any(|a| a == "--restore-default") {
        std::process::exit(wt::gui::prefs::run_helper(false));
    }
    let software = args.iter().any(|a| a == "--software") || std::env::var("WT_SOFTWARE").is_ok();
    let code = wt::gui::app::run(software);
    if code != 0 {
        let _ = std::fs::write(std::env::temp_dir().join("wintask-crash.log"), format!("run() returned {code}"));
    }
    std::process::exit(code);
}
