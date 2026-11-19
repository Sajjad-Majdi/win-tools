# Rust Task Manager and Rust File Explorer

Two small, fast replacements for built-in Windows tools, written in Rust with plain Win32 and Direct2D (no web view):

- **Rust Task Manager** (`RustTaskManager.exe`): opens in a few milliseconds. One list sorted by real impact, same-name
  processes merged, problems shown up front (an app not responding, CPU high for minutes, memory still growing),
  "Why is this running?" (startup entry, scheduled task, service, parent), "Free up memory", Restart, and startup apps.
- **Rust File Explorer** (`RustFileExplorer.exe`): a file manager with instant search as you type (through
  [Everything](https://www.voidtools.com/), if installed), folder sizes, quick preview with Space, tabs, split view,
  a copy queue with pause and Undo, batch rename and the normal Windows right-click menu.

Both apps follow the Windows light/dark theme and accent color, and every action is also available from a command line
(`wtm` and `wfm`, JSON output), so scripts and AI agents can use them without the window.

## Download

Get the latest files from the [Releases](../../releases) page:

| File | What it is |
|---|---|
| `RustTaskManager-<version>-setup.exe` | Installer for the Task Manager |
| `RustTaskManager-<version>-portable.exe` | The Task Manager without installing |
| `RustFileExplorer-<version>-setup.exe` | Installer for the file manager |
| `RustFileExplorer-<version>-portable.exe` | The file manager without installing |

Windows 10 or 11, 64-bit.

Upgrading from 0.1.0 ("Win Tools"): just run the new installer. It removes the old copy and its Start menu entry.

## Install and uninstall

- **Install:** run the `-setup.exe` file. It installs for the current user only, in
  `%LOCALAPPDATA%\Programs\<app name>`, adds a Start menu shortcut, and needs no administrator rights.
- **Uninstall:** Settings > Apps > Installed apps > the app > Uninstall, or run `uninstall.exe` in the install folder.
  Your settings in `%LOCALAPPDATA%\WinTools` are kept; delete that folder to remove them too.
- **Portable:** the `-portable.exe` files run from anywhere and change nothing on install.
- **Everything:** the Rust File Explorer setup also installs [Everything](https://www.voidtools.com/) (voidtools, MIT)
  when it is not on the PC, for instant whole-PC search and folder sizes. Windows asks for administrator rights for that
  step only. Without Everything, search walks the current folder instead.

The installer never changes system settings.

## Making them the default (optional)

Each app has a button in its Settings. Nothing changes until you press it.

- **Task Manager:** "Set as default Task Manager" makes Ctrl+Shift+Esc and the taskbar's Task Manager entry open Rust Task Manager.
  It sets the Windows "Image File Execution Options" entry for `Taskmgr.exe`, so Windows asks for administrator rights.
  **Warning:** while it is set, Windows' own Task Manager does not open. Use "Restore Windows Task Manager" in the same
  place to undo it, and do it before you uninstall.
- **File Explorer:** "Set as default file manager" makes Win+E and opened folders use Rust File Explorer for your user account.
  "Restore Explorer" undoes it. File dialogs inside other programs (Open / Save As) stay the Windows ones.

## AI features (optional, off by default)

"What is this?" (for a process or a file) and batch-rename name suggestions can use Google Gemini:

- They are **off** until you turn them on in Settings > AI features and enter **your own** Gemini API key
  ([get a free key](https://aistudio.google.com/apikey)). The key is stored encrypted for your Windows user (DPAPI).
- What is sent is listed next to the switch: for a process its name, path, file description, publisher, window title
  and command line; for a file its name, type, size and first lines. Nothing is sent while AI is off.
- Common Windows processes and file types are explained by a built-in table and need no AI. Facts read on your PC
  (publisher, signature, path) are always shown and never replaced by AI text.
- If you use a proxy or VPN, the apps use `HTTPS_PROXY` or the Windows proxy settings.

**Meaning search** in Rust File Explorer ("By meaning") is also optional and off by default. It needs a local semantic index;
the button only appears when one is installed and the switch is on.

## Command line

```
wtm help        (Task Manager: list, find, info, why, kill, restart, free, anomalies, startup, services, perf, gui ...)
wfm help        (File Explorer: ls, find, copy, move, rename, delete, undo, zip, bin, thumb, dialog save, gui ...)
```

Add `--json` to any command for machine-readable output.

## Build from source

Rust (stable) on Windows, then:

```
cd "Source Code/app" && cargo build --release
cd "Source Code/fm"  && cargo build --release
```

Installers: `powershell -File "Source Code/tools/release.ps1" -Online` (needs NSIS).

## Code signing policy

Free code signing provided by [SignPath.io](https://about.signpath.io), certificate by [SignPath Foundation](https://signpath.org).

Team roles:

- Committers and reviewers: [Sajjad Majdi](https://github.com/Sajjad-Majdi)
- Approvers: [Sajjad Majdi](https://github.com/Sajjad-Majdi)

Only release builds made by this repository's GitHub Actions workflow from tagged commits are signed.

## Privacy policy

This program will not transfer any information to other networked systems unless specifically requested by the user
or the person installing or operating it. The only network use is the optional AI feature described above, which sends
data to Google Gemini with the user's own key only after the user turns it on.

## License

Written in Rust. Not affiliated with or endorsed by the Rust Project or the Rust Foundation.


[MIT](LICENSE). Third-party notices: `Source Code/installer/LICENSES.txt`.
