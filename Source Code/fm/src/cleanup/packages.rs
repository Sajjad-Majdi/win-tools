//! Download caches with tool-specific locations. Keep project and tool config out of these roots.
use super::{key, under};
use std::path::{Path, PathBuf};

pub(super) fn roots(profile: &str, local: Option<&str>, roaming: Option<&str>) -> Vec<String> {
    let mut roots = Vec::new();
    let mut add = |base: &Path, dirs: &[&str]| {
        for dir in dirs {
            let path = base.join(dir);
            if let Some(name) = path.to_str().and_then(|p| key(Path::new(p.trim_end_matches(['\\', '/']))))
                && name.len() > 3 && path.is_dir()
            {
                roots.push(name);
            }
        }
    };
    add(Path::new(profile), &[
        r".cargo\registry", r".gradle\caches\modules-2", r".m2\repository", r".nuget\packages", r".yarn\berry\cache",
        r".cache\huggingface\hub", r".conda\pkgs", r"miniconda3\pkgs", r"anaconda3\pkgs", r"miniforge3\pkgs",
    ]);
    if let Some(local) = local {
        add(Path::new(local), &[
            r"uv\cache", r"pip\cache", r"npm-cache\_cacache", r"npm-cache\_npx", r"Yarn\Cache", r"Yarn\Berry\cache",
            r"pnpm\store", "pnpm-cache", r"NuGet\v3-cache", r"conda\conda\pkgs",
        ]);
    }
    if let Some(roaming) = roaming {
        add(Path::new(roaming), &[r"npm-cache\_cacache", r"npm-cache\_npx"]);
    }
    for (var, dirs) in [
        ("UV_CACHE_DIR", &[""][..]), ("PIP_CACHE_DIR", &[""]),
        ("npm_config_cache", &["_cacache", "_npx"]), ("YARN_CACHE_FOLDER", &[""]),
        ("YARN_GLOBAL_FOLDER", &["cache"]), ("PNPM_HOME", &["store"]),
        ("npm_config_store_dir", &[""]), ("npm_config_cache_dir", &[""]),
        ("CARGO_HOME", &["registry"]), ("NUGET_PACKAGES", &[""]), ("NUGET_HTTP_CACHE_PATH", &[""]),
        ("GRADLE_USER_HOME", &[r"caches\modules-2"]), ("MAVEN_USER_HOME", &["repository"]),
        ("HF_HOME", &["hub"]), ("HF_HUB_CACHE", &[""]), ("HUGGINGFACE_HUB_CACHE", &[""]),
        ("XDG_CACHE_HOME", &[r"huggingface\hub"]),
    ] {
        if let Some(base) = std::env::var_os(var).filter(|p| !p.is_empty()) {
            add(Path::new(&base), dirs);
        }
    }
    if let Some(dirs) = std::env::var_os("CONDA_PKGS_DIRS") {
        for dir in std::env::split_paths(&dirs) {
            add(&dir, &[""]);
        }
    }
    if let Some(exe) = std::env::var_os("CONDA_EXE").map(PathBuf::from)
        && let Some(base) = exe.parent().and_then(Path::parent)
    {
        add(base, &["pkgs"]);
    }
    roots.sort_unstable();
    roots.dedup();
    roots
}

pub(super) fn contains(roots: &[String], path: &str) -> bool {
    roots.iter().any(|root| path == root || under(path, root))
}
