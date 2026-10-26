//! Candidate targets of a local device (FLOW-3, `docs/config.md` "Runtime Resolution").

use crate::config::{Config, Device, Handoff};
use crate::ids::ComputerId;

/// One handoff route from a local device to a target computer.
#[derive(Debug, Clone, PartialEq)]
pub struct Route<'a> {
    pub target: &'a ComputerId,
    /// The device alias on the target computer.
    pub target_alias: &'a str,
    pub target_device: &'a Device,
    pub handoff: &'a Handoff,
}

/// Returns the candidate targets of a local device: the local `handoff` keys that
/// are also computers with a device of the same `id` (AC-1, AC-2). The result is in
/// the order of the target computer IDs.
pub fn candidates<'a>(config: &'a Config, local_device: &'a Device) -> Vec<Route<'a>> {
    local_device
        .handoff
        .iter()
        .filter_map(|(target, handoff)| {
            let (target_alias, target_device) =
                config.computer(target)?.device_by_id(&local_device.id)?;
            Some(Route {
                target,
                target_alias,
                target_device,
                handoff,
            })
        })
        .collect()
}
