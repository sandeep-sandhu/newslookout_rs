// file: mod_in_bse.rs
// Download BSE (Bombay Stock Exchange) equity bhavcopy CSV and push through the pipeline.
//
// BSE publishes the daily equity bhavcopy as a ZIP archive at:
//   https://www.bseindia.com/download/BhavCopy/Equity/EQ{DD}{MM}{YY}_CSV.ZIP
// The archive contains a single CSV file (columns: Sc_Code,Sc_Name,Open,High,Low,Close,...).

use std::io::Read;
use std::sync::Arc;
use std::sync::mpsc::Sender;

use chrono::Utc;
use log::{error, info, warn};
use zip::ZipArchive;

use crate::cfg::get_database_filename;
use crate::document::Document;
use crate::network::{http_get_binary, make_http_client, read_network_parameters};
use crate::utils::get_urls_from_database;

pub(crate) const PLUGIN_NAME: &str = "mod_in_bse";
const PUBLISHER_NAME: &str = "Bombay Stock Exchange";
const BASE_URL: &str = "https://www.bseindia.com/";

pub(crate) fn run_worker_thread(tx: Sender<Document>, app_config: Arc<config::Config>) {
    info!("{}: Starting plugin.", PLUGIN_NAME);

    let mut netw_params = read_network_parameters(&app_config);
    netw_params.referrer_url = Some(BASE_URL.to_string());
    let client = make_http_client(&netw_params);

    let database_filename = get_database_filename(&app_config);
    let already_retrieved = get_urls_from_database(&database_filename, PLUGIN_NAME);

    let today = Utc::now();
    let day = today.format("%d").to_string();
    let month = today.format("%m").to_string();
    let year_2 = today.format("%y").to_string();
    let date_str = today.format("%Y-%m-%d").to_string();

    // BSE bhavcopy URL: EQ{DD}{MM}{YY}_CSV.ZIP
    let bhavcopy_url = format!(
        "{}download/BhavCopy/Equity/EQ{}{}{}_CSV.ZIP",
        BASE_URL, day, month, year_2
    );

    if already_retrieved.contains(&bhavcopy_url) {
        info!("{}: BSE bhavcopy already retrieved today, skipping.", PLUGIN_NAME);
        return;
    }

    info!("{}: Downloading BSE bhavcopy from: {}", PLUGIN_NAME, bhavcopy_url);
    // BSE requires a Windows Chrome User-Agent to serve the bhavcopy file.
    let bhavcopy_client = reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/114.0.0.0 Safari/537.36")
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .unwrap_or_else(|_| client.clone());

    let zip_bytes = http_get_binary(&bhavcopy_url, &bhavcopy_client);

    if zip_bytes.is_empty() {
        warn!("{}: Empty response for BSE bhavcopy: {}", PLUGIN_NAME, bhavcopy_url);
        info!("{}: Completed with 0 documents.", PLUGIN_NAME);
        return;
    }

    // Extract the CSV file from the downloaded ZIP archive.
    // If bytes start with '<' the server returned an HTML error page instead of a zip.
    if zip_bytes.first() == Some(&b'<') {
        warn!("{}: BSE returned an HTML page instead of a zip file — authentication or data not yet published.", PLUGIN_NAME);
        return;
    }
    let cursor = std::io::Cursor::new(zip_bytes.as_ref());
    let mut archive = match ZipArchive::new(cursor) {
        Ok(a) => a,
        Err(e) => {
            warn!("{}: BSE bhavcopy zip invalid (data may not be published yet): {}", PLUGIN_NAME, e);
            return;
        }
    };

    let mut csv_content = String::new();
    let mut found_csv = false;

    for i in 0..archive.len() {
        let mut entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(e) => {
                warn!("{}: Zip entry #{} error: {}", PLUGIN_NAME, i, e);
                continue;
            }
        };
        if entry.name().expect("REASON").to_lowercase().ends_with(".csv") {
            match entry.read_to_string(&mut csv_content) {
                Ok(_) => { found_csv = true; }
                Err(e) => error!("{}: Failed to read CSV from zip: {}", PLUGIN_NAME, e),
            }
            break;
        }
    }

    if !found_csv || csv_content.is_empty() {
        warn!("{}: No CSV found in BSE bhavcopy zip: {}", PLUGIN_NAME, bhavcopy_url);
        return;
    }

    let row_count = csv_content.lines().count().saturating_sub(1);
    info!("{}: Extracted BSE bhavcopy CSV with {} data rows.", PLUGIN_NAME, row_count);

    let mut doc = Document::default();
    doc.module = PLUGIN_NAME.to_string();
    doc.plugin_name = PUBLISHER_NAME.to_string();
    doc.source_author = PUBLISHER_NAME.to_string();
    doc.section_name = "Equity Bhavcopy".to_string();
    doc.url = bhavcopy_url.clone();
    doc.title = format!("BSE Equity Bhavcopy {}", date_str);
    doc.publish_date = date_str;
    doc.publish_date_ms = today.timestamp();
    doc.text = csv_content;

    match tx.send(doc) {
        Ok(_) => info!("{}: Sent BSE bhavcopy document ({} rows).", PLUGIN_NAME, row_count),
        Err(e) => error!("{}: Channel send error: {}", PLUGIN_NAME, e),
    }

    info!("{}: Completed.", PLUGIN_NAME);
}
