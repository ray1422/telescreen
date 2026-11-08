//! Layer 3 of `docs/config.md`: semantic checks. Parity with `tools/validate_config.py`.

use std::collections::BTreeSet;

use super::{Config, Device, Driver, Operation};

/// Returns one message for each semantic error. An empty list means valid.
///
/// Checks: a device `id` is unique in one computer; a handoff target exists, is
/// not the computer itself, and has a device with the same `id`; a program with a
/// path, each `cwd`, and each `path` entry are absolute paths in POSIX or Windows
/// form; special key groups are not in both lists of one computer. The typed
/// model already split the `command` strings.
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
                program_errors(operation, &where_, &mut errors);
            }
        }
        for group in computer
            .special_keys
            .send_to_target
            .intersection(&computer.special_keys.keep_on_source)
        {
            errors.push(format!(
                "{computer_id}.special-keys: {} is in send-to-target and keep-on-source",
                group.name()
            ));
        }
        for directory in computer.path.iter().flatten() {
            let directory = directory.to_string_lossy();
            if !absolute_path(&directory) {
                errors.push(format!(
                    "{computer_id}.path: the directory must be absolute: {directory}"
                ));
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
        .chain(device.verify.iter())
        .chain(
            device
                .handoff
                .values()
                .flat_map(|handoff| handoff.steps.iter()),
        )
}

fn program_errors(operation: &Operation, where_: &str, errors: &mut Vec<String>) {
    let (Driver::Exec(program) | Driver::Command(program)) = &operation.driver else {
        return;
    };
    // A program name with no path is found in the search path.
    if let Some(name) = program.argv.first()
        && name.contains(['/', '\\'])
        && !absolute_path(name)
    {
        errors.push(format!(
            "{where_}: a program with a path must be absolute: {name}"
        ));
    }
    if let Some(cwd) = &program.cwd {
        let cwd = cwd.to_string_lossy();
        if !absolute_path(&cwd) {
            errors.push(format!("{where_}: cwd must be absolute: {cwd}"));
        }
    }
}

fn absolute_path(path: &str) -> bool {
    path.starts_with('/')
        || path.starts_with("\\\\")
        || matches!(path.as_bytes(), [drive, b':', b'/' | b'\\', ..] if drive.is_ascii_alphabetic())
}
