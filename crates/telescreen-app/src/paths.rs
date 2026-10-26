//! Local data files: the key pair and the trust list (ADR 0004). Never in the config.

use std::path::PathBuf;

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
