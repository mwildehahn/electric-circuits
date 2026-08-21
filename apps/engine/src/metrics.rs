//! Lightweight, lock-free engine telemetry: counters + log-bucketed latency histograms, exposed
//! via `GET /metrics`. Used by the benchmark harness to attribute bottlenecks (per-envelope
//! fan-out, family-step, and shape-append latencies) under sustained load.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

const NB: usize = 40; // buckets cover [2^0, 2^40) microseconds (~12 days) — plenty of headroom.

/// A lock-free latency histogram with power-of-two buckets (bucket `i` = `[2^(i-1), 2^i)` µs).
/// Percentiles are reported as the bucket's upper bound — coarse but allocation-free and contention
/// free, which is what we want on the hot path.
pub struct Hist {
    buckets: [AtomicU64; NB],
    count: AtomicU64,
    sum: AtomicU64,
    max: AtomicU64,
}

impl Hist {
    const fn new() -> Self {
        Hist {
            buckets: [const { AtomicU64::new(0) }; NB],
            count: AtomicU64::new(0),
            sum: AtomicU64::new(0),
            max: AtomicU64::new(0),
        }
    }

    pub fn record(&self, us: u64) {
        self.buckets[bucket(us)].fetch_add(1, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
        self.sum.fetch_add(us, Ordering::Relaxed);
        self.max.fetch_max(us, Ordering::Relaxed);
    }

    fn quantile(&self, q: f64) -> u64 {
        let count = self.count.load(Ordering::Relaxed);
        if count == 0 {
            return 0;
        }
        let target = ((q * count as f64).ceil() as u64).max(1);
        let mut cum = 0u64;
        for (i, b) in self.buckets.iter().enumerate() {
            cum += b.load(Ordering::Relaxed);
            if cum >= target {
                return 1u64 << i; // upper bound of bucket i
            }
        }
        1u64 << (NB - 1)
    }

    fn snapshot(&self) -> serde_json::Value {
        let count = self.count.load(Ordering::Relaxed);
        let sum = self.sum.load(Ordering::Relaxed);
        serde_json::json!({
            "count": count,
            "mean_us": if count > 0 { sum / count } else { 0 },
            "p50_us": self.quantile(0.50),
            "p99_us": self.quantile(0.99),
            "p999_us": self.quantile(0.999),
            "max_us": self.max.load(Ordering::Relaxed),
        })
    }

    fn reset(&self) {
        for b in &self.buckets {
            b.store(0, Ordering::Relaxed);
        }
        self.count.store(0, Ordering::Relaxed);
        self.sum.store(0, Ordering::Relaxed);
        self.max.store(0, Ordering::Relaxed);
    }
}

fn bucket(us: u64) -> usize {
    if us == 0 {
        return 0;
    }
    ((64 - us.leading_zeros()) as usize).min(NB - 1)
}

/// A scoped timer: records its elapsed microseconds into `hist` on drop.
pub struct Timer<'a> {
    hist: &'a Hist,
    start: std::time::Instant,
}
impl<'a> Timer<'a> {
    pub fn new(hist: &'a Hist) -> Self {
        Timer { hist, start: std::time::Instant::now() }
    }
}
impl Drop for Timer<'_> {
    fn drop(&mut self) {
        self.hist.record(self.start.elapsed().as_micros() as u64);
    }
}

pub struct Metrics {
    pub envelopes: AtomicU64,    // table change events processed
    pub shape_appends: AtomicU64, // appends to shape streams
    pub family_steps: AtomicU64,  // family circuit transactions (write path)
    pub shapes_dormanted: AtomicU64,   // retention: active -> dormant transitions
    pub shapes_reactivated: AtomicU64, // retention: dormant -> active (table-stream replay)
    pub shapes_evicted: AtomicU64,     // retention: dormant shapes evicted (stream deleted)
    pub retention_pressure: AtomicU64, // retention: sweeps where a cap/budget was exceeded with nothing dormant to evict
    pub schema_drift: AtomicU64,       // ADR-0005: tables whose dependents were retired (drift / TRUNCATE / identity regression / drop)
    pub schema_unresolved: AtomicU64,  // ADR-0005: drifts that could not be resolved (table parked, retrying)
    pub epoch_breaks: AtomicU64,       // ADR-0004: slots the engine could no longer vouch for
    pub epoch_resets: AtomicU64,       // ADR-0004: new epochs bound (every shape retired, fresh slot)
    pub changes_rotations: AtomicU64,  // ADR-0006: change-log segments closed and succeeded
    pub changes_segments_deleted: AtomicU64, // ADR-0006: rotated-out segments retired (nothing could resume inside)
    pub txn_spills: AtomicU64,         // ADR-0003: transactions whose buffer outgrew the memory cap and went to disk
    /// COUNTER (cumulative), not a gauge: bytes ever written to transaction spill files. A gauge
    /// would be meaningless — a spill file exists only between one `Begin` and its `Commit`, so any
    /// sample would almost always read zero. `reset()` clears it with the other counters.
    pub txn_spill_bytes: AtomicU64,
    /// ADR-0003: chunk appends made by commits too large for a single append. A commit that fits in
    /// one append contributes 0, so this counts exactly the chunked commits' POSTs.
    pub txn_chunked_appends: AtomicU64,
    /// GAUGE, not a counter: how many change-log segments exist right now (republished by every
    /// retention sweep). `reset()` leaves it alone — a gauge describes the world, not the window.
    pub changes_segments_retained: AtomicU64,
    pub process_envelope: Hist,   // end-to-end fan-out latency per table envelope
    pub family_step: Hist,        // one family circuit transaction
    pub append: Hist,             // one shape-stream append (durable-streams round-trip)
}

static METRICS: OnceLock<Metrics> = OnceLock::new();

pub fn metrics() -> &'static Metrics {
    METRICS.get_or_init(|| Metrics {
        envelopes: AtomicU64::new(0),
        shape_appends: AtomicU64::new(0),
        family_steps: AtomicU64::new(0),
        shapes_dormanted: AtomicU64::new(0),
        shapes_reactivated: AtomicU64::new(0),
        shapes_evicted: AtomicU64::new(0),
        retention_pressure: AtomicU64::new(0),
        schema_drift: AtomicU64::new(0),
        schema_unresolved: AtomicU64::new(0),
        epoch_breaks: AtomicU64::new(0),
        epoch_resets: AtomicU64::new(0),
        changes_rotations: AtomicU64::new(0),
        changes_segments_deleted: AtomicU64::new(0),
        changes_segments_retained: AtomicU64::new(0),
        txn_spills: AtomicU64::new(0),
        txn_spill_bytes: AtomicU64::new(0),
        txn_chunked_appends: AtomicU64::new(0),
        process_envelope: Hist::new(),
        family_step: Hist::new(),
        append: Hist::new(),
    })
}

impl Metrics {
    pub fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "counters": {
                "envelopes_processed": self.envelopes.load(Ordering::Relaxed),
                "shape_appends": self.shape_appends.load(Ordering::Relaxed),
                "family_steps": self.family_steps.load(Ordering::Relaxed),
                "shapes_dormanted": self.shapes_dormanted.load(Ordering::Relaxed),
                "shapes_reactivated": self.shapes_reactivated.load(Ordering::Relaxed),
                "shapes_evicted": self.shapes_evicted.load(Ordering::Relaxed),
                "retention_pressure": self.retention_pressure.load(Ordering::Relaxed),
                "schema_drift_total": self.schema_drift.load(Ordering::Relaxed),
                "schema_unresolved_total": self.schema_unresolved.load(Ordering::Relaxed),
                "epoch_breaks_total": self.epoch_breaks.load(Ordering::Relaxed),
                "epoch_resets_total": self.epoch_resets.load(Ordering::Relaxed),
                "changes_rotations_total": self.changes_rotations.load(Ordering::Relaxed),
                "changes_segments_deleted_total": self.changes_segments_deleted.load(Ordering::Relaxed),
                "txn_spills_total": self.txn_spills.load(Ordering::Relaxed),
                "txn_spill_bytes": self.txn_spill_bytes.load(Ordering::Relaxed),
                "txn_chunked_appends_total": self.txn_chunked_appends.load(Ordering::Relaxed),
            },
            "gauges": {
                "changes_segments_retained": self.changes_segments_retained.load(Ordering::Relaxed),
            },
            "process_envelope_us": self.process_envelope.snapshot(),
            "family_step_us": self.family_step.snapshot(),
            "append_us": self.append.snapshot(),
        })
    }

    /// Zero all counters and histograms — the benchmark calls this after shape registration so the
    /// load-phase percentiles aren't skewed by setup.
    pub fn reset(&self) {
        self.envelopes.store(0, Ordering::Relaxed);
        self.shape_appends.store(0, Ordering::Relaxed);
        self.family_steps.store(0, Ordering::Relaxed);
        self.shapes_dormanted.store(0, Ordering::Relaxed);
        self.shapes_reactivated.store(0, Ordering::Relaxed);
        self.shapes_evicted.store(0, Ordering::Relaxed);
        self.retention_pressure.store(0, Ordering::Relaxed);
        self.schema_drift.store(0, Ordering::Relaxed);
        self.schema_unresolved.store(0, Ordering::Relaxed);
        self.epoch_breaks.store(0, Ordering::Relaxed);
        self.epoch_resets.store(0, Ordering::Relaxed);
        self.changes_rotations.store(0, Ordering::Relaxed);
        self.changes_segments_deleted.store(0, Ordering::Relaxed);
        self.txn_spills.store(0, Ordering::Relaxed);
        self.txn_spill_bytes.store(0, Ordering::Relaxed);
        self.txn_chunked_appends.store(0, Ordering::Relaxed);
        // `changes_segments_retained` is deliberately NOT reset: it is a gauge of what exists.
        self.process_envelope.reset();
        self.family_step.reset();
        self.append.reset();
    }
}
