//! Configuration and R discovery for portable archives and native installers.
use std::path::{Path, PathBuf};

pub fn user_directory() -> PathBuf {
    #[cfg(windows)]
    if let Some(base) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(base).join("ShinyHarbor");
    }
    #[cfg(target_os = "macos")]
    if let Some(base) = std::env::var_os("HOME") {
        return PathBuf::from(base).join("Library/Application Support/ShinyHarbor");
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(base) = std::env::var_os("XDG_DATA_HOME") {
            return PathBuf::from(base).join("shinyharbor");
        }
        if let Some(base) = std::env::var_os("HOME") {
            return PathBuf::from(base).join(".local/share/shinyharbor");
        }
    }
    PathBuf::from("shinyharbor-data")
}

pub fn default_configuration() -> PathBuf {
    if let Some(path) = std::env::var_os("SHINYHARBOR_CONFIG") {
        return path.into();
    }
    let mut candidates = vec![PathBuf::from("harbor.toml"), "shiny-server.conf".into()];
    if let Ok(executable) = std::env::current_exe()
        && let Some(directory) = executable.parent()
    {
        // The native installer writes this UTF-8 pointer for custom data drives.
        if let Ok(path) = std::fs::read_to_string(directory.join("data-directory.txt")) {
            let directory = PathBuf::from(path.trim().trim_start_matches('\u{feff}'));
            if directory.is_absolute() {
                candidates.push(directory.join("harbor.toml"));
            }
        }
        candidates.push(directory.join("harbor.toml"));
        candidates.push(directory.join("shiny-server.conf"));
    }
    candidates.push(user_directory().join("harbor.toml"));
    #[cfg(windows)]
    if let Some(base) = std::env::var_os("PROGRAMDATA") {
        candidates.push(PathBuf::from(base).join("ShinyHarbor/harbor.toml"));
    }
    #[cfg(unix)]
    candidates.extend([
        PathBuf::from("/etc/shinyharbor/harbor.toml"),
        PathBuf::from("/etc/shiny-server/shiny-server.conf"),
    ]);
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .unwrap_or_else(|| user_directory().join("harbor.toml"))
}

pub fn rscript() -> PathBuf {
    let filename = if cfg!(windows) {
        "Rscript.exe"
    } else {
        "Rscript"
    };
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            let candidate = directory.join(filename);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    if let Some(base) = std::env::var_os("R_HOME") {
        let candidate = PathBuf::from(base).join("bin").join(filename);
        if candidate.is_file() {
            return candidate;
        }
    }
    #[cfg(windows)]
    {
        for variable in ["ProgramW6432", "ProgramFiles", "LOCALAPPDATA"] {
            let Some(base) = std::env::var_os(variable) else {
                continue;
            };
            let Ok(entries) = std::fs::read_dir(PathBuf::from(base).join("R")) else {
                continue;
            };
            let mut versions: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
            versions.sort_by_key(|path| {
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .split(|c: char| !c.is_ascii_digit())
                    .filter_map(|part| part.parse::<u32>().ok())
                    .collect::<Vec<_>>()
            });
            for version in versions.into_iter().rev() {
                for subdirectory in ["bin", "bin/x64", "bin/aarch64"] {
                    let candidate = version.join(subdirectory).join(filename);
                    if candidate.is_file() {
                        return candidate;
                    }
                }
            }
        }
    }
    for path in [
        "/Library/Frameworks/R.framework/Resources/bin/Rscript",
        "/opt/homebrew/bin/Rscript",
        "/usr/local/bin/Rscript",
        "/usr/bin/Rscript",
    ] {
        if Path::new(path).is_file() {
            return path.into();
        }
    }
    filename.into()
}

pub fn r_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    if cfg!(windows) {
        if let Some(rest) = text.strip_prefix("\\\\?\\UNC\\") {
            return format!("//{}", rest.replace('\\', "/"));
        }
        return text
            .strip_prefix("\\\\?\\")
            .unwrap_or(&text)
            .replace('\\', "/");
    }
    text.into_owned()
}

pub fn library_path(library: &Path) -> String {
    let separator = if cfg!(windows) { ";" } else { ":" };
    let mut value = r_path(library);
    if let Ok(existing) = std::env::var("R_LIBS")
        && !existing.is_empty()
    {
        value.push_str(separator);
        value.push_str(&existing);
    }
    value
}
