// file: mod_en_in_sebi
// Purpose: Retrieve data published by SEBI (Securities and Exchange Board of India)

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::panic;
use std::panic::AssertUnwindSafe;
use chrono::NaiveDate;
use log::{error, info};
use reqwest::blocking::Client;
use scraper::ElementRef;
use crate::{document, get_plugin_cfg};
use crate::cfg::{get_data_folder, get_database_filename, get_pdf_data_folder};
use crate::content_extraction::extract_text_from_html;
use crate::document::Document;
use crate::network::{NetworkParameters, http_get, make_http_client, read_network_parameters};
use crate::utils::{check_and_fix_url, get_urls_from_database, load_pdf_content, make_unique_filename, to_local_datetime};

pub(crate) const PLUGIN_NAME: &str = "mod_en_in_sebi";
const PUBLISHER_NAME: &str = "Securities and Exchange Board of India";
const BASE_URL: &str = "https://www.sebi.gov.in/";


/// Executes this function of the module in the separate thread launched by the pipeline/queue module
///
/// # Arguments
///
/// * `tx`: The channel to transmit newly identified or web scraped documents
/// * `app_config`: The application configuration object to be used to get various config parameters
///
/// returns: ()
pub(crate) fn run_worker_thread(tx: Sender<document::Document>, app_config: Arc<config::Config>) {

    info!("{}: Starting plugin.", PLUGIN_NAME);

    let database_filename = get_database_filename(&app_config);
    let data_folder = get_data_folder(&app_config);
    let data_folder_str = data_folder.to_str().unwrap_or("data").to_string();
    let pdf_folder = get_pdf_data_folder(&app_config);
    let pdf_folder_str = pdf_folder.to_str().unwrap_or("data/master_data").to_string();

    let mut counter = 0;
    let mut netw_params = read_network_parameters(&app_config);
    netw_params.referrer_url = Some(BASE_URL.to_string());
    let client = make_http_client(&netw_params);

    let mut already_retrieved_urls = get_urls_from_database(database_filename.as_str(), PLUGIN_NAME);
    info!("For Plugin {}: Got {} previously retrieved urls from table.", PLUGIN_NAME, already_retrieved_urls.len());

    let mut max_pages: u64 = 1;
    match get_plugin_cfg!(PLUGIN_NAME, "max_pages", &app_config) {
        Some(max_pages_str) => {
            match max_pages_str.parse::<u64>() {
                Ok(val) => max_pages = val,
                Err(e) => error!("{}: Could not parse max_pages: {}", PLUGIN_NAME, e),
            }
        },
        None => {}
    };

    let section_listing_url_params = vec![
        // sid, ssid, smid, section_name
        ( 1, 1, 0, "Acts"),
        ( 1, 2, 0, "Rules"),
        ( 1, 3, 0, "Regulations"),
        ( 1, 4, 0, "General Orders"),
        ( 1, 5, 0, "Guidelines"),
        ( 1, 6, 0, "Master Circulars"),
        ( 1, 7, 0, "Circulars"),
        ( 1, 8, 0, "Gazette notifications"),
        ( 2, 9, 3, "Settlement Orders"),
        ( 2, 9, 77, "Special Court Orders"),
        ( 2, 9, 7, "Court Orders"),
        ( 2, 9, 6, "Orders of AO"),
        ( 2, 9, 2, "Orders of chairpersons"),
        ( 2, 9, 133, "Orders of ED"),
        ( 2, 1, 0, "Informal Guidance"),
        ( 2, 5, 0, "Recovery Proceedings"),
    ];

    for (sid, ssid, smid, section_name) in section_listing_url_params {
        for pageno in 1..(max_pages + 1) {

            // retrieve content from this url and extract vector of documents, mainly individual urls to retrieve.
            let content = get_sebi_urllist(&client, sid.to_string(), ssid.to_string(), smid.to_string(), section_name, pageno);
            let url_listing_page = format!("https://www.sebi.gov.in/sebiweb/home/HomeAction.do?doListing=yes&sid={}&ssid={}&smid={}", sid, ssid, smid);

            let count_of_docs = sebi_retrieve_docs(
                content,
                &tx,
                url_listing_page.as_str(),
                section_name,
                &mut already_retrieved_urls,
                &client,
                &netw_params,
                data_folder_str.as_str(),
                pdf_folder_str.as_str()
            );

            counter += count_of_docs;
        }
    }
    info!("{}: Completed retrieving {} documents.", PLUGIN_NAME, counter);
}

/// Minimal application/x-www-form-urlencoded value encoder (RFC 3986 unreserved set kept
/// literal; everything else percent-encoded). Avoids a dependency on `RequestBuilder::form`.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push_str("%20"),
            other => out.push_str(&format!("%{:02X}", other)),
        }
    }
    out
}

/// Build the form-encoded request body from key/value pairs.
fn encode_form(params: &[(&str, &str)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", urlencode(k), urlencode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Compute the (next, nextValue, doDirect) listing parameters for a page number.
///
/// The live site loads the FIRST page with the "search" direction and doDirect=-1 (mirrors
/// `searchFormNewsList('s','-1')`); using next='n'/doDirect=1 returns ZERO rows for shorter
/// sections (Acts, Guidelines, General Orders), which is why those were never retrieved.
/// Subsequent pages use next='n' with the (1-based) page index minus one. (Verified live
/// 2026-06-13.)
fn page_nav_params(page_no: u64) -> (String, String, String) {
    if page_no <= 1 {
        ("s".to_string(), "-1".to_string(), "-1".to_string())
    } else {
        let p = (page_no - 1).to_string();
        ("n".to_string(), p.clone(), p)
    }
}

fn get_sebi_urllist(
    client: &Client,
    sid: String,
    ssid: String,
    smid: String,
    sub_section_name: &str,
    page_no: u64,
) -> String {

    let listing_url = "https://www.sebi.gov.in/sebiweb/ajax/home/getnewslistinfo.jsp";

    let (next, next_value, do_direct) = page_nav_params(page_no);

    // Form-encoded payload. NOTE: this MUST be sent as application/x-www-form-urlencoded
    // (.form), NOT JSON — SEBI's WAF returns HTTP 530 "Unauthorized Request Blocked" for a
    // JSON body. The X-Requested-With header marks it as an XHR as the site does.
    let params = [
        ("nextValue", next_value.as_str()),
        ("next", next.as_str()),
        ("search", ""),
        ("fromDate", ""),
        ("toDate", ""),
        ("fromYear", ""),
        ("toYear", ""),
        ("deptId", "-1"),
        ("sid", sid.as_str()),
        ("ssid", ssid.as_str()),
        ("smid", smid.as_str()),
        ("ssidhidden", ssid.as_str()),
        ("intmid", "-1"),
        ("sText", "Legal"),
        ("ssText", sub_section_name),
        ("smText", ""),
        ("doDirect", do_direct.as_str()),
    ];

    let body = encode_form(&params);

    // get response
    match client
        .post(listing_url)
        .header("X-Requested-With", "XMLHttpRequest")
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(body)
        .send()
    {
        Ok(resp) => {
            let status = resp.status();
            crate::metrics::record_http_status(status.as_u16());
            match resp.text() {
                Ok(resptext) => return resptext,
                Err(e) => error!("{}: When reading listing response body: {}", PLUGIN_NAME, e),
            }
        }
        Err(e) => {
            crate::metrics::record_http_transport_error();
            error!("{}: When getting list of urls: {}", PLUGIN_NAME, e)
        }
    }

    String::new()
}

pub fn sebi_retrieve_docs(
    content: String,
    tx: &Sender<document::Document>,
    url_listing_page: &str,
    section_name: &str,
    already_retrieved_urls: &mut HashSet<String>,
    client: &reqwest::blocking::Client,
    netw_params: &NetworkParameters,
    data_folder: &str,
    pdf_folder: &str) -> usize
{
    let mut counter: usize = 0;

    // Descendant (not direct-child) selector: robust whether or not a <tbody> is present in
    // the source (the AJAX fragment omits it; html5ever inserts one on parse). The header
    // row lives in <thead> so it is excluded; any stray header row yields no <a> and is
    // skipped downstream.
    let rows_selector = scraper::Selector::parse("table.dataTable tbody tr").unwrap();
    info!("{}: Retrieving url listing from: {}", PLUGIN_NAME, url_listing_page);
    let html_document = scraper::Html::parse_document(&content.as_str());

    'rows_loop: for row_each in html_document.select(&rows_selector) {

        let mut this_new_doc = extract_sebi_doc_from_row(row_each, url_listing_page);

        this_new_doc.module = PLUGIN_NAME.to_string();
        this_new_doc.plugin_name = PUBLISHER_NAME.to_string();
        this_new_doc.source_author = PUBLISHER_NAME.to_string();
        this_new_doc.section_name = section_name.to_string();

        // Initialize classification with default "other" values
        this_new_doc.classification = HashMap::from([
            ("channel".to_string(), "other".to_string()),
            ("customer_type".to_string(), "other".to_string()),
            ("function".to_string(), "other".to_string()),
            ("market_type".to_string(), "other".to_string()),
            ("occupation".to_string(), "other".to_string()),
            ("product_type".to_string(), "other".to_string()),
            ("risk_type".to_string(), "other".to_string()),
            ("doc_type".to_string(), "regulatory-notification".to_string()),
        ]);

        this_new_doc.data_proc_flags = document::DATA_PROC_CLASSIFY_INDUSTRY |
            document::DATA_PROC_CLASSIFY_MARKET | document::DATA_PROC_CLASSIFY_PRODUCT |
            document::DATA_PROC_EXTRACT_NAME_ENTITY | document::DATA_PROC_SUMMARIZE |
            document::DATA_PROC_EXTRACT_ACTIONS;

        if already_retrieved_urls.contains(&this_new_doc.url) {
            info!("{}: Ignoring already retrieved url: {}", PLUGIN_NAME, this_new_doc.url);
            continue 'rows_loop;
        }

        if let Some(proper_url) = check_and_fix_url(this_new_doc.url.as_str(), BASE_URL) {
            this_new_doc.url = proper_url;
        } else {
            info!("{}: Ignoring invalid url: {}", PLUGIN_NAME, this_new_doc.url);
            continue 'rows_loop;
        }

        extract_content_from_sebi_page(&mut this_new_doc, client, netw_params, data_folder);

        _ = already_retrieved_urls.insert(this_new_doc.url.clone());

        let filename = make_unique_filename(&this_new_doc, "json");
        let json_file_path = Path::new(data_folder).join(filename);
        this_new_doc.filename = String::from(
            json_file_path.as_path().to_str().expect("Trying to convert path to string")
        );

        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            load_pdf_content(&mut this_new_doc, &client, pdf_folder);
        }));
        if result.is_err() {
            if let Err(errvar) = result {
                error!("{}: When reading PDF of document '{}' the error was: {:?}", PLUGIN_NAME, this_new_doc.title, errvar);
            }
        }
        info!("{}: Retrieved document titled: '{}', with content text length: {}",
            PLUGIN_NAME, this_new_doc.title, this_new_doc.text.len());

        match tx.send(this_new_doc) {
            Result::Ok(_res) => {
                counter += 1;
            },
            Err(e) => error!("{}: When sending document via channel: {}", PLUGIN_NAME, e)
        }
    }
    return counter;
}

fn extract_sebi_doc_from_row(row_each: ElementRef, url_listing_page: &str) -> Document {
    let mut doc = Document::default();
    doc.links_inward.push(url_listing_page.to_string());

    let alink_selector = scraper::Selector::parse("a").unwrap();
    let cell_selector = scraper::Selector::parse("td").unwrap();

    for cell in row_each.select(&cell_selector) {

        for alink in cell.select(&alink_selector) {
            info!("title: {}", alink.inner_html());
            doc.title = alink.inner_html().trim().to_string();
            info!("Link: {}", alink.attr("href").unwrap_or_else(|| ""));
            doc.url = alink.attr("href").unwrap_or_else(|| "").to_string();
        }

        if cell.select(&alink_selector).count() == 0 {
            let date_str = cell.inner_html();
            match NaiveDate::parse_from_str(date_str.as_str(), "%b %d, %Y") {
                Ok(naive_date) => {
                    doc.publish_date_ms = to_local_datetime(naive_date).timestamp();
                    doc.publish_date = naive_date.format("%Y-%m-%d").to_string();
                },
                Err(date_err) => {
                    error!("Could not parse date '{}', error: {}", date_str.as_str(), date_err)
                }
            }
        }

    }
    doc
}


fn extract_content_from_sebi_page(new_doc: &mut Document, client: &Client, netw_params: &NetworkParameters, _data_folder: &str) {

    // extract content from url:
    let content = http_get(&(new_doc.url), client, netw_params.retry_times, netw_params.wait_time_min);
    let html_document = scraper::Html::parse_document(&content.as_str());

    let unique_circ_no_select = scraper::Selector::parse("div.id_area").expect("Construct circular no selector");
    let iframe_select = scraper::Selector::parse("iframe").expect("Construct iframe selector");
    let date_select = scraper::Selector::parse("div.date_value>h5").expect("Construct date selector");
    let span_select = scraper::Selector::parse("span").expect("Construct span selector");

    if let Some(uniquediv) = html_document.select(&unique_circ_no_select).nth(0) {
        if uniquediv.has_children() {
            if let Some(second_span) = uniquediv.select(&span_select).nth(1) {
                new_doc.unique_id = second_span.inner_html();
            }
        }
    }

    if let Some(datenode) = html_document.select(&date_select).nth(0) {
        let date_str = datenode.inner_html();
        match NaiveDate::parse_from_str(date_str.as_str(), "%b %d, %Y") {
            Ok(naive_date) => {
                new_doc.publish_date_ms = to_local_datetime(naive_date).timestamp();
                new_doc.publish_date = naive_date.format("%Y-%m-%d").to_string();
            },
            Err(date_err) => {
                error!("Could not parse date '{}', error: {}", date_str.as_str(), date_err)
            }
        }
    }

    if let Some(iframe) = html_document.select(&iframe_select).nth(0) {
        if let Some(src_attr) = iframe.attr("src") {
            // The iframe src is either:
            //   https://www.sebi.gov.in/web/?file=https://www.sebi.gov.in/sebi_data/...pdf
            //   ../../../web/?file=https://...pdf
            // Extract the real PDF URL from the ?file= query parameter.
            let pdf_url = if let Some(pos) = src_attr.find("?file=") {
                src_attr[pos + 6..].to_string()
            } else {
                src_attr.to_string()
            };
            new_doc.pdf_url = pdf_url;
        }
    }

    // Capture the inline HTML body so HTML-only documents (e.g. some guidelines/circulars
    // that render text rather than only embedding a PDF) persist as JSON articles with text.
    // The PDF (when present) is still downloaded later by load_pdf_content, which overwrites
    // `text` when it yields more content.
    for sel_str in &["div.content-box", "div.m_section", "div#main-content", "div.news-detail-slider"] {
        if let Ok(sel) = scraper::Selector::parse(sel_str) {
            if let Some(div) = html_document.select(&sel).next() {
                let inner = div.inner_html();
                if inner.len() > 200 {
                    new_doc.html_content = inner.clone();
                    new_doc.text = extract_text_from_html(&inner);
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scraper::{Html, Selector};

    // First page must use the search direction (doDirect=-1); this is the fix that makes
    // short sections (Acts/Guidelines/General Orders) return rows.
    #[test]
    fn test_page_nav_params_first_page() {
        assert_eq!(page_nav_params(1), ("s".to_string(), "-1".to_string(), "-1".to_string()));
        assert_eq!(page_nav_params(0), ("s".to_string(), "-1".to_string(), "-1".to_string()));
    }

    #[test]
    fn test_page_nav_params_subsequent_pages() {
        assert_eq!(page_nav_params(2), ("n".to_string(), "1".to_string(), "1".to_string()));
        assert_eq!(page_nav_params(3), ("n".to_string(), "2".to_string(), "2".to_string()));
    }

    #[test]
    fn test_urlencode_and_form() {
        assert_eq!(urlencode("Settlement Orders"), "Settlement%20Orders");
        assert_eq!(urlencode("-1"), "-1");
        assert_eq!(urlencode("a&b=c"), "a%26b%3Dc");
        let body = encode_form(&[("ssText", "Settlement Orders"), ("doDirect", "-1")]);
        assert_eq!(body, "ssText=Settlement%20Orders&doDirect=-1");
    }

    // The tbody-robust selector must match data rows and exclude the <thead> header row,
    // against the real SEBI dataTable structure (captured live 2026-06-13).
    #[test]
    fn test_datatable_row_selection_and_extraction() {
        let fragment = r#"
        <table class='table table-striped bordered fix_table dataTable no-footer' id='sample_1'>
          <thead><tr role='row'><th>Date</th><th>Title</th></tr></thead>
          <tr role='row' class='odd'>
            <td>Mar 06, 2026</td>
            <td><a href='https://www.sebi.gov.in/legal/guidelines/mar-2026/anti-money-laundering-aml-guidelines_100200.html' target="_blank" title="AML Guidelines" class='points'>Anti Money Laundering (AML) Guidelines</a></td>
          </tr>
          <tr role='row' class='even'>
            <td>Mar 04, 2026</td>
            <td><a href='https://www.sebi.gov.in/legal/circulars/mar-2026/regulatory-reporting-by-aifs_100120.html' class='points'>Regulatory Reporting by AIFs</a></td>
          </tr>
        </table>"#;

        let doc = Html::parse_document(fragment);
        let rows_sel = Selector::parse("table.dataTable tbody tr").unwrap();
        let rows: Vec<_> = doc.select(&rows_sel).collect();
        assert_eq!(rows.len(), 2, "should select 2 data rows, excluding the thead header");

        let first = extract_sebi_doc_from_row(rows[0], "https://listing");
        assert_eq!(first.title, "Anti Money Laundering (AML) Guidelines");
        assert_eq!(
            first.url,
            "https://www.sebi.gov.in/legal/guidelines/mar-2026/anti-money-laundering-aml-guidelines_100200.html"
        );
        assert_eq!(first.publish_date, "2026-03-06");
    }
}
