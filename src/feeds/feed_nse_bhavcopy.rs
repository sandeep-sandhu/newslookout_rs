// file: feeds/feed_nse_bhavcopy.rs
// Purpose:
//   Batch feed: download the NSE Capital Market (equity) UDiFF bhavcopy and load it directly
//   into the market-data SQLite DB (`market_series`/`nse_cm_bhavcopy`). This replaces the
//   bhavcopy-download path that previously lived in the `mod_in_nse` news retriever (roadmap
//   point 2g) — here there is no Document; the feed writes its own data.
//
//   NSE publishes the day's file only after market close, and not at all on
//   weekends/holidays, so we walk back over recent business days and use the first valid zip.

use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

use chrono::{NaiveDate, Utc};
use log::{info, warn};
use zip::ZipArchive;

use crate::cfg::get_market_data_db;
use crate::feeds::FeedOutcome;
use crate::network::http_get_binary;

pub const FEED_NAME: &str = "feed_nse_bhavcopy";
const NSE_LOOKBACK_BUSINESS_DAYS: usize = 5;

/// NSE equity bhavcopy URL (UDiFF zip) for a given date.
fn nse_bhavcopy_url_for(date: NaiveDate) -> String {
    let date_compact = date.format("%Y%m%d").to_string();
    format!(
        "https://nsearchives.nseindia.com/content/cm/BhavCopy_NSE_CM_0_0_0_{}_F_0000.csv.zip",
        date_compact
    )
}

/// Extract the first `.csv` member from a bhavcopy zip's bytes.
fn csv_from_zip(zip_bytes: &[u8]) -> Option<String> {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut archive = ZipArchive::new(cursor).ok()?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).ok()?;
        if entry.name().to_lowercase().ends_with(".csv") {
            let mut csv = String::new();
            if entry.read_to_string(&mut csv).is_ok() && !csv.is_empty() {
                return Some(csv);
            }
        }
    }
    None
}

/// Entry point invoked by the batch-feed runner.
pub fn run(app_config: Arc<config::Config>) -> FeedOutcome {
    let db_path = get_market_data_db(&app_config);
    info!("{}: market-data DB = {}", FEED_NAME, db_path);

    // NSE requires a desktop Chrome User-Agent for bhavcopy downloads.
    let client = match reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
        .timeout(Duration::from_secs(60))
        .gzip(true)
        .build()
    {
        Ok(c) => c,
        Err(e) => return FeedOutcome::fail(format!("could not build HTTP client: {}", e)),
    };

    let candidate_days = crate::utils::recent_business_days(Utc::now().date_naive(), NSE_LOOKBACK_BUSINESS_DAYS);

    for date in candidate_days {
        let url = nse_bhavcopy_url_for(date);
        let date_str = date.format("%Y-%m-%d").to_string();
        info!("{}: trying bhavcopy for {}: {}", FEED_NAME, date_str, url);

        let zip_bytes = http_get_binary(&url, &client);
        // Empty or HTML (starts with '<') means the file is not published for this date.
        if zip_bytes.is_empty() || zip_bytes.first() == Some(&b'<') {
            warn!("{}: no bhavcopy for {} (not published / non-trading day), trying earlier.", FEED_NAME, date_str);
            continue;
        }

        let csv = match csv_from_zip(zip_bytes.as_ref()) {
            Some(c) => c,
            None => { warn!("{}: no CSV inside zip for {}, trying earlier.", FEED_NAME, date_str); continue; }
        };

        return match crate::market_data::save_nse_csv_to_sqlite(&csv, &db_path) {
            Ok(rows) => {
                crate::metrics::record_db_writes(rows as u64);
                FeedOutcome::ok(rows as i64, format!("NSE bhavcopy {} loaded", date_str))
            }
            Err(e) => {
                crate::metrics::record_db_error();
                FeedOutcome::fail(format!("save NSE bhavcopy {}: {}", date_str, e))
            }
        };
    }

    FeedOutcome::fail(format!("no NSE bhavcopy found in last {} business days", NSE_LOOKBACK_BUSINESS_DAYS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nse_bhavcopy_url_format() {
        let date = NaiveDate::from_ymd_opt(2025, 6, 3).unwrap();
        assert_eq!(
            nse_bhavcopy_url_for(date),
            "https://nsearchives.nseindia.com/content/cm/BhavCopy_NSE_CM_0_0_0_20250603_F_0000.csv.zip"
        );
    }

    #[test]
    fn test_csv_from_zip_roundtrip() {
        // Build a tiny in-memory zip containing a .csv and a non-csv entry.
        use std::io::Write;
        let mut buf = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut buf);
            let mut zw = zip::ZipWriter::new(cursor);
            let opts: zip::write::FileOptions<()> = zip::write::FileOptions::default();
            zw.start_file("readme.txt", opts).unwrap();
            zw.write_all(b"ignore me").unwrap();
            zw.start_file("BhavCopy.csv", opts).unwrap();
            zw.write_all(b"TradDt,TckrSymb\n2025-06-03,RELIANCE\n").unwrap();
            zw.finish().unwrap();
        }
        let csv = csv_from_zip(&buf).expect("should find the csv member");
        assert!(csv.contains("TckrSymb"));
        assert!(csv.contains("RELIANCE"));
    }

    #[test]
    fn test_csv_from_zip_none_when_no_csv() {
        use std::io::Write;
        let mut buf = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut buf);
            let mut zw = zip::ZipWriter::new(cursor);
            let opts: zip::write::FileOptions<()> = zip::write::FileOptions::default();
            zw.start_file("data.txt", opts).unwrap();
            zw.write_all(b"no csv here").unwrap();
            zw.finish().unwrap();
        }
        assert!(csv_from_zip(&buf).is_none());
    }
}
