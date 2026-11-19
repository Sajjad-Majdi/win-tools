//! Zip and unzip with what Windows already has, run hidden:
//! - Windows' own tar.exe (bsdtar) for the normal case (fast, native).
//! - .NET's ZipArchive (through a hidden PowerShell) when a name has characters outside ASCII: Windows' tar.exe
//!   3.8.x crashes (access violation) on names such as Persian ones, and .NET writes and reads UTF-8 names right.
use crate::util::*;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

fn tar_exe() -> PathBuf {
    let sys = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    sys.join(r"System32\tar.exe")
}

fn run_tar(args: &[std::ffi::OsString]) -> Res<()> {
    let tar = tar_exe();
    if !tar.is_file() {
        return Err("tar.exe not found (it ships with Windows 10 1803 and later)".into());
    }
    let out = hidden_command(&tar).args(args).output().map_err(|e| format!("cannot run tar.exe: {e}"))?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(format!("tar.exe failed: {}", if msg.is_empty() { format!("exit code 0x{:X}", out.status.code().unwrap_or(-1) as u32) } else { msg }));
    }
    Ok(())
}

fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut o = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        o.push(T[(n >> 18) as usize & 63] as char);
        o.push(T[(n >> 12) as usize & 63] as char);
        o.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        o.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    o
}

/// PowerShell single-quoted literal.
fn psq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn run_ps(script: &str) -> Res<()> {
    let sys = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let ps = sys.join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    let enc: Vec<u8> = script.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let out = hidden_command(&ps)
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand", &b64(&enc)])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("cannot run PowerShell: {e}"))?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr);
        let line = msg.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("unknown error").to_string();
        return Err(format!("zip failed: {line}"));
    }
    Ok(())
}

fn ascii(s: &std::ffi::OsStr) -> bool {
    s.to_string_lossy().is_ascii() && s.to_str().is_some()
}

/// Is every name below `p` (and `p` itself) plain ASCII?
fn tree_ascii(p: &Path) -> bool {
    if !ascii(p.as_os_str()) {
        return false;
    }
    let mut stack = vec![p.to_path_buf()];
    while let Some(d) = stack.pop() {
        if !d.is_dir() {
            continue;
        }
        let Ok(list) = crate::list::list_dir(&d, true) else { continue };
        for e in list {
            if !e.name.is_ascii() {
                return false;
            }
            if e.is_dir() && e.attrs & ATTR_REPARSE == 0 {
                stack.push(d.join(&e.name));
            }
        }
    }
    true
}

/// Are all entry names in this .zip plain ASCII? (reads only the central directory; None when it cannot tell)
pub fn zip_names_ascii(zip: &Path) -> Option<bool> {
    let mut f = std::fs::File::open(zip).ok()?;
    let len = f.metadata().ok()?.len();
    let tail = len.min(66 * 1024);
    f.seek(SeekFrom::Start(len - tail)).ok()?;
    let mut buf = vec![0u8; tail as usize];
    f.read_exact(&mut buf).ok()?;
    let eocd = (0..buf.len().saturating_sub(21)).rev().find(|&i| buf[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])?;
    let count = u16::from_le_bytes([buf[eocd + 10], buf[eocd + 11]]) as usize;
    let size = u32::from_le_bytes(buf[eocd + 12..eocd + 16].try_into().ok()?) as u64;
    let off = u32::from_le_bytes(buf[eocd + 16..eocd + 20].try_into().ok()?) as u64;
    if off == 0xFFFF_FFFF || size == 0xFFFF_FFFF || off + size > len {
        return None; // zip64: let .NET handle it
    }
    let mut cd = vec![0u8; size as usize];
    f.seek(SeekFrom::Start(off)).ok()?;
    f.read_exact(&mut cd).ok()?;
    let mut i = 0usize;
    for _ in 0..count {
        if i + 46 > cd.len() || cd[i..i + 4] != [0x50, 0x4b, 0x01, 0x02] {
            return None;
        }
        let n = u16::from_le_bytes([cd[i + 28], cd[i + 29]]) as usize;
        let x = u16::from_le_bytes([cd[i + 30], cd[i + 31]]) as usize;
        let c = u16::from_le_bytes([cd[i + 32], cd[i + 33]]) as usize;
        let name = cd.get(i + 46..i + 46 + n)?;
        if !name.is_ascii() {
            return Some(false);
        }
        i += 46 + n + x + c;
    }
    Some(true)
}

/// Pack files and folders into a .zip. Each item goes in under its own name. Fails when `out` exists.
pub fn zip(srcs: &[PathBuf], out: &Path) -> Res<PathBuf> {
    let out = abs(out);
    if out.exists() {
        return Err(format!("already exists: {}", out.display()));
    }
    if srcs.is_empty() {
        return Err("nothing to zip".into());
    }
    let srcs: Vec<PathBuf> = srcs.iter().map(abs).collect();
    for s in &srcs {
        if !s.exists() {
            return Err(format!("not found: {}", s.display()));
        }
        if s.parent().is_none() || s.file_name().is_none() {
            return Err(format!("cannot zip a drive root: {}", s.display()));
        }
    }
    let plain = ascii(out.as_os_str()) && srcs.iter().all(|s| tree_ascii(s));
    let r = if plain {
        // -a picks the format from the .zip suffix.
        let mut args: Vec<std::ffi::OsString> = vec!["-a".into(), "-c".into(), "-f".into(), out.clone().into_os_string()];
        for s in &srcs {
            args.push("-C".into());
            args.push(s.parent().unwrap().as_os_str().to_owned());
            args.push(s.file_name().unwrap().to_owned());
        }
        run_tar(&args)
    } else {
        let items = srcs.iter().map(|s| psq(&s.to_string_lossy())).collect::<Vec<_>>().join(",");
        let script = format!(
            "$ErrorActionPreference='Stop'\nAdd-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem\n\
             $z = [System.IO.Compression.ZipFile]::Open({}, 'Create')\n\
             try {{ foreach ($item in @({items})) {{\n\
               $i = Get-Item -LiteralPath $item -Force\n\
               if ($i.PSIsContainer) {{\n\
                 [void]$z.CreateEntry($i.Name + '/')\n\
                 Get-ChildItem -LiteralPath $i.FullName -Recurse -Force | ForEach-Object {{\n\
                   $rel = $i.Name + '/' + $_.FullName.Substring($i.FullName.Length + 1).Replace('\\','/')\n\
                   if ($_.PSIsContainer) {{ [void]$z.CreateEntry($rel + '/') }}\n\
                   else {{ [void][System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($z, $_.FullName, $rel, 'Optimal') }} }}\n\
               }} else {{ [void][System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($z, $i.FullName, $i.Name, 'Optimal') }}\n\
             }} }} finally {{ $z.Dispose() }}\n",
            psq(&out.to_string_lossy())
        );
        run_ps(&script)
    };
    if let Err(e) = r {
        let _ = std::fs::remove_file(&out);
        return Err(e);
    }
    let mut op = crate::journal::Op::new("zip");
    op.created.push(out.clone());
    crate::journal::record_quiet(op);
    Ok(out)
}

/// Unpack an archive into `dir` (created when missing). .zip always works; .tar, .tgz, .7z and others go through
/// tar.exe. Existing files with the same names are replaced.
pub fn unzip(archive: &Path, dir: &Path) -> Res<PathBuf> {
    let archive = abs(archive);
    let dir = abs(dir);
    if !archive.is_file() {
        return Err(format!("not found: {}", archive.display()));
    }
    let created = !dir.exists();
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let before: std::collections::HashSet<String> = crate::list::list_dir(&dir, true)?.into_iter().map(|e| e.name.to_lowercase()).collect();
    let is_zip = std::fs::File::open(&archive).ok().and_then(|mut f| {
        let mut m = [0u8; 4];
        f.read_exact(&mut m).ok().map(|_| m == [0x50, 0x4b, 0x03, 0x04] || m == [0x50, 0x4b, 0x05, 0x06])
    }) == Some(true);
    let plain = ascii(archive.as_os_str()) && ascii(dir.as_os_str()) && (!is_zip || zip_names_ascii(&archive) == Some(true));
    let r = if plain || !is_zip {
        let args: Vec<std::ffi::OsString> = vec!["-x".into(), "-f".into(), archive.clone().into_os_string(), "-C".into(), dir.clone().into_os_string()];
        run_tar(&args)
    } else {
        let script = format!(
            "$ErrorActionPreference='Stop'\nAdd-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem\n\
             $dir = [System.IO.Path]::GetFullPath({}) + '\\'\n\
             $z = [System.IO.Compression.ZipFile]::OpenRead({})\n\
             try {{ foreach ($e in $z.Entries) {{\n\
               $dest = [System.IO.Path]::GetFullPath([System.IO.Path]::Combine($dir, $e.FullName))\n\
               if (-not $dest.StartsWith($dir, [System.StringComparison]::OrdinalIgnoreCase)) {{ throw ('unsafe path in the zip: ' + $e.FullName) }}\n\
               if ($e.FullName.EndsWith('/')) {{ [void][System.IO.Directory]::CreateDirectory($dest); continue }}\n\
               [void][System.IO.Directory]::CreateDirectory([System.IO.Path]::GetDirectoryName($dest))\n\
               [System.IO.Compression.ZipFileExtensions]::ExtractToFile($e, $dest, $true)\n\
             }} }} finally {{ $z.Dispose() }}\n",
            psq(&dir.to_string_lossy()),
            psq(&archive.to_string_lossy())
        );
        run_ps(&script)
    };
    r?;
    let mut op = crate::journal::Op::new("unzip");
    if created {
        op.created.push(dir.clone());
    } else {
        for e in crate::list::list_dir(&dir, true)? {
            if !before.contains(&e.name.to_lowercase()) {
                op.created.push(dir.join(&e.name));
            }
        }
    }
    crate::journal::record_quiet(op);
    Ok(dir)
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64() {
        assert_eq!(super::b64(b"Man"), "TWFu");
        assert_eq!(super::b64(b"Ma"), "TWE=");
        assert_eq!(super::b64(b"M"), "TQ==");
    }
}
