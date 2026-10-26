//! Layer 3 of `docs/config.md`: semantic checks. Parity with `tools/validate_config.py`.

use std::borrow::Cow;
use std::collections::BTreeSet;

use super::{Config, Device, Driver, Operation};

/// Returns one message for each semantic error. An empty list means valid.
///
/// Checks: a device `id` is unique in one computer; a handoff target exists, is
/// not the computer itself, and has a device with the same `id`; the executable
/// (`argv[0]`) and `cwd` of each `command` operation are absolute paths in POSIX
/// or Windows form.
pub fn errors(config: &Config) -> Vec<String> {
    let mut errors = Vec::new();
    for (computer_id, computer) in &config.computers {
        let mut seen = BTreeSet::new();
        for (alias, device) in &computer.devices {
            let where_ = format!("{computer_id}.devices.{alias}");
            if !seen.insert(&device.id) {
                errors.push(format!("{where_}: duplicate device id {}", device.id));
            }
            for target in device.handoff.keys() {
                if target == computer_id {
                    errors.push(format!("{where_}: cannot handoff to self"));
                } else if let Some(target_peer) = config.computers.get(target) {
                    if target_peer.device_by_id(&device.id).is_none() {
                        errors.push(format!("{where_}: {target} has no device {}", device.id));
                    }
                } else {
                    errors.push(format!("{where_}: unknown target {target}"));
                }
            }
            for operation in operations(device) {
                command_errors(operation, &where_, &mut errors);
            }
        }
    }
    errors
}

fn operations(device: &Device) -> impl Iterator<Item = &Operation> {
    device
        .portal_match
        .iter()
        .chain(device.input_target.iter())
        .chain(
            device
                .handoff
                .values()
                .flat_map(|handoff| handoff.steps.iter().chain(handoff.verify.iter())),
        )
}

fn command_errors(operation: &Operation, where_: &str, errors: &mut Vec<String>) {
    let Driver::Command(options) = &operation.driver else {
        return;
    };
    let paths = options
        .argv
        .first()
        .map(|path| Cow::Borrowed(path.as_str()))
        .into_iter()
        .chain(options.cwd.as_ref().map(|path| path.to_string_lossy()));
    for path in paths {
        if !absolute_path(&path) {
            errors.push(format!("{where_}: command path must be absolute: {path}"));
        }
    }
}

fn absolute_path(path: &str) -> bool {
    path.starts_with('/')
        || path.starts_with("\\\\")
        || matches!(path.as_bytes(), [drive, b':', b'/' | b'\\', ..] if drive.is_ascii_alphabetic())
}
