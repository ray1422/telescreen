//! Pointer datagram statistics on the target (NET-9). All arrival times come
//! from the caller; the clocks of the source and target need not agree.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Instant;

use serde::Serialize;

use crate::ids::SessionId;
use crate::wire::PointerDatagram;

const MAX_SAMPLES: usize = 1 << 18;
const PAUSE_US: u64 = 100_000;

/// Nearest-rank percentiles of retained samples, in microseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Percentiles {
    pub count: u64,
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub max_us: u64,
}

impl Percentiles {
    fn from_samples(mut samples: Vec<u64>) -> Self {
        if samples.is_empty() {
            return Self::default();
        }
        samples.sort_unstable();
        let rank = |percent: usize| samples[(samples.len() * percent).div_ceil(100) - 1];
        Self {
            count: samples.len() as u64,
            p50_us: rank(50),
            p95_us: rank(95),
            p99_us: rank(99),
            max_us: samples[samples.len() - 1],
        }
    }
}

impl fmt::Display for Percentiles {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "p50 {:.1} ms, p95 {:.1} ms, p99 {:.1} ms, max {:.1} ms (n {})",
            self.p50_us as f64 / 1000.0,
            self.p95_us as f64 / 1000.0,
            self.p99_us as f64 / 1000.0,
            self.max_us as f64 / 1000.0,
            self.count,
        )
    }
}

/// Counts for the whole session and percentiles of at most `1 << 18` samples
/// per series. Delay variation is relative to the smallest retained offset.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PointerSummary {
    pub session: SessionId,
    pub received: u64,
    pub used: u64,
    pub late: u64,
    pub lost: u64,
    pub pauses: u64,
    pub send_interval: Percentiles,
    pub arrival_interval: Percentiles,
    pub delay_variation: Percentiles,
}

impl fmt::Display for PointerSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "pointer session {}: received {}, used {}, late {}, lost {}, pauses {}; send interval {}; arrival interval {}; delay variation {}",
            self.session,
            self.received,
            self.used,
            self.late,
            self.lost,
            self.pauses,
            self.send_interval,
            self.arrival_interval,
            self.delay_variation,
        )
    }
}

/// Statistics of one incoming session, independent of pointer injection.
#[derive(Debug)]
pub struct PointerStats {
    session: SessionId,
    received: u64,
    used: u64,
    late: u64,
    pauses: u64,
    first_seq: Option<u64>,
    first_arrival: Option<Instant>,
    newest_used: Option<(u64, u64, Instant)>,
    /// Send times indexed by sequence number so late arrivals can complete pairs.
    sent: BTreeMap<u64, u64>,
    send_intervals: Vec<u64>,
    arrival_intervals: Vec<u64>,
    offsets: Vec<i64>,
}

impl PointerStats {
    pub fn new(session: SessionId) -> Self {
        Self {
            session,
            received: 0,
            used: 0,
            late: 0,
            pauses: 0,
            first_seq: None,
            first_arrival: None,
            newest_used: None,
            sent: BTreeMap::new(),
            send_intervals: Vec::new(),
            arrival_intervals: Vec::new(),
            offsets: Vec::new(),
        }
    }

    pub fn session(&self) -> SessionId {
        self.session
    }

    /// Records a datagram of this session, including late and duplicate arrivals.
    /// Each series stops retaining samples at its limit; counts continue.
    pub fn record(&mut self, datagram: &PointerDatagram, arrived: Instant) {
        if datagram.session != self.session {
            return;
        }
        self.received = self.received.saturating_add(1);
        self.first_seq.get_or_insert(datagram.seq);
        let first_arrival = *self.first_arrival.get_or_insert(arrived);
        if self.offsets.len() < MAX_SAMPLES {
            let elapsed = arrived.saturating_duration_since(first_arrival).as_micros();
            // A Duration's microseconds fit in i128. Clamp the signed offset,
            // rather than its operands, to preserve large differences.
            let offset = (elapsed as i128 - i128::from(datagram.sent_us))
                .clamp(i128::from(i64::MIN), i128::from(i64::MAX));
            self.offsets.push(offset as i64);
        }

        match self.newest_used {
            Some((seq, _, _)) if datagram.seq <= seq => {
                self.late = self.late.saturating_add(1);
            }
            previous => {
                self.used = self.used.saturating_add(1);
                if let Some((_, sent_us, arrival)) = previous {
                    if datagram.sent_us.saturating_sub(sent_us) > PAUSE_US {
                        self.pauses = self.pauses.saturating_add(1);
                    } else {
                        let interval = arrived
                            .saturating_duration_since(arrival)
                            .as_micros()
                            .min(u128::from(u64::MAX))
                            as u64;
                        push_sample(&mut self.arrival_intervals, interval);
                    }
                }
                self.newest_used = Some((datagram.seq, datagram.sent_us, arrived));
            }
        }

        if self.send_intervals.len() < MAX_SAMPLES && !self.sent.contains_key(&datagram.seq) {
            if let Some(previous) = datagram.seq.checked_sub(1)
                && let Some(sent_us) = self.sent.get(&previous)
            {
                let interval = datagram.sent_us.saturating_sub(*sent_us);
                if interval <= PAUSE_US {
                    push_sample(&mut self.send_intervals, interval);
                }
            }
            if let Some(next) = datagram.seq.checked_add(1)
                && let Some(sent_us) = self.sent.get(&next)
            {
                let interval = sent_us.saturating_sub(datagram.sent_us);
                if interval <= PAUSE_US {
                    push_sample(&mut self.send_intervals, interval);
                }
            }
            if self.send_intervals.len() == MAX_SAMPLES {
                self.sent.clear();
            } else {
                self.sent.insert(datagram.seq, datagram.sent_us);
            }
        }
    }

    pub fn summary(&self) -> PointerSummary {
        let lost = match (self.first_seq, self.newest_used) {
            (Some(first), Some((newest, _, _))) => u128::from(newest)
                .saturating_sub(u128::from(first))
                .saturating_add(1)
                .saturating_sub(u128::from(self.received))
                .min(u128::from(u64::MAX))
                as u64,
            _ => 0,
        };
        let smallest = self.offsets.iter().min().copied().unwrap_or_default();
        PointerSummary {
            session: self.session,
            received: self.received,
            used: self.used,
            late: self.late,
            lost,
            pauses: self.pauses,
            send_interval: Percentiles::from_samples(self.send_intervals.clone()),
            arrival_interval: Percentiles::from_samples(self.arrival_intervals.clone()),
            delay_variation: Percentiles::from_samples(
                self.offsets
                    .iter()
                    .map(|offset| offset.abs_diff(smallest))
                    .collect(),
            ),
        }
    }
}

fn push_sample(samples: &mut Vec<u64>, sample: u64) {
    if samples.len() < MAX_SAMPLES {
        samples.push(sample);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pointer::Normalized;
    use std::time::Duration;

    const SESSION: SessionId = SessionId(7);

    fn datagram(seq: u64, sent_us: u64) -> PointerDatagram {
        PointerDatagram {
            session: SESSION,
            seq,
            sent_us,
            position: Normalized { x: 0.5, y: 0.5 },
        }
    }

    #[test]
    fn late_arrivals_complete_both_neighbors_without_duplicate_intervals() {
        let now = Instant::now();
        let mut stats = PointerStats::new(SESSION);
        for (seq, sent_us) in [(1, 0), (3, 16_000), (2, 8_000), (2, 8_000)] {
            stats.record(&datagram(seq, sent_us), now);
        }
        let summary = stats.summary();
        assert_eq!(stats.session(), SESSION);
        assert_eq!(summary.received, 4);
        assert_eq!(summary.used, 2);
        assert_eq!(summary.late, 2);
        assert_eq!(summary.lost, 0);
        assert_eq!(summary.send_interval.count, 2);
        assert_eq!(summary.send_interval.p50_us, 8_000);
        assert_eq!(summary.send_interval.max_us, 8_000);
        assert_eq!(summary.arrival_interval.count, 1);
        assert_eq!(summary.delay_variation.count, 4);
    }

    #[test]
    fn pause_threshold_is_strict_and_late_pairs_do_not_count_pauses_again() {
        let now = Instant::now();
        let mut stats = PointerStats::new(SESSION);
        for (seq, sent_us) in [(1, 0), (2, 100_000), (4, 300_002), (3, 200_001)] {
            stats.record(&datagram(seq, sent_us), now);
        }
        let summary = stats.summary();
        assert_eq!(summary.pauses, 1);
        assert_eq!(summary.send_interval.count, 1);
        assert_eq!(summary.send_interval.max_us, 100_000);
        assert_eq!(summary.arrival_interval.count, 1);
    }

    #[test]
    fn full_series_stop_sampling_but_counts_continue() {
        let now = Instant::now();
        let mut stats = PointerStats::new(SESSION);
        for seq in 0..=(MAX_SAMPLES as u64 + 2) {
            stats.record(&datagram(seq, seq), now + Duration::from_micros(seq));
        }
        stats.record(&datagram(MAX_SAMPLES as u64 + 3, 1_000_000), now);
        stats.record(&datagram(0, 0), now);
        let summary = stats.summary();
        assert_eq!(summary.received, MAX_SAMPLES as u64 + 5);
        assert_eq!(summary.used, MAX_SAMPLES as u64 + 4);
        assert_eq!(summary.late, 1);
        assert_eq!(summary.pauses, 1);
        assert_eq!(summary.send_interval.count, MAX_SAMPLES as u64);
        assert_eq!(summary.arrival_interval.count, MAX_SAMPLES as u64);
        assert_eq!(summary.delay_variation.count, MAX_SAMPLES as u64);
        assert_eq!(summary.send_interval.max_us, 1);
        assert_eq!(summary.arrival_interval.max_us, 1);
        assert_eq!(summary.delay_variation.max_us, 0);
    }

    #[test]
    fn extreme_sequence_numbers_and_send_times_saturate_without_overflow() {
        let now = Instant::now();
        let mut stats = PointerStats::new(SESSION);
        stats.record(&datagram(0, 0), now);
        stats.record(&datagram(u64::MAX, u64::MAX), now);
        let summary = stats.summary();
        assert_eq!(summary.lost, u64::MAX - 1);
        assert_eq!(summary.pauses, 1);
        assert_eq!(summary.delay_variation.max_us, 1 << 63);
    }

    #[test]
    fn an_earlier_arrival_instant_saturates_the_interval() {
        let now = Instant::now();
        let mut stats = PointerStats::new(SESSION);
        stats.record(&datagram(1, 0), now + Duration::from_micros(10));
        stats.record(&datagram(2, 1), now);
        let summary = stats.summary();
        assert_eq!(summary.arrival_interval.count, 1);
        assert_eq!(summary.arrival_interval.max_us, 0);
    }
}
