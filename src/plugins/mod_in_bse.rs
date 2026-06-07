// file: mod_in_bse.rs
// Download BSE (Bombay Stock Exchange) equity bhavcopy and push it through the pipeline.
//
// As of the SEBI "Common Equity Bhavcopy" (UDiFF) rollout, BSE publishes the daily
// equity bhavcopy as a *plain, uncompressed CSV* (not a ZIP) at:
//   https://www.bseindia.com/download/BhavCopy/Equity/BhavCopy_BSE_CM_0_0_0_{YYYYMMDD}_F_0000.CSV
// The older  EQ{DD}{MM}{YY}_CSV.ZIP  endpoint was discontinued and now returns an HTML
// error page (the cause of the recurring "returned an HTML page instead of a zip" logs).
//
// The current day's file does not exist until after market close, and no file is
// published on weekends/holidays, so we walk back over recent business days and use the
// most recent one that returns CSV data.

use std::sync::Arc;
use std::sync::mpsc::Sender;

use chrono::{NaiveDate, Utc};
use log::{error, info, warn};

use crate::cfg::get_database_filename;
use crate::document::Document;
use crate::network::{http_get_binary, make_http_client, read_network_parameters};
use crate::utils::{get_urls_from_database, recent_business_days};

pub(crate) const PLUGIN_NAME: &str = "mod_in_bse";
const PUBLISHER_NAME: &str = "Bombay Stock Exchange";
const BASE_URL: &str = "https://www.bseindia.com/";
// BSE serves the bhavcopy only after market close; check this many recent business
// days (most recent first) to tolerate weekends, holidays, and early-in-day runs.
const LOOKBACK_BUSINESS_DAYS: usize = 5;

/// Build the BSE UDiFF equity bhavcopy CSV URL for a given date.
fn bhavcopy_url_for(date: NaiveDate) -> String {
    let date_compact = date.format("%Y%m%d").to_string();
    format!(
        "{}download/BhavCopy/Equity/BhavCopy_BSE_CM_0_0_0_{}_F_0000.CSV",
        BASE_URL, date_compact
    )
}

/// Returns true if the downloaded bytes look like a real bhavcopy CSV rather than an
/// HTML error page or an empty body.
fn looks_like_csv(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    // HTML error pages start with '<' (e.g. "<!DOCTYPE", "<html").
    if bytes.first() == Some(&b'<') {
        return false;
    }
    // The UDiFF bhavcopy header begins with "TradDt,". Accept any comma-delimited text as
    // a fallback in case the header layout changes.
    let prefix = &bytes[..bytes.len().min(64)];
    let prefix_str = String::from_utf8_lossy(prefix);
    prefix_str.starts_with("TradDt") || prefix_str.contains(',')
}

pub(crate) fn run_worker_thread(tx: Sender<Document>, app_config: Arc<config::Config>) {
    info!("{}: Starting plugin.", PLUGIN_NAME);

    let mut netw_params = read_network_parameters(&app_config);
    netw_params.referrer_url = Some(BASE_URL.to_string());
    let client = make_http_client(&netw_params);

    let database_filename = get_database_filename(&app_config);
    let already_retrieved = get_urls_from_database(&database_filename, PLUGIN_NAME);

    // BSE requires a desktop browser User-Agent to serve the bhavcopy file.
    let bhavcopy_client = reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .unwrap_or_else(|_| client.clone());

    let candidate_days = recent_business_days(Utc::now().date_naive(), LOOKBACK_BUSINESS_DAYS);

    for date in candidate_days {
        let bhavcopy_url = bhavcopy_url_for(date);
        let date_str = date.format("%Y-%m-%d").to_string();

        if already_retrieved.contains(&bhavcopy_url) {
            info!("{}: BSE bhavcopy for {} already retrieved, stopping.", PLUGIN_NAME, date_str);
            return;
        }

        info!("{}: Trying BSE bhavcopy for {}: {}", PLUGIN_NAME, date_str, bhavcopy_url);
        let body = http_get_binary(&bhavcopy_url, &bhavcopy_client);

        if !looks_like_csv(body.as_ref()) {
            warn!("{}: No bhavcopy CSV for {} (not yet published / non-trading day), trying earlier day.",
                PLUGIN_NAME, date_str);
            continue;
        }

        let csv_content = String::from_utf8_lossy(body.as_ref()).into_owned();
        let row_count = csv_content.lines().count().saturating_sub(1);
        info!("{}: Retrieved BSE bhavcopy for {} with {} data rows.", PLUGIN_NAME, date_str, row_count);

        let mut doc = Document::default();
        doc.module = PLUGIN_NAME.to_string();
        doc.plugin_name = PUBLISHER_NAME.to_string();
        doc.source_author = PUBLISHER_NAME.to_string();
        doc.section_name = "Equity Bhavcopy".to_string();
        doc.url = bhavcopy_url.clone();
        doc.title = format!("BSE Equity Bhavcopy {}", date_str);
        doc.publish_date = date_str.clone();
        doc.publish_date_ms = crate::utils::to_local_datetime(date).timestamp();
        doc.text = csv_content;

        match tx.send(doc) {
            Ok(_) => info!("{}: Sent BSE bhavcopy document for {} ({} rows).", PLUGIN_NAME, date_str, row_count),
            Err(e) => error!("{}: Channel send error: {}", PLUGIN_NAME, e),
        }
        info!("{}: Completed.", PLUGIN_NAME);
        return;
    }

    warn!("{}: No BSE bhavcopy found in the last {} business days.", PLUGIN_NAME, LOOKBACK_BUSINESS_DAYS);
    info!("{}: Completed with 0 documents.", PLUGIN_NAME);
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
    fn test_looks_like_csv_accepts_udiff_header() {
        let body = b"TradDt,BizDt,Sgmt,Src,FinInstrmTp\n2025-06-03,2025-06-03,CM,BSE,STK";
        assert!(looks_like_csv(body));
    }

    #[test]
    fn test_looks_like_csv_rejects_html() {
        assert!(!looks_like_csv(b"<!DOCTYPE html><html><body>error</body></html>"));
        assert!(!looks_like_csv(b"<html>"));
    }

    #[test]
    fn test_looks_like_csv_rejects_empty() {
        assert!(!looks_like_csv(b""));
    }
}
