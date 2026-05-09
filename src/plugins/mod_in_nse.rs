// file: mod_in_nse.rs
// Retrieve regulatory circulars and announcements from NSE India

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use chrono::{NaiveDate, Utc};
use log::{error, info, warn};
use serde_json::Value;

use crate::{document, get_plugin_cfg};
use crate::cfg::{get_data_folder, get_database_filename};
use crate::document::Document;
use crate::network::{http_get, make_http_client, read_network_parameters};
use crate::utils::{check_and_fix_url, clean_text, get_urls_from_database, to_local_datetime};

pub(crate) const PLUGIN_NAME: &str = "mod_in_nse";
const PUBLISHER_NAME: &str = "National Stock Exchange of India";
const BASE_URL: &str = "https://www.nseindia.com/";

pub(crate) fn run_worker_thread(tx: Sender<Document>, app_config: Arc<config::Config>) {

    info!("{}: Starting plugin.", PLUGIN_NAME);
    let database_filename = get_database_filename(&app_config);
    let mut netw_params = read_network_parameters(&app_config);
    // NSE requires browser-like headers
    netw_params.referrer_url = Some(BASE_URL.to_string());
    let client = make_http_client(&netw_params);

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

    info!("{}: Completed, retrieved {} documents.", PLUGIN_NAME, counter);
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
