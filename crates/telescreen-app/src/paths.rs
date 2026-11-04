//! Local data files: the key pair and the trust list (ADR 0004). Never in the config.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The local data directory, for example `~/Library/Application Support/dev.telescreen.telescreen`
/// on macOS or `~/.local/share/telescreen` on Linux. `TELESCREEN_DATA_DIR` overrides it.
pub fn data_dir() -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os("TELESCREEN_DATA_DIR") {
        return Ok(PathBuf::from(dir));
    }
    directories::ProjectDirs::from("dev", "telescreen", "telescreen")
        .map(|dirs| dirs.data_dir().to_owned())
        .ok_or_else(|| "cannot find the home directory".into())
}

pub fn identity_dir() -> Result<PathBuf, String> {
    Ok(data_dir()?.join("identity"))
}

pub fn trust_file() -> Result<PathBuf, String> {
    Ok(data_dir()?.join("trust.json"))
}

/// The file with the local computer ID that the user selected (PEER-3).
pub fn computer_file() -> Result<PathBuf, String> {
    Ok(data_dir()?.join("computer"))
}

/// The default config file (PEER-5).
pub fn default_config() -> Result<PathBuf, String> {
    let home = directories::BaseDirs::new()
        .ok_or("cannot find the home directory")?
        .home_dir()
        .to_owned();
    Ok(default_config_in(
        std::env::var_os("XDG_CONFIG_HOME").as_deref(),
        &home,
    ))
}

/// `$XDG_CONFIG_HOME/telescreen/config.yaml`, else `~/.config/telescreen/config.yaml`,
/// on all platforms. XDG requires that a relative value is ignored.
pub fn default_config_in(xdg_config_home: Option<&OsStr>, home: &Path) -> PathBuf {
    let base = match xdg_config_home.map(Path::new) {
        Some(dir) if dir.is_absolute() => dir.to_owned(),
        _ => home.join(".config"),
    };
    base.join("telescreen").join("config.yaml")
}

/// The saved local computer ID. A missing or empty file means no ID.
pub fn saved_computer(file: &Path) -> Option<String> {
    let text = std::fs::read_to_string(file).ok()?;
    let id = text.trim();
    (!id.is_empty()).then(|| id.to_owned())
}

pub fn save_computer(file: &Path, computer: &str) -> Result<(), String> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    }
    std::fs::write(file, format!("{computer}\n"))
        .map_err(|e| format!("cannot write {}: {e}", file.display()))
}
