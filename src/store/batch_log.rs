// file: store/batch_log.rs
// Purpose:
//   Helpers over the `batch_run_log` table (created by migration 0001). The batch-feed CLI
//   (roadmap Stage 2) consults this before each run to avoid re-extracting the same
//   (source, dataset) within its configured frequency window, and records the outcome of
//   every attempt. Externally scheduled (cron) runs are therefore safely re-entrant.

use log::error;
use rusqlite::Connection;

/// Outcome of a batch feed run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Success,
    Failure,
}

impl RunStatus {
    fn as_str(&self) -> &'static str {
        match self {
            RunStatus::Success => "success",
            RunStatus::Failure => "failure",
        }
    }
}

/// Returns true if `(source, dataset)` had a successful run within the last
/// `frequency_days` days and should therefore be skipped on this invocation.
/// `now_ts` is seconds since epoch (passed in for testability).
pub fn should_skip(
    conn: &Connection,
    source: &str,
    dataset: &str,
    frequency_days: u32,
    now_ts: i64,
) -> bool {
    if frequency_days == 0 {
        return false; // always run
    }
    let last_success: Option<i64> = conn
        .query_row(
            "SELECT last_success_ts FROM batch_run_log WHERE source=?1 AND dataset=?2",
            rusqlite::params![source, dataset],
            |r| r.get(0),
        )
        .ok()
        .flatten();

    match last_success {
        Some(ts) => {
            let window_secs = frequency_days as i64 * 86_400;
            now_ts - ts < window_secs
        }
        None => false,
    }
}

/// Record the start of an attempt (sets `last_attempt_ts`, status='running'), upserting the
/// row if it does not yet exist.
pub fn record_attempt(conn: &Connection, source: &str, dataset: &str, now_ts: i64) {
    let res = conn.execute(
        "INSERT INTO batch_run_log (source, dataset, last_attempt_ts, status)
         VALUES (?1, ?2, ?3, 'running')
         ON CONFLICT(source, dataset) DO UPDATE SET last_attempt_ts=?3, status='running'",
        rusqlite::params![source, dataset, now_ts],
    );
    if let Err(e) = res {
        error!("batch_log: record_attempt({}, {}): {}", source, dataset, e);
    }
}

/// Record the outcome of an attempt. On success, also updates `last_success_ts`.
pub fn record_result(
    conn: &Connection,
    source: &str,
    dataset: &str,
    status: RunStatus,
    rows_ingested: i64,
    message: &str,
    now_ts: i64,
) {
    let sql = match status {
        RunStatus::Success => {
            "INSERT INTO batch_run_log
                (source, dataset, last_attempt_ts, last_success_ts, status, rows_ingested, message)
             VALUES (?1, ?2, ?3, ?3, ?4, ?5, ?6)
             ON CONFLICT(source, dataset) DO UPDATE SET
                last_attempt_ts=?3, last_success_ts=?3, status=?4, rows_ingested=?5, message=?6"
        }
        RunStatus::Failure => {
            "INSERT INTO batch_run_log
                (source, dataset, last_attempt_ts, status, rows_ingested, message)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(source, dataset) DO UPDATE SET
                last_attempt_ts=?3, status=?4, rows_ingested=?5, message=?6"
        }
    };
    let res = conn.execute(
        sql,
        rusqlite::params![source, dataset, now_ts, status.as_str(), rows_ingested, message],
    );
    if let Err(e) = res {
        error!("batch_log: record_result({}, {}): {}", source, dataset, e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        store::migrate(&c).unwrap();
        c
    }

    const DAY: i64 = 86_400;

    #[test]
    fn test_skip_false_when_never_run() {
        let c = db();
        assert!(!should_skip(&c, "NSE", "cm_bhavcopy", 1, 1_000_000));
    }

    #[test]
    fn test_skip_true_within_window() {
        let c = db();
        let t0 = 10 * DAY;
        record_result(&c, "NSE", "cm_bhavcopy", RunStatus::Success, 1500, "ok", t0);
        // 12 hours later, with a 1-day frequency → skip
        assert!(should_skip(&c, "NSE", "cm_bhavcopy", 1, t0 + DAY / 2));
    }

    #[test]
    fn test_skip_false_after_window() {
        let c = db();
        let t0 = 10 * DAY;
        record_result(&c, "NSE", "cm_bhavcopy", RunStatus::Success, 1500, "ok", t0);
        // 2 days later, with a 1-day frequency → run again
        assert!(!should_skip(&c, "NSE", "cm_bhavcopy", 1, t0 + 2 * DAY));
    }

    #[test]
    fn test_frequency_zero_always_runs() {
        let c = db();
        let t0 = 10 * DAY;
        record_result(&c, "NSE", "cm_bhavcopy", RunStatus::Success, 1, "ok", t0);
        assert!(!should_skip(&c, "NSE", "cm_bhavcopy", 0, t0 + 1));
    }

    #[test]
    fn test_failure_does_not_set_success_ts() {
        let c = db();
        let t0 = 10 * DAY;
        record_attempt(&c, "RBI", "wss", t0);
        record_result(&c, "RBI", "wss", RunStatus::Failure, 0, "http 500", t0 + 5);
        // a failed run must not suppress the next attempt
        assert!(!should_skip(&c, "RBI", "wss", 7, t0 + 6));
        let status: String = c
            .query_row(
                "SELECT status FROM batch_run_log WHERE source='RBI' AND dataset='wss'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "failure");
    }
}
