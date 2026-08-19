use std::env;
use std::path::{Path, PathBuf};

pub fn zene_home() -> PathBuf {
    if let Ok(home) = env::var("ZENE_HOME") {
        if !home.is_empty() {
            return PathBuf::from(home);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".zene")
}

pub fn sessions_dir() -> PathBuf {
    zene_home().join("sessions")
}

pub fn workdir_slug(workdir: &Path) -> String {
    let canonical = workdir
        .canonicalize()
        .unwrap_or_else(|_| workdir.to_path_buf());
    let raw = canonical.display().to_string();
    raw.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}
