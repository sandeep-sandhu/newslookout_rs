// file: mod_in_nse.rs
// Retrieve regulatory circulars and announcements from NSE India

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use chrono::{NaiveDate, Utc};
use log::{error, info, warn};
use serde_json::Value;
use zip::ZipArchive;

use crate::{document, get_plugin_cfg};
use crate::cfg::{get_data_folder, get_database_filename};
use crate::document::Document;
use crate::network::{http_get, http_get_binary, make_http_client, read_network_parameters};
use crate::utils::{check_and_fix_url, clean_text, get_urls_from_database, recent_business_days, to_local_datetime};

pub(crate) const PLUGIN_NAME: &str = "mod_in_nse";
const PUBLISHER_NAME: &str = "National Stock Exchange of India";
const BASE_URL: &str = "https://www.nseindia.com/";
// Walk back this many business days to find the most recent published bhavcopy.
const NSE_LOOKBACK_BUSINESS_DAYS: usize = 5;

pub(crate) fn run_worker_thread(tx: Sender<Document>, app_config: Arc<config::Config>) {

    info!("{}: Starting plugin.", PLUGIN_NAME);
    let database_filename = get_database_filename(&app_config);
    let mut netw_params = read_network_parameters(&app_config);
    // NSE requires browser-like headers
    netw_params.referrer_url = Some(BASE_URL.to_string());
    let client = make_http_client(&netw_params);

    // Warm up the session: NSE sets cookies on the landing page that are required for its
    // JSON API and listing endpoints. The shared client has a cookie store, so this primes it.
    let warmup = http_get(&BASE_URL.to_string(), &client, 1, netw_params.wait_time_min);
    if warmup.is_empty() {
        warn!("{}: Could not load NSE landing page to establish session cookies.", PLUGIN_NAME);
    } else {
        info!("{}: Established NSE session via landing page.", PLUGIN_NAME);
    }

    let mut max_pages: u64 = 2;
    if let Some(v) = get_plugin_cfg!(PLUGIN_NAME, "max_pages", &app_config) {
        if let Ok(n) = v.parse::<u64>() { max_pages = n; }
    }

    let already_retrieved_urls = get_urls_from_database(database_filename.as_str(), PLUGIN_NAME);
    info!("{}: Got {} previously retrieved urls.", PLUGIN_NAME, already_retrieved_urls.len());

    // NSE publishes circulars at a paginated HTML listing
    let listing_sections = vec![
        ("https://www.nseindia.com/regulations/circulars", "Circulars"),
        ("https://www.nseindia.com/regulations/notices", "Notices"),
    ];

    let mut counter: usize = 0;
    for (base_section_url, section_name) in listing_sections {
        for pageno in 0..max_pages {
            let listing_url = if pageno == 0 {
                base_section_url.to_string()
            } else {
                format!("{}?page={}", base_section_url, pageno)
            };

            let content = http_get(&listing_url, &client, netw_params.retry_times, netw_params.wait_time_min);
            if content.is_empty() {
                warn!("{}: Empty response from {}", PLUGIN_NAME, listing_url);
                continue;
            }

            let count = extract_docs_from_nse_listing(
                content,
                &tx,
                &listing_url,
                section_name,
                &already_retrieved_urls,
                &client,
                &netw_params,
            );
            counter += count;
        }
    }

    // Also try NSE's JSON API endpoint for latest circulars (may require session cookies)
    let api_url = "https://www.nseindia.com/api/latest-circular?index=equities".to_string();
    let api_content = http_get(&api_url, &client, netw_params.retry_times, netw_params.wait_time_min);
    if !api_content.is_empty() {
        if let Ok(json_val) = serde_json::from_str::<Value>(&api_content) {
            if let Some(data_arr) = json_val.as_array() {
                for item in data_arr {
                    if let Some(url_str) = item.get("filePath").and_then(|v| v.as_str()) {
                        if already_retrieved_urls.contains(url_str) {
                            continue;
                        }
                        let mut doc = Document::default();
                        doc.module = PLUGIN_NAME.to_string();
                        doc.plugin_name = PUBLISHER_NAME.to_string();
                        doc.source_author = PUBLISHER_NAME.to_string();
                        doc.section_name = "Circulars".to_string();
                        doc.url = format!("{}{}", BASE_URL.trim_end_matches('/'), url_str);
                        doc.title = item.get("subject")
                            .and_then(|v| v.as_str())
                            .unwrap_or("NSE Circular")
                            .to_string();

                        if let Some(date_str) = item.get("notif_dt").and_then(|v| v.as_str()) {
                            if let Ok(d) = NaiveDate::parse_from_str(date_str, "%d-%b-%Y") {
                                doc.publish_date = d.format("%Y-%m-%d").to_string();
                                doc.publish_date_ms = to_local_datetime(d).timestamp();
                            }
                        }
                        if doc.publish_date == "1970-01-01" {
                            doc.publish_date = Utc::now().format("%Y-%m-%d").to_string();
                        }
                        doc.unique_id = item.get("nseCircularNumber")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        doc.data_proc_flags = document::DATA_PROC_CLASSIFY_INDUSTRY
                            | document::DATA_PROC_EXTRACT_NAME_ENTITY
                            | document::DATA_PROC_SUMMARIZE;

                        match tx.send(doc) {
                            Ok(_) => counter += 1,
                            Err(e) => error!("{}: Channel send error: {}", PLUGIN_NAME, e),
                        }
                    }
                }
            }
        }
    }

    // Also download today's NSE equity bhavcopy CSV.
    download_nse_bhavcopy(&tx, &client, &already_retrieved_urls);

    info!("{}: Completed, retrieved {} documents.", PLUGIN_NAME, counter);
}

/// NSE equity bhavcopy URL (UDiFF zip) for a given date.
///   https://nsearchives.nseindia.com/content/cm/BhavCopy_NSE_CM_0_0_0_{YYYYMMDD}_F_0000.csv.zip
fn nse_bhavcopy_url_for(date: NaiveDate) -> String {
    let date_compact = date.format("%Y%m%d").to_string();
    format!(
        "https://nsearchives.nseindia.com/content/cm/BhavCopy_NSE_CM_0_0_0_{}_F_0000.csv.zip",
        date_compact
    )
}

/// Download the most recent available NSE equity bhavcopy and send it as a document.
///
/// The current day's file is published only after market close and not at all on
/// weekends/holidays, so we walk back over recent business days and use the first that
/// returns a valid zip.
fn download_nse_bhavcopy(
    tx: &Sender<Document>,
    client: &reqwest::blocking::Client,
    already_retrieved: &HashSet<String>,
) {
    // NSE requires a desktop Chrome User-Agent for bhavcopy downloads.
    let bhavcopy_client = reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .unwrap_or_else(|_| client.clone());

    let candidate_days = recent_business_days(Utc::now().date_naive(), NSE_LOOKBACK_BUSINESS_DAYS);

    for date in candidate_days {
        let bhavcopy_url = nse_bhavcopy_url_for(date);
        let date_str = date.format("%Y-%m-%d").to_string();

        if already_retrieved.contains(&bhavcopy_url) {
            info!("{}: NSE bhavcopy for {} already retrieved, stopping.", PLUGIN_NAME, date_str);
            return;
        }

        info!("{}: Trying NSE bhavcopy for {}: {}", PLUGIN_NAME, date_str, bhavcopy_url);
        let zip_bytes = http_get_binary(&bhavcopy_url, &bhavcopy_client);

        // Empty or HTML (starts with '<') means the file is not published for this date.
        if zip_bytes.is_empty() || zip_bytes.first() == Some(&b'<') {
            warn!("{}: No bhavcopy for {} (not yet published / non-trading day), trying earlier day.",
                PLUGIN_NAME, date_str);
            continue;
        }

        let cursor = std::io::Cursor::new(zip_bytes.as_ref());
        let mut archive = match ZipArchive::new(cursor) {
            Ok(a) => a,
            Err(e) => {
                warn!("{}: NSE bhavcopy zip for {} invalid: {}; trying earlier day.", PLUGIN_NAME, date_str, e);
                continue;
            }
        };

        let mut csv_content = String::new();
        let mut found = false;
        for i in 0..archive.len() {
            let mut entry = match archive.by_index(i) {
                Ok(e) => e,
                Err(e) => { warn!("{}: Zip entry error: {}", PLUGIN_NAME, e); continue; }
            };
            if entry.name().to_lowercase().ends_with(".csv") {
                if let Err(e) = entry.read_to_string(&mut csv_content) {
                    error!("{}: Failed reading NSE CSV: {}", PLUGIN_NAME, e);
                } else {
                    found = true;
                }
                break;
            }
        }

        if !found || csv_content.is_empty() {
            warn!("{}: No CSV inside NSE bhavcopy zip for {}; trying earlier day.", PLUGIN_NAME, date_str);
            continue;
        }

        let row_count = csv_content.lines().count().saturating_sub(1);

        let mut doc = Document::default();
        doc.module = PLUGIN_NAME.to_string();
        doc.plugin_name = PUBLISHER_NAME.to_string();
        doc.source_author = PUBLISHER_NAME.to_string();
        doc.section_name = "Equity Bhavcopy".to_string();
        doc.url = bhavcopy_url;
        doc.title = format!("NSE Equity Bhavcopy {}", date_str);
        doc.publish_date = date_str.clone();
        doc.publish_date_ms = to_local_datetime(date).timestamp();
        doc.text = csv_content;

        match tx.send(doc) {
            Ok(_) => info!("{}: Sent NSE bhavcopy document for {} ({} rows).", PLUGIN_NAME, date_str, row_count),
            Err(e) => error!("{}: Channel send error: {}", PLUGIN_NAME, e),
        }
        return;
    }

    warn!("{}: No NSE bhavcopy found in the last {} business days.", PLUGIN_NAME, NSE_LOOKBACK_BUSINESS_DAYS);
}

fn extract_docs_from_nse_listing(
    content: String,
    tx: &Sender<Document>,
    listing_url: &str,
    section_name: &str,
    already_retrieved_urls: &HashSet<String>,
    client: &reqwest::blocking::Client,
    netw_params: &crate::network::NetworkParameters,
) -> usize {
    let mut counter = 0;
    let html = scraper::Html::parse_document(&content);
    let row_sel = scraper::Selector::parse("table tbody tr, div.circular-item, li.circular-row").unwrap();
    let link_sel = scraper::Selector::parse("a").unwrap();

    for row in html.select(&row_sel) {
        let mut url = String::new();
        let mut title = String::new();

        for alink in row.select(&link_sel) {
            title = clean_text(alink.inner_html());
            if let Some(href) = alink.value().attr("href") {
                url = href.to_string();
            }
        }

        if url.is_empty() || title.is_empty() {
            continue;
        }
        if let Some(fixed_url) = check_and_fix_url(&url, BASE_URL) {
            url = fixed_url;
        } else {
            continue;
        }
        if already_retrieved_urls.contains(&url) {
            continue;
        }

        let mut doc = Document::default();
        doc.module = PLUGIN_NAME.to_string();
        doc.plugin_name = PUBLISHER_NAME.to_string();
        doc.source_author = PUBLISHER_NAME.to_string();
        doc.section_name = section_name.to_string();
        doc.url = url.clone();
        doc.title = title;
        doc.publish_date = Utc::now().format("%Y-%m-%d").to_string();
        doc.publish_date_ms = Utc::now().timestamp();
        doc.links_inward = vec![listing_url.to_string()];
        doc.data_proc_flags = document::DATA_PROC_CLASSIFY_INDUSTRY
            | document::DATA_PROC_EXTRACT_NAME_ENTITY
            | document::DATA_PROC_SUMMARIZE;

        match tx.send(doc) {
            Ok(_) => counter += 1,
            Err(e) => error!("{}: Channel send error: {}", PLUGIN_NAME, e),
        }
    }
    counter
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
}
