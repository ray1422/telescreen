#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.
//! NET-9: the statistics of the pointer datagrams on the target (M14).

use std::time::{Duration, Instant};

use telescreen_core::ids::SessionId;
use telescreen_core::pointer::Normalized;
use telescreen_core::pointer_stats::{Percentiles, PointerStats};
use telescreen_core::wire::PointerDatagram;

const S: SessionId = SessionId(7);

fn datagram(seq: u64, sent_ms: u64) -> PointerDatagram {
    PointerDatagram {
        session: S,
        seq,
        sent_us: sent_ms * 1000,
        position: Normalized { x: 0.5, y: 0.5 },
    }
}

/// Records each `(seq, sent ms, arrival ms)`. The arrival is from a fixed start.
fn record(samples: &[(u64, u64, u64)]) -> PointerStats {
    let start = Instant::now();
    let mut stats = PointerStats::new(S);
    for &(seq, sent, arrived) in samples {
        stats.record(&datagram(seq, sent), start + Duration::from_millis(arrived));
    }
    stats
}

#[test]
fn net9_no_datagram_gives_empty_statistics() {
    let summary = PointerStats::new(S).summary();
    assert_eq!(summary.session, S);
    assert_eq!(summary.received, 0);
    assert_eq!(summary.lost, 0);
    assert_eq!(summary.arrival_interval, Percentiles::default());
    assert_eq!(summary.delay_variation.count, 0);
}

#[test]
fn net9_counts_used_late_and_lost_datagrams() {
    // 3 arrives after 4, so it is late. 6 and 7 do not arrive.
    let summary = record(&[
        (1, 0, 10),
        (2, 8, 18),
        (4, 24, 34),
        (3, 16, 35),
        (5, 32, 42),
        (8, 56, 66),
    ])
    .summary();
    assert_eq!(summary.received, 6);
    assert_eq!(summary.used, 5);
    assert_eq!(summary.late, 1);
    assert_eq!(summary.lost, 2, "6 and 7; the late 3 is not lost");
}

#[test]
fn net9_intervals_and_percentiles() {
    // The source sends each 8 ms. The arrivals are each 8 ms, but one is 20 ms
    // after the one before it.
    let mut samples = Vec::new();
    let mut arrived = 100;
    for seq in 1..=101u64 {
        let sent = (seq - 1) * 8;
        if seq > 1 {
            arrived += if seq == 50 { 20 } else { 8 };
        }
        samples.push((seq, sent, arrived));
    }
    let summary = record(&samples).summary();

    assert_eq!(summary.send_interval.count, 100);
    assert_eq!(summary.send_interval.p50_us, 8000);
    assert_eq!(summary.send_interval.max_us, 8000);

    assert_eq!(summary.arrival_interval.count, 100);
    assert_eq!(summary.arrival_interval.p50_us, 8000);
    assert_eq!(
        summary.arrival_interval.p99_us, 8000,
        "nearest rank: 99th of 100"
    );
    assert_eq!(summary.arrival_interval.max_us, 20_000);

    // From 50 on, each datagram arrives 12 ms later than the first ones.
    assert_eq!(summary.delay_variation.count, 101);
    assert_eq!(summary.delay_variation.p50_us, 12_000);
    assert_eq!(summary.delay_variation.p95_us, 12_000);
    assert_eq!(summary.delay_variation.max_us, 12_000);
}

#[test]
fn net9_the_smallest_delay_is_the_zero_of_the_delay_variation() {
    // The first datagram is slow; the second has the smallest delay.
    let summary = record(&[(1, 0, 15), (2, 8, 18), (3, 16, 28)]).summary();
    assert_eq!(summary.delay_variation.count, 3);
    assert_eq!(summary.delay_variation.p50_us, 2000);
    assert_eq!(summary.delay_variation.max_us, 5000);
}

#[test]
fn net9_a_pause_is_not_an_interval() {
    // The pointer stops for 500 ms after 2.
    let summary = record(&[(1, 0, 10), (2, 8, 18), (3, 508, 518), (4, 516, 526)]).summary();
    assert_eq!(summary.pauses, 1);
    assert_eq!(summary.send_interval.count, 2);
    assert_eq!(summary.send_interval.max_us, 8000);
    assert_eq!(summary.arrival_interval.count, 2);
    assert_eq!(summary.arrival_interval.max_us, 8000);
}

#[test]
fn net9_send_interval_needs_consecutive_sequence_numbers() {
    // 2 is lost: the send interval of 1 to 3 is not known. The arrival interval
    // of 1 to 3 is what the target sees.
    let summary = record(&[(1, 0, 10), (3, 16, 26), (4, 24, 34)]).summary();
    assert_eq!(summary.send_interval.count, 1);
    assert_eq!(summary.send_interval.max_us, 8000);
    assert_eq!(summary.arrival_interval.count, 2);
    assert_eq!(summary.arrival_interval.max_us, 16_000);
}

#[test]
fn net9_a_datagram_of_another_session_is_ignored() {
    let start = Instant::now();
    let mut stats = PointerStats::new(S);
    let mut other = datagram(1, 0);
    other.session = SessionId(8);
    stats.record(&other, start);
    assert_eq!(stats.summary().received, 0);
}

#[test]
fn net9_the_log_line_names_the_session_and_the_loss() {
    let line = record(&[(1, 0, 10), (3, 16, 26)]).summary().to_string();
    assert!(line.contains("session 7"), "{line}");
    assert!(line.contains("lost 1"), "{line}");
    assert!(line.contains("p95"), "{line}");
}
