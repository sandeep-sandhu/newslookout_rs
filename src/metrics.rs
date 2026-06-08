// file: metrics.rs
// Purpose:
//   Process-wide telemetry counters (roadmap point 10). Lightweight atomic counters that any
//   module can increment without locking; the web API / dashboard (Stage 3) reads a snapshot
//   to show HTTP health, retry/timeout rates and DB write activity. Kept separate from the
//   per-run PipelineStatus (which tracks plugin/doc progress) — these are cumulative process
//   counters.

use std::sync::atomic::{AtomicU64, Ordering};
use serde::Serialize;

/// Global counters. `Relaxed` ordering is sufficient: these are independent statistics, not
/// synchronisation flags.
pub struct Metrics {
    pub http_requests: AtomicU64,
    pub http_2xx: AtomicU64,
    pub http_403: AtomicU64,
    pub http_404: AtomicU64,
    pub http_4xx_other: AtomicU64,
    pub http_5xx: AtomicU64,
    pub http_timeouts: AtomicU64,
    pub http_retries: AtomicU64,
    /// Transport-level failures (connection refused/DNS/etc.) where no status was received.
    pub http_transport_errors: AtomicU64,
    pub db_writes: AtomicU64,
    pub db_errors: AtomicU64,
}

impl Metrics {
    const fn new() -> Self {
        Metrics {
            http_requests: AtomicU64::new(0),
            http_2xx: AtomicU64::new(0),
            http_403: AtomicU64::new(0),
            http_404: AtomicU64::new(0),
            http_4xx_other: AtomicU64::new(0),
            http_5xx: AtomicU64::new(0),
            http_timeouts: AtomicU64::new(0),
            http_retries: AtomicU64::new(0),
            http_transport_errors: AtomicU64::new(0),
            db_writes: AtomicU64::new(0),
            db_errors: AtomicU64::new(0),
        }
    }
}

/// The single global metrics instance.
pub static METRICS: Metrics = Metrics::new();

/// Record one HTTP response by status code.
pub fn record_http_status(code: u16) {
    METRICS.http_requests.fetch_add(1, Ordering::Relaxed);
    match code {
        200..=299 => { METRICS.http_2xx.fetch_add(1, Ordering::Relaxed); }
        403 => { METRICS.http_403.fetch_add(1, Ordering::Relaxed); }
        404 => { METRICS.http_404.fetch_add(1, Ordering::Relaxed); }
        408 => { METRICS.http_timeouts.fetch_add(1, Ordering::Relaxed); }
        400..=499 => { METRICS.http_4xx_other.fetch_add(1, Ordering::Relaxed); }
        500..=599 => { METRICS.http_5xx.fetch_add(1, Ordering::Relaxed); }
        _ => {}
    }
}

/// Record a retry attempt.
pub fn record_http_retry() {
    METRICS.http_retries.fetch_add(1, Ordering::Relaxed);
}

/// Record a transport-level failure (no HTTP status received).
pub fn record_http_transport_error() {
    METRICS.http_transport_errors.fetch_add(1, Ordering::Relaxed);
}

/// Record successful DB row writes.
pub fn record_db_writes(n: u64) {
    METRICS.db_writes.fetch_add(n, Ordering::Relaxed);
}

/// Record a DB write error.
pub fn record_db_error() {
    METRICS.db_errors.fetch_add(1, Ordering::Relaxed);
}

/// A plain, serializable copy of the counters for the API/dashboard.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct MetricsSnapshot {
    pub http_requests: u64,
    pub http_2xx: u64,
    pub http_403: u64,
    pub http_404: u64,
    pub http_4xx_other: u64,
    pub http_5xx: u64,
    pub http_timeouts: u64,
    pub http_retries: u64,
    pub http_transport_errors: u64,
    pub db_writes: u64,
    pub db_errors: u64,
}

/// Take a consistent-enough snapshot of all counters.
pub fn snapshot() -> MetricsSnapshot {
    MetricsSnapshot {
        http_requests: METRICS.http_requests.load(Ordering::Relaxed),
        http_2xx: METRICS.http_2xx.load(Ordering::Relaxed),
        http_403: METRICS.http_403.load(Ordering::Relaxed),
        http_404: METRICS.http_404.load(Ordering::Relaxed),
        http_4xx_other: METRICS.http_4xx_other.load(Ordering::Relaxed),
        http_5xx: METRICS.http_5xx.load(Ordering::Relaxed),
        http_timeouts: METRICS.http_timeouts.load(Ordering::Relaxed),
        http_retries: METRICS.http_retries.load(Ordering::Relaxed),
        http_transport_errors: METRICS.http_transport_errors.load(Ordering::Relaxed),
        db_writes: METRICS.db_writes.load(Ordering::Relaxed),
        db_errors: METRICS.db_errors.load(Ordering::Relaxed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // NB: counters are global; this test only checks deltas, not absolute values, so it is
    // robust to other tests incrementing them.
    #[test]
    fn test_status_classification_deltas() {
        let before = snapshot();
        record_http_status(200);
        record_http_status(403);
        record_http_status(404);
        record_http_status(503);
        record_http_status(401);
        let after = snapshot();
        assert_eq!(after.http_2xx - before.http_2xx, 1);
        assert_eq!(after.http_403 - before.http_403, 1);
        assert_eq!(after.http_404 - before.http_404, 1);
        assert_eq!(after.http_5xx - before.http_5xx, 1);
        assert_eq!(after.http_4xx_other - before.http_4xx_other, 1);
        assert_eq!(after.http_requests - before.http_requests, 5);
    }
}
