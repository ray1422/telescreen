//! Device leases on the target (FLOW-5, AC-20, NET-6).
//!
//! The target is the only peer that gives a lease for its device. The source has a
//! session only after the target granted it, so the local state of one peer alone
//! does not decide the lease owner.

use std::collections::BTreeMap;

use crate::ids::{ComputerId, DeviceId, SessionId};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("device {device} is leased to session {session} of {holder}")]
pub struct Busy {
    pub device: DeviceId,
    pub session: SessionId,
    pub holder: ComputerId,
}

#[derive(Debug, Clone)]
pub struct LeaseTable {
    next: u64,
    leases: BTreeMap<DeviceId, (SessionId, ComputerId)>,
}

impl LeaseTable {
    /// `first_generation` must be larger than all generations of earlier runs, for
    /// example the start time in milliseconds since the Unix epoch.
    pub fn new(first_generation: u64) -> Self {
        Self {
            next: first_generation,
            leases: BTreeMap::new(),
        }
    }

    /// Gives the lease of a device to a new session. Only one session holds the
    /// lease of a device at one time.
    pub fn acquire(&mut self, device: &DeviceId, holder: &ComputerId) -> Result<SessionId, Busy> {
        if let Some((session, current)) = self.leases.get(device) {
            return Err(Busy {
                device: device.clone(),
                session: *session,
                holder: current.clone(),
            });
        }
        let session = SessionId(self.next);
        self.next += 1;
        self.leases
            .insert(device.clone(), (session, holder.clone()));
        Ok(session)
    }

    /// Releases the lease of a session. Returns false if the session has no lease.
    pub fn release(&mut self, session: SessionId) -> bool {
        let before = self.leases.len();
        self.leases.retain(|_, (s, _)| *s != session);
        self.leases.len() != before
    }

    pub fn holder(&self, device: &DeviceId) -> Option<(SessionId, &ComputerId)> {
        self.leases.get(device).map(|(s, c)| (*s, c))
    }
}
