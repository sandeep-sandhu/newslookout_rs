// file: feeds/feed_bse_bhavcopy.rs
// Purpose:
//   Batch feed: download the BSE equity UDiFF bhavcopy (a plain, uncompressed CSV) and load
//   it directly into the market-data SQLite DB. Replaces the bhavcopy path that lived in the
//   `mod_in_bse` news retriever (roadmap point 2g). No Document flows from this feed.
//
//   BSE serves the file only after market close and not on non-trading days, so we walk back
//   over recent business days and use the first body that looks like a real CSV.

use std::sync::Arc;
use std::time::Duration;

use chrono::{NaiveDate, Utc};
use log::{info, warn};

use crate::cfg::get_market_data_db;
use crate::feeds::FeedOutcome;
use crate::network::http_get_binary;

pub const FEED_NAME: &str = "feed_bse_bhavcopy";
const BASE_URL: &str = "https://www.bseindia.com/";
const LOOKBACK_BUSINESS_DAYS: usize = 5;
/// Destination table name for the generic CSV loader.
const BSE_TABLE: &str = "bse_cm_bhavcopy";

/// Build the BSE UDiFF equity bhavcopy CSV URL for a given date.
fn bhavcopy_url_for(date: NaiveDate) -> String {
    let date_compact = date.format("%Y%m%d").to_string();
    format!(
        "{}download/BhavCopy/Equity/BhavCopy_BSE_CM_0_0_0_{}_F_0000.CSV",
        BASE_URL, date_compact
    )
}

/// True if the bytes look like a real bhavcopy CSV rather than an HTML error / empty body.
fn looks_like_csv(bytes: &[u8]) -> bool {
    if bytes.is_empty() || bytes.first() == Some(&b'<') {
        return false;
    }
    let prefix = &bytes[..bytes.len().min(64)];
    let prefix_str = String::from_utf8_lossy(prefix);
    prefix_str.starts_with("TradDt") || prefix_str.contains(',')
}

/// Entry point invoked by the batch-feed runner.
pub fn run(app_config: Arc<config::Config>) -> FeedOutcome {
    let db_path = get_market_data_db(&app_config);
    info!("{}: market-data DB = {}", FEED_NAME, db_path);

    // BSE requires a desktop browser User-Agent to serve the bhavcopy file.
    let client = match reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
        .timeout(Duration::from_secs(60))
        .gzip(true)
        .build()
    {
        Ok(c) => c,
        Err(e) => return FeedOutcome::fail(format!("could not build HTTP client: {}", e)),
    };

    let candidate_days = crate::utils::recent_business_days(Utc::now().date_naive(), LOOKBACK_BUSINESS_DAYS);

    for date in candidate_days {
        let url = bhavcopy_url_for(date);
        let date_str = date.format("%Y-%m-%d").to_string();
        info!("{}: trying bhavcopy for {}: {}", FEED_NAME, date_str, url);

        let body = http_get_binary(&url, &client);
        if !looks_like_csv(body.as_ref()) {
            warn!("{}: no CSV for {} (not published / non-trading day), trying earlier.", FEED_NAME, date_str);
            continue;
        }

        let csv = String::from_utf8_lossy(body.as_ref()).to_string();
        return match crate::market_data::save_csv_to_sqlite(&csv, BSE_TABLE, &date_str, &db_path) {
            Ok(rows) => {
                crate::metrics::record_db_writes(rows as u64);
                FeedOutcome::ok(rows as i64, format!("BSE bhavcopy {} loaded", date_str))
            }
            Err(e) => {
                crate::metrics::record_db_error();
                FeedOutcome::fail(format!("save BSE bhavcopy {}: {}", date_str, e))
            }
        };
    }

    FeedOutcome::fail(format!("no BSE bhavcopy found in last {} business days", LOOKBACK_BUSINESS_DAYS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bhavcopy_url_format() {
        let date = NaiveDate::from_ymd_opt(2025, 6, 3).unwrap();
        assert_eq!(
            bhavcopy_url_for(date),
            "https://www.bseindia.com/download/BhavCopy/Equity/BhavCopy_BSE_CM_0_0_0_20250603_F_0000.CSV"
        );
    }

    #[test]
    fn test_looks_like_csv() {
        assert!(looks_like_csv(b"TradDt,TckrSymb\n2025-06-03,RELIANCE"));
        assert!(looks_like_csv(b"SYMBOL,OPEN,CLOSE\nX,1,2"));
        assert!(!looks_like_csv(b"<!DOCTYPE html><html>"));
        assert!(!looks_like_csv(b""));
    }
}
