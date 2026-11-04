#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.
//! NET-2: the target handles only the waiting pointer datagrams, not a queue of
//! wake-ups.

use std::time::Instant;

use telescreen_app::pointer_mailbox::{PointerMailbox, ReceivedPointer};
use telescreen_core::ids::{ComputerId, SessionId};
use telescreen_core::pointer::Normalized;
use telescreen_core::wire::PointerDatagram;

fn received(seq: u64) -> ReceivedPointer {
    ReceivedPointer {
        peer: ComputerId::from("mac"),
        datagram: PointerDatagram {
            session: SessionId(1),
            seq,
            sent_us: seq * 8000,
            position: Normalized { x: 0.5, y: 0.5 },
        },
        received: Instant::now(),
    }
}

#[test]
fn net2_only_the_first_waiting_datagram_wakes_the_main_loop() {
    let mailbox = PointerMailbox::new();
    assert!(mailbox.put(received(1)), "empty: wake the main loop");
    assert!(!mailbox.put(received(2)), "a wake-up is already pending");
    assert!(!mailbox.put(received(3)));

    let taken = mailbox.take();
    let seqs: Vec<u64> = taken.iter().map(|p| p.datagram.seq).collect();
    assert_eq!(seqs, [1, 2, 3], "in the sequence of arrival");
    assert!(mailbox.take().is_empty());
    assert!(mailbox.put(received(4)), "empty again: wake the main loop");
}

#[test]
fn net2_a_full_mailbox_keeps_the_newest_datagrams() {
    let mailbox = PointerMailbox::new();
    let limit = PointerMailbox::LIMIT as u64;
    for seq in 1..=limit + 10 {
        mailbox.put(received(seq));
    }
    let taken = mailbox.take();
    assert_eq!(taken.len() as u64, limit);
    assert_eq!(taken.first().unwrap().datagram.seq, 11);
    assert_eq!(taken.last().unwrap().datagram.seq, limit + 10);
}
