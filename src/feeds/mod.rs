// file: feeds/mod.rs
// Purpose:
//   The batch-feed subsystem (roadmap Stage 2 / point 2). Distinct from the news pipeline in
//   `plugins/`: batch feeds fetch *structured datasets* (NSE/BSE bhavcopy, and later RBI/CCIL
//   series, reference-data dumps) on a periodic, externally-scheduled cadence. Unlike news
//   retrievers, **no shared `Document` flows** out of a feed — each feed is independent and
//   writes its own data directly to the relevant store (e.g. the market-data SQLite DB).
//
//   Design highlights:
//   - Configured in the same TOML as news plugins, with `type = "batch_feed"` and a
//     per-feed `frequency_days`. One CLI entry (`newslookout_app batch <config>`) runs them.
//   - Feeds run in PARALLEL (one thread each), since they are independent.
//   - A feed that pulls many files for the SAME source (e.g. NSE multiple zips) keeps all of
//     that logic inside ONE feed function and fetches serially, to stay polite to the server.
//   - `batch_run_log` (store layer) records the last successful run per (source, dataset) so
//     re-running within `frequency_days` is skipped — safe for cron re-entry.
//   - All feeds may write to the same metadata DB; SQLite WAL + busy_timeout (store::open)
//     coordinates concurrent writers.

use std::sync::Arc;
use std::thread;

use log::{error, info, warn};

use crate::pipeline::{extract_plugin_params, PluginType};
use crate::store::batch_log::{self, RunStatus};

pub mod feed_nse_bhavcopy;
pub mod feed_bse_bhavcopy;

/// Outcome of a single feed run, used to populate `batch_run_log`.
pub struct FeedOutcome {
    pub status: RunStatus,
    pub rows: i64,
    pub message: String,
}

impl FeedOutcome {
    pub fn ok(rows: i64, message: impl Into<String>) -> Self {
        FeedOutcome { status: RunStatus::Success, rows, message: message.into() }
    }
    pub fn fail(message: impl Into<String>) -> Self {
        FeedOutcome { status: RunStatus::Failure, rows: 0, message: message.into() }
    }
}

/// A feed's worker entry point: independent, returns its outcome (no Document channel).
pub type FeedFn = fn(Arc<config::Config>) -> FeedOutcome;

/// A configured batch feed ready to run.
pub struct BatchFeed {
    pub name: String,
    pub enabled: bool,
    pub frequency_days: u32,
    pub run: FeedFn,
}

/// Dispatch table: feed config name → worker entry point. To add a feed, add one line.
fn registry() -> &'static [(&'static str, FeedFn)] {
    &[
        (feed_nse_bhavcopy::FEED_NAME, feed_nse_bhavcopy::run),
        (feed_bse_bhavcopy::FEED_NAME, feed_bse_bhavcopy::run),
    ]
}

/// Read `frequency_days` from a plugin's config map (default 1 = daily).
fn read_frequency_days(plugin_map: &config::Map<String, config::Value>) -> u32 {
    plugin_map
        .get("frequency_days")
        .and_then(|v| v.clone().into_int().ok())
        .map(|n| n.max(0) as u32)
        .unwrap_or(1)
}

/// Load enabled batch-feed definitions from the application config (entries with
/// `type = "batch_feed"`), resolving each name against the dispatch table.
pub fn load_batch_feeds(app_config: &config::Config) -> Vec<BatchFeed> {
    let reg = registry();
    let mut feeds = Vec::new();

    let plugins_configured = match app_config.get_array("plugins") {
        Ok(p) => p,
        Err(e) => {
            error!("feeds: no 'plugins' array in config: {}", e);
            return feeds;
        }
    };

    for plugin in plugins_configured {
        let plugin_map = match plugin.into_table() {
            Ok(m) => m,
            Err(e) => { error!("feeds: bad plugin entry in config: {}", e); continue; }
        };
        let (name, plugin_type, enabled, _priority) = extract_plugin_params(plugin_map.clone());
        if plugin_type != PluginType::BatchFeed {
            continue;
        }
        match reg.iter().find(|(n, _)| *n == name.as_str()) {
            Some((_, run)) => {
                feeds.push(BatchFeed {
                    name,
                    enabled,
                    frequency_days: read_frequency_days(&plugin_map),
                    run: *run,
                });
            }
            None => warn!("feeds: unknown batch_feed '{}' in config (no module registered)", name),
        }
    }
    feeds
}

/// Run all enabled batch feeds in parallel, honouring per-feed `frequency_days` via
/// `batch_run_log`, and recording each outcome. `db_path` is the metadata DB holding
/// `batch_run_log` (already migrated). Returns the number of feeds actually executed.
pub fn run_batch_feeds(app_config: Arc<config::Config>, db_path: &str) -> usize {
    let feeds = load_batch_feeds(&app_config);
    info!("feeds: {} batch feed(s) configured.", feeds.len());

    let now_ts = chrono::Utc::now().timestamp();
    let mut handles = Vec::new();

    for feed in feeds {
        if !feed.enabled {
            info!("feeds: '{}' disabled, skipping.", feed.name);
            continue;
        }

        // Frequency check uses (source=name, dataset=name) at this level; a feed that manages
        // multiple datasets records finer-grained dataset rows itself.
        let skip = match crate::store::open(db_path) {
            Ok(conn) => batch_log::should_skip(&conn, &feed.name, &feed.name, feed.frequency_days, now_ts),
            Err(e) => { error!("feeds: cannot open '{}' for run-log check: {}", db_path, e); false }
        };
        if skip {
            info!("feeds: '{}' ran within {} day(s); skipping (use frequency_days=0 to force).",
                feed.name, feed.frequency_days);
            continue;
        }

        let cfg = app_config.clone();
        let dbp = db_path.to_string();
        let handle = thread::Builder::new()
            .name(feed.name.clone())
            .spawn(move || {
                let name = feed.name.clone();
                info!("feeds: starting '{}'", name);
                if let Ok(conn) = crate::store::open(&dbp) {
                    batch_log::record_attempt(&conn, &name, &name, now_ts);
                }
                let outcome = (feed.run)(cfg);
                let done_ts = chrono::Utc::now().timestamp();
                if let Ok(conn) = crate::store::open(&dbp) {
                    batch_log::record_result(&conn, &name, &name, outcome.status, outcome.rows, &outcome.message, done_ts);
                }
                match outcome.status {
                    RunStatus::Success => info!("feeds: '{}' OK — {} ({} rows)", name, outcome.message, outcome.rows),
                    RunStatus::Failure => error!("feeds: '{}' FAILED — {}", name, outcome.message),
                }
            });
        match handle {
            Ok(h) => handles.push(h),
            Err(e) => error!("feeds: could not spawn thread for feed: {}", e),
        }
    }

    let executed = handles.len();
    for h in handles {
        if let Err(e) = h.join() {
            error!("feeds: a feed thread panicked: {:?}", e);
        }
    }
    info!("feeds: completed {} feed run(s).", executed);
    executed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with(toml: &str) -> config::Config {
        config::Config::builder()
            .add_source(config::File::from_str(toml, config::FileFormat::Toml))
            .build()
            .unwrap()
    }

    #[test]
    fn test_load_batch_feeds_filters_by_type_and_registry() {
        let toml = r#"
            plugins = [
              { enabled = true,  name = "feed_nse_bhavcopy", type = "batch_feed", priority = 1, frequency_days = 1 },
              { enabled = true,  name = "mod_en_in_rbi",     type = "retriever",  priority = 1 },
              { enabled = false, name = "feed_bse_bhavcopy", type = "batch_feed", priority = 1 },
              { enabled = true,  name = "feed_unknown_xyz",  type = "batch_feed", priority = 1 },
            ]
        "#;
        let cfg = cfg_with(toml);
        let feeds = load_batch_feeds(&cfg);
        // retriever excluded; unknown name dropped; nse + bse (disabled but registered) kept.
        let names: Vec<&str> = feeds.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"feed_nse_bhavcopy"));
        assert!(names.contains(&"feed_bse_bhavcopy"));
        assert!(!names.contains(&"mod_en_in_rbi"));
        assert!(!names.contains(&"feed_unknown_xyz"));
    }

    #[test]
    fn test_frequency_days_parsed_with_default() {
        let toml = r#"
            plugins = [
              { enabled = true, name = "feed_nse_bhavcopy", type = "batch_feed", priority = 1, frequency_days = 7 },
              { enabled = true, name = "feed_bse_bhavcopy", type = "batch_feed", priority = 1 },
            ]
        "#;
        let cfg = cfg_with(toml);
        let feeds = load_batch_feeds(&cfg);
        let nse = feeds.iter().find(|f| f.name == "feed_nse_bhavcopy").unwrap();
        let bse = feeds.iter().find(|f| f.name == "feed_bse_bhavcopy").unwrap();
        assert_eq!(nse.frequency_days, 7);
        assert_eq!(bse.frequency_days, 1, "default frequency should be 1 day");
    }
}
