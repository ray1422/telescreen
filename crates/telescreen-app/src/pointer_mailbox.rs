//! Waiting pointer datagrams and a shared wake-up for the main loop (NET-2).

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use telescreen_core::ids::ComputerId;
use telescreen_core::wire::PointerDatagram;

/// A pointer datagram with its peer and network arrival time.
#[derive(Debug, Clone)]
pub struct ReceivedPointer {
    pub peer: ComputerId,
    pub datagram: PointerDatagram,
    pub received: Instant,
}

/// A bounded queue of pointer datagrams waiting for the main loop.
#[derive(Debug, Default)]
pub struct PointerMailbox {
    pointers: Mutex<VecDeque<ReceivedPointer>>,
}

impl PointerMailbox {
    pub const LIMIT: usize = 4096;

    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a datagram, dropping the oldest if full. Returns true only when
    /// the mailbox was empty, so the caller can wake the main loop.
    pub fn put(&self, pointer: ReceivedPointer) -> bool {
        let mut pointers = self.pointers.lock().unwrap_or_else(PoisonError::into_inner);
        let was_empty = pointers.is_empty();
        pointers.push_back(pointer);
        if pointers.len() > Self::LIMIT {
            pointers.pop_front();
        }
        was_empty
    }

    /// Removes all waiting datagrams, oldest first.
    pub fn take(&self) -> Vec<ReceivedPointer> {
        self.pointers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect()
    }
}
