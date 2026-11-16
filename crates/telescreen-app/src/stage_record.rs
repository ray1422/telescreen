//! The process record used to give the desktop session priority (STG-11).

use std::path::Path;
use std::process::Command;

use crate::paths;

pub fn write() -> Result<(), String> {
    let dir = paths::data_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    let path = dir.join("stage.pid");
    std::fs::write(&path, format!("{}\n", std::process::id()))
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

pub fn remove() {
    if let Ok(dir) = paths::data_dir() {
        let _ = std::fs::remove_file(dir.join("stage.pid"));
    }
}

pub fn stop_recorded() -> bool {
    let Ok(dir) = paths::data_dir() else {
        return false;
    };
    let Ok(text) = std::fs::read_to_string(dir.join("stage.pid")) else {
        return false;
    };
    let Some(pid) = parse_pid(&text) else {
        return false;
    };
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline"));
    if !cmdline.as_deref().is_ok_and(is_stage_command) {
        remove();
        return false;
    }
    Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .is_ok_and(|status| status.success())
}

fn parse_pid(text: &str) -> Option<u32> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok().filter(|pid| *pid > 0)
}

fn is_stage_command(bytes: &[u8]) -> bool {
    use std::os::unix::ffi::OsStrExt;

    let mut args = bytes.split(|byte| *byte == 0);
    let Some(program) = args.next() else {
        return false;
    };
    Path::new(std::ffi::OsStr::from_bytes(program))
        .file_name()
        .is_some_and(|name| name == "telescreen")
        && args.any(|arg| arg == b"--stage" || arg.starts_with(b"--stage="))
}

#[cfg(test)]
mod tests {
    use super::{is_stage_command, parse_pid};

    #[test]
    fn parses_only_positive_decimal_process_ids() {
        assert_eq!(parse_pid("123\n"), Some(123));
        for text in ["", "abc", "0", "-12", "+12", "12 34", "4294967296"] {
            assert_eq!(parse_pid(text), None, "{text}");
        }
    }

    #[test]
    fn requires_the_program_and_the_stage_argument() {
        assert!(is_stage_command(
            b"/usr/bin/telescreen\0run\0--stage\0login\0"
        ));
        assert!(is_stage_command(b"telescreen\0run\0--stage\0login\0"));
        assert!(is_stage_command(b"telescreen\0run\0--stage=login\0"));
        for bytes in [
            &b""[..],
            &b"telescreen\0run\0"[..],
            &b"other\0--stage\0telescreen\0"[..],
            &b"telescreen\0--stagecoach\0"[..],
        ] {
            assert!(!is_stage_command(bytes));
        }
    }
}
