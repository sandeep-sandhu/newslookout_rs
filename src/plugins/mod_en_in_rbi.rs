// file: mod_en_in_rbi.rs
// Purpose: Retrieve data published by RBI from https://www.rbi.org.in/ (ASP.NET portal)
//
// Website structure:
//   - Listing pages: <table class="tablebg">
//       Date header row: <tr><td class="tableheader"><b>Month DD, YYYY</b></td></tr>
//                    or: <tr><th>DD Month YYYY</th></tr>  (WSS pages)
//       Article row:    <tr><td><a class="link2" href="relative.aspx?Id=N">TITLE</a></td>
//                           <td><a id="APDF_..." href="https://rbidocs.rbi.org.in/...PDF">...</a>
//                               <a id="ADOC_..." href="https://rbidocs.rbi.org.in/...XLSX">...</a>
//                           </td></tr>
//   - Detail pages: content in <div class="content_area"> → <div class="text1">
//
// Content preference: HTML article page → PDF text extraction
// Dataset files (XLSX): saved to master_data_dir with _YYYY_MM_DD.xlsx suffix

use std::collections::HashSet;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use chrono::NaiveDate;
use log::{debug, error, info};
use rand::RngExt;
use scraper::{Html, Selector};
use regex::Regex;

use crate::document;
use crate::document::Document;
use crate::get_plugin_cfg;
use crate::cfg::{get_data_folder, get_database_filename, get_master_data_folder, get_pdf_data_folder};
use crate::content_extraction::{extract_text_from_html, html_to_markdown};
use crate::network::{self, read_network_parameters, make_http_client, NetworkParameters};
use crate::utils::{
    clean_text, get_text_from_element, get_urls_from_database,
    make_unique_filename, to_local_datetime, load_pdf_content,
};

pub(crate) const PLUGIN_NAME: &str = "mod_en_in_rbi";
const PUBLISHER_NAME: &str = "Reserve Bank of India";
const BASE_URL: &str = "https://www.rbi.org.in/";

/// Resolve a potentially-relative href against the section listing page URL.
fn resolve_url(href: &str, section_url: &str) -> String {
    let href = href.trim();
    if href.is_empty() { return String::new(); }
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_string();
    }
    if href.starts_with('/') {
        return format!("https://www.rbi.org.in{}", href);
    }
    // Relative: prepend directory portion of section_url
    let dir = section_url.rfind('/').map(|i| &section_url[..=i]).unwrap_or(section_url);
    format!("{}{}", dir, href)
}

/// Parse an RBI date header string into "YYYY-MM-DD".
/// Tries "Month DD, YYYY" (Notifications) and "DD Month YYYY" (WSS).
fn parse_rbi_date(raw: &str) -> Option<String> {
    let s = raw.trim().trim_matches(|c: char| c == ',' || c == '.' || c.is_whitespace());
    for fmt in &["%B %d, %Y", "%d %B %Y", "%d %b %Y", "%B %d %Y"] {
        if let Ok(d) = NaiveDate::parse_from_str(s, fmt) {
            return Some(d.format("%Y-%m-%d").to_string());
        }
    }
    None
}

/// Entry point called by the pipeline on a dedicated thread.
pub(crate) fn run_worker_thread(
    tx: std::sync::mpsc::Sender<document::Document>,
    app_config: Arc<config::Config>,
) {
    info!("{}: Reading plugin specific configuration.", PLUGIN_NAME);
    let mut netw_params = read_network_parameters(&app_config);
    netw_params.referrer_url = Some(BASE_URL.to_string());
    let client = make_http_client(&netw_params);
    let database_filename = get_database_filename(&app_config);

    // max_pages kept for config compatibility; GET-based pagination is not supported
    // by the old ASP.NET RBI site, so only the main listing page is fetched per section.
    if let Some(s) = get_plugin_cfg!(PLUGIN_NAME, "max_pages", &app_config) {
        let _: u64 = s.parse().unwrap_or(1);
    }

    let starter_urls: Vec<(&str, &str)> = vec![
        ("https://www.rbi.org.in/Scripts/NotificationUser.aspx",              "Notifications"),
        ("https://www.rbi.org.in/Scripts/BS_PressReleaseDisplay.aspx",        "Press Releases"),
        ("https://www.rbi.org.in/Scripts/BS_ViewMasterDirections.aspx",       "Master Directions"),
        ("https://www.rbi.org.in/Scripts/BS_ViewMasterCirculardetails.aspx",  "Master Circulars"),
        ("https://www.rbi.org.in/Scripts/DraftNotificationsGuildelines.aspx", "Draft Notifications"),
        ("https://www.rbi.org.in/Scripts/BS_ViewREwiseDraftDirections.aspx",  "Draft Directions"),
        ("https://www.rbi.org.in/Scripts/BS_ViewSpeeches.aspx",               "Speeches"),
        ("https://www.rbi.org.in/Scripts/WssUser.aspx",                       "Weekly Statistical Supplement"),
    ];

    let data_folder   = get_data_folder(&app_config);
    let pdf_folder    = get_pdf_data_folder(&app_config);
    let master_folder = get_master_data_folder(&app_config);

    match (data_folder.to_str(), pdf_folder.to_str(), master_folder.to_str()) {
        (Some(data_dir), Some(pdf_dir), Some(master_dir)) => {
            retrieve_data(
                starter_urls,
                database_filename.as_str(),
                &client,
                tx,
                data_dir,
                pdf_dir,
                master_dir,
                netw_params,
            );
        }
        _ => error!("{}: Unable to determine data/pdf/master folder paths.", PLUGIN_NAME),
    }
}

fn retrieve_data(
    starter_urls: Vec<(&str, &str)>,
    database_filename: &str,
    client: &reqwest::blocking::Client,
    tx: std::sync::mpsc::Sender<document::Document>,
    data_folder: &str,
    pdf_folder: &str,
    master_data_folder: &str,
    netw_params: NetworkParameters,
) -> usize {
    let mut already_retrieved = get_urls_from_database(database_filename, PLUGIN_NAME);
    info!("{}: {} previously retrieved URLs in database.", PLUGIN_NAME, already_retrieved.len());

    let mut rng = rand::rng();
    let mut total = 0usize;

    for (section_url, section_name) in &starter_urls {
        let wait = rng.random_range(netw_params.wait_time_min..=(netw_params.wait_time_min * 3));
        let html = network::http_get(&section_url.to_string(), client, netw_params.retry_times, wait);
        info!("{}: Fetched {} bytes from {}", PLUGIN_NAME, html.len(), section_url);

        let n = get_docs_from_listing_page(
            html,
            &tx,
            section_url,
            section_name,
            &mut already_retrieved,
            client,
            &netw_params,
            data_folder,
            pdf_folder,
            master_data_folder,
        );
        total += n;
    }
    total
}

/// Parse a listing page (table.tablebg), extract each document row, and send documents
/// through the pipeline channel.  Returns the count of documents sent.
pub fn get_docs_from_listing_page(
    content: String,
    tx: &std::sync::mpsc::Sender<document::Document>,
    section_url: &str,
    section_name: &str,
    already_retrieved: &mut HashSet<String>,
    client: &reqwest::blocking::Client,
    netw_params: &NetworkParameters,
    data_folder: &str,
    pdf_folder: &str,
    master_data_folder: &str,
) -> usize {
    let mut counter = 0usize;

    let html = Html::parse_document(&content);

    let row_sel       = Selector::parse("table.tablebg tr").unwrap();
    let th_sel        = Selector::parse("th").unwrap();
    let td_header_sel = Selector::parse("td.tableheader").unwrap();
    let link2_sel     = Selector::parse("a.link2").unwrap();
    let pdf_sel       = Selector::parse("a[id^='APDF_']").unwrap();
    let xls_sel       = Selector::parse("a[id^='ADOC_']").unwrap();

    let total_rows = html.select(&row_sel).count();
    info!("{}: {} rows in table.tablebg on {}", PLUGIN_NAME, total_rows, section_url);

    let mut current_date = String::from("1970-01-01");
    let mut rng = rand::rng();

    'row: for row in html.select(&row_sel) {

        // Date header: WSS uses <th>, Notifications use td.tableheader
        for th in row.select(&th_sel) {
            let text = clean_text(get_text_from_element(th));
            if let Some(d) = parse_rbi_date(&text) { current_date = d; continue 'row; }
        }
        for td in row.select(&td_header_sel) {
            let text = clean_text(get_text_from_element(td));
            if let Some(d) = parse_rbi_date(&text) { current_date = d; continue 'row; }
        }

        // Extract article fields
        let mut title       = String::new();
        let mut article_url = String::new();
        let mut pdf_url     = String::new();
        let mut xls_url     = String::new();

        for lnk in row.select(&link2_sel) {
            title = clean_text(get_text_from_element(lnk));
            if let Some(h) = lnk.value().attr("href") {
                article_url = resolve_url(h, section_url);
            }
        }
        for a in row.select(&pdf_sel) {
            if let Some(h) = a.value().attr("href") {
                if !h.is_empty() { pdf_url = h.to_string(); }
            }
        }
        for a in row.select(&xls_sel) {
            if let Some(h) = a.value().attr("href") {
                if !h.is_empty() { xls_url = h.to_string(); }
            }
        }

        // Primary URL for deduplication: article page > XLS > PDF
        let primary_url = if !article_url.is_empty() {
            article_url.clone()
        } else if !xls_url.is_empty() {
            xls_url.clone()
        } else {
            pdf_url.clone()
        };

        if primary_url.is_empty() { continue; }

        if already_retrieved.contains(&primary_url) {
            debug!("{}: Skipping already retrieved: {}", PLUGIN_NAME, primary_url);
            continue;
        }

        let mut doc = Document::default();
        doc.module        = PLUGIN_NAME.to_string();
        doc.plugin_name   = PUBLISHER_NAME.to_string();
        doc.section_name  = section_name.to_string();
        doc.source_author = PUBLISHER_NAME.to_string();
        doc.url           = primary_url.clone();
        doc.pdf_url       = pdf_url.clone();
        doc.title         = title.clone();
        doc.publish_date  = current_date.clone();
        doc.links_inward  = vec![section_url.to_string()];

        if let Ok(nd) = NaiveDate::parse_from_str(&current_date, "%Y-%m-%d") {
            doc.publish_date_ms = to_local_datetime(nd).timestamp();
        }

        doc.data_proc_flags = document::DATA_PROC_CLASSIFY_INDUSTRY
            | document::DATA_PROC_CLASSIFY_MARKET
            | document::DATA_PROC_CLASSIFY_PRODUCT
            | document::DATA_PROC_EXTRACT_NAME_ENTITY
            | document::DATA_PROC_SUMMARIZE
            | document::DATA_PROC_EXTRACT_ACTIONS;

        // Download XLS dataset file to master_data_folder (prefer XLS over PDF for datasets)
        if !xls_url.is_empty() {
            download_xls_file(&xls_url, &current_date, client, master_data_folder);
        }

        // Prefer HTML content from the article detail page; fall back to PDF
        if !article_url.is_empty() {
            let wait = rng.random_range(netw_params.wait_time_min..=(netw_params.wait_time_max * 3));
            populate_content_in_doc(&mut doc, &article_url, client, netw_params.retry_times, wait);
        }

        if doc.text.is_empty() && !pdf_url.is_empty() {
            load_pdf_content(&mut doc, client, pdf_folder);
        }

        custom_data_processing(&mut doc);

        let fname = make_unique_filename(&doc, "json");
        doc.filename = Path::new(data_folder).join(fname).to_str().unwrap_or("").to_string();

        already_retrieved.insert(primary_url.clone());

        match tx.send(doc) {
            Ok(_) => {
                counter += 1;
                debug!("{}: Sent doc #{} — {}", PLUGIN_NAME, counter, primary_url);
            }
            Err(e) => error!("{}: Channel send error: {}", PLUGIN_NAME, e),
        }
    }

    info!("{}: Extracted {} docs from {}", PLUGIN_NAME, counter, section_url);
    counter
}

/// Fetch an article detail page and extract the main content div as HTML and Markdown.
fn populate_content_in_doc(
    doc: &mut Document,
    article_url: &str,
    client: &reqwest::blocking::Client,
    retry_times: usize,
    wait_secs: usize,
) {
    let raw_html = network::http_get(&article_url.to_string(), client, retry_times, wait_secs);
    if raw_html.is_empty() {
        info!("{}: Empty response for {}", PLUGIN_NAME, article_url);
        return;
    }

    let page = Html::parse_document(&raw_html);

    // Try these selectors in order; first one with substantial content wins
    for sel_str in &["div.content_area", "div#pnlDetails", "div.text1"] {
        if let Ok(sel) = Selector::parse(sel_str) {
            if let Some(div) = page.select(&sel).next() {
                let inner = div.inner_html();
                if inner.len() > 100 {
                    doc.html_content = inner.clone();
                    doc.text = html_to_markdown(&inner);
                    debug!("{}: Extracted {} bytes via '{}' from {}",
                        PLUGIN_NAME, inner.len(), sel_str, article_url);
                    return;
                }
            }
        }
    }

    // Last resort: convert full page
    doc.html_content = raw_html.clone();
    doc.text = extract_text_from_html(&raw_html);
    debug!("{}: Used full-page fallback for {}", PLUGIN_NAME, article_url);
}

/// Download an XLSX dataset file to master_data_folder with a _YYYY_MM_DD.xlsx suffix.
fn download_xls_file(
    xls_url: &str,
    date_str: &str,
    client: &reqwest::blocking::Client,
    master_data_folder: &str,
) {
    let date_part = date_str.replace('-', "_");

    let url_file = xls_url.rfind('/').map(|i| &xls_url[i + 1..]).unwrap_or("data");
    let stem = url_file.rfind('.').map(|i| &url_file[..i]).unwrap_or(url_file);
    let stem_short: String = stem.chars().take(48).collect();

    let filename = format!("{}_{}_{}.xlsx", PLUGIN_NAME, stem_short, date_part);
    let file_path = Path::new(master_data_folder).join(&filename);

    if file_path.exists() {
        debug!("{}: XLS already exists: {:?}", PLUGIN_NAME, file_path);
        return;
    }

    info!("{}: Downloading XLS: {}", PLUGIN_NAME, xls_url);
    let data = network::http_get_binary(&xls_url.to_string(), client);

    if data.is_empty() {
        error!("{}: Empty response for XLS: {}", PLUGIN_NAME, xls_url);
        return;
    }

    match File::create(&file_path) {
        Ok(mut f) => match f.write_all(&data) {
            Ok(_)  => info!("{}: Saved {} bytes to {:?}", PLUGIN_NAME, data.len(), file_path),
            Err(e) => error!("{}: Write XLS error: {}", PLUGIN_NAME, e),
        },
        Err(e) => error!("{}: Create XLS error {:?}: {}", PLUGIN_NAME, file_path, e),
    }
}

fn custom_data_processing(doc: &mut Document) {
    if doc.text.is_empty() && !doc.html_content.is_empty() {
        info!("{}: Extracting text from cached HTML.", PLUGIN_NAME);
        doc.text = extract_text_from_html(&doc.html_content);
    }
    if doc.recipients.len() > 2 {
        doc.recipients = clean_recepients(&doc.recipients);
    }
}

fn clean_recepients(recepients: &str) -> String {
    let re = Regex::new(
        r"([Dear ]*Madam[ ]*/[Dear ]*Sir|Dear Sir/|Dear Sir /|Madam / Dear Sir|Madam / Sir|Madam|Sir)"
    ).unwrap();
    if let Some(s) = re.split(recepients).next() {
        return s.trim().to_string();
    }
    recepients.to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // resolve_url

    #[test]
    fn test_resolve_url_already_absolute() {
        let r = resolve_url(
            "https://rbidocs.rbi.org.in/rdocs/PDFs/file.PDF",
            "https://www.rbi.org.in/Scripts/NotificationUser.aspx",
        );
        assert_eq!(r, "https://rbidocs.rbi.org.in/rdocs/PDFs/file.PDF");
    }

    #[test]
    fn test_resolve_url_root_relative() {
        let r = resolve_url(
            "/Scripts/NotificationUser.aspx?Id=1",
            "https://www.rbi.org.in/Scripts/NotificationUser.aspx",
        );
        assert_eq!(r, "https://www.rbi.org.in/Scripts/NotificationUser.aspx?Id=1");
    }

    #[test]
    fn test_resolve_url_same_dir_relative() {
        let r = resolve_url(
            "NotificationUser.aspx?Id=13459&Mode=0",
            "https://www.rbi.org.in/Scripts/NotificationUser.aspx",
        );
        assert_eq!(r, "https://www.rbi.org.in/Scripts/NotificationUser.aspx?Id=13459&Mode=0");
    }

    #[test]
    fn test_resolve_url_empty() {
        assert_eq!(resolve_url("", "https://www.rbi.org.in/Scripts/foo.aspx"), "");
    }

    // parse_rbi_date

    #[test]
    fn test_parse_date_notification_format() {
        assert_eq!(parse_rbi_date("May 18, 2026"), Some("2026-05-18".to_string()));
        assert_eq!(parse_rbi_date("May 21, 2026"), Some("2026-05-21".to_string()));
        assert_eq!(parse_rbi_date("January 1, 2026"), Some("2026-01-01".to_string()));
    }

    #[test]
    fn test_parse_date_wss_format() {
        assert_eq!(parse_rbi_date("15 May 2026"), Some("2026-05-15".to_string()));
        assert_eq!(parse_rbi_date("01 January 2026"), Some("2026-01-01".to_string()));
    }

    #[test]
    fn test_parse_date_invalid() {
        assert!(parse_rbi_date("").is_none());
        assert!(parse_rbi_date("not a date").is_none());
        assert!(parse_rbi_date("tableheader").is_none());
    }

    // html_to_markdown (via content_extraction)

    #[test]
    fn test_html_to_markdown_headings() {
        use crate::content_extraction::html_to_markdown;
        let html = "<h1>Title</h1><h2>Section</h2><p>Body text.</p>";
        let md = html_to_markdown(html);
        assert!(md.contains("# Title"), "got: {}", md);
        assert!(md.contains("## Section"), "got: {}", md);
        assert!(md.contains("Body text"), "got: {}", md);
    }

    #[test]
    fn test_html_to_markdown_bold_italic() {
        use crate::content_extraction::html_to_markdown;
        let html = "<p><b>Bold</b> and <i>italic</i> text.</p>";
        let md = html_to_markdown(html);
        assert!(md.contains("**Bold**"), "got: {}", md);
        assert!(md.contains("*italic*"), "got: {}", md);
    }

    #[test]
    fn test_html_to_markdown_links() {
        use crate::content_extraction::html_to_markdown;
        let html = r#"<p><a href="https://rbi.org.in">RBI</a></p>"#;
        let md = html_to_markdown(html);
        assert!(md.contains("[RBI](https://rbi.org.in)"), "got: {}", md);
    }

    #[test]
    fn test_html_to_markdown_list() {
        use crate::content_extraction::html_to_markdown;
        let html = "<ul><li>Item one</li><li>Item two</li></ul>";
        let md = html_to_markdown(html);
        assert!(md.contains("* Item one"), "got: {}", md);
        assert!(md.contains("* Item two"), "got: {}", md);
    }

    #[test]
    fn test_html_to_markdown_skips_scripts() {
        use crate::content_extraction::html_to_markdown;
        let html = "<script>alert('x')</script><p>Real content</p>";
        let md = html_to_markdown(html);
        assert!(!md.contains("alert"), "Script should be excluded, got: {}", md);
        assert!(md.contains("Real content"), "got: {}", md);
    }

    // get_docs_from_listing_page

    #[test]
    fn test_listing_page_date_only_no_docs() {
        let html = r#"<html><body>
            <table class="tablebg"><tr><td class="tableheader"><b>May 18, 2026</b></td></tr></table>
        </body></html>"#;
        let (tx, rx) = std::sync::mpsc::channel();
        let mut seen = HashSet::new();
        let netw = NetworkParameters {
            user_agent: "test".to_string(), retry_times: 1,
            wait_time_min: 0, wait_time_max: 0,
            fetch_timeout: 10, connect_timeout: 10,
            proxy_server: None, referrer_url: None,
        };
        let client = reqwest::blocking::Client::new();
        let n = get_docs_from_listing_page(
            html.to_string(), &tx,
            "https://www.rbi.org.in/Scripts/NotificationUser.aspx",
            "Notifications", &mut seen, &client, &netw,
            "/tmp", "/tmp", "/tmp",
        );
        drop(tx);
        assert_eq!(n, 0);
        assert_eq!(rx.try_iter().count(), 0);
    }

    #[test]
    fn test_listing_page_skips_already_retrieved() {
        let html = r#"<html><body>
            <table class="tablebg">
              <tr><td class="tableheader"><b>May 18, 2026</b></td></tr>
              <tr>
                <td><a class='link2' href='NotificationUser.aspx?Id=1&Mode=0'>Test</a></td>
                <td><a id='APDF_ABC' href='https://rbidocs.rbi.org.in/foo.PDF'></a></td>
              </tr>
            </table></body></html>"#;
        let (tx, rx) = std::sync::mpsc::channel();
        let mut seen = HashSet::new();
        seen.insert(
            "https://www.rbi.org.in/Scripts/NotificationUser.aspx?Id=1&Mode=0".to_string()
        );
        let netw = NetworkParameters {
            user_agent: "test".to_string(), retry_times: 1,
            wait_time_min: 0, wait_time_max: 0,
            fetch_timeout: 10, connect_timeout: 10,
            proxy_server: None, referrer_url: None,
        };
        let client = reqwest::blocking::Client::new();
        let n = get_docs_from_listing_page(
            html.to_string(), &tx,
            "https://www.rbi.org.in/Scripts/NotificationUser.aspx",
            "Notifications", &mut seen, &client, &netw,
            "/tmp", "/tmp", "/tmp",
        );
        drop(tx);
        assert_eq!(n, 0, "Already-retrieved URL should be skipped");
        assert_eq!(rx.try_iter().count(), 0);
    }

    #[test]
    fn test_clean_recepients_dear_madam_sir() {
        assert_eq!(
            clean_recepients("All SCBs ALL AIFs   Dear Madam/Sir,"),
            "All SCBs ALL AIFs"
        );
    }

    #[test]
    fn test_clean_recepients_madam_sir() {
        assert_eq!(
            clean_recepients("All SCBs ALL AIFs and Mad houses   Madam/Sir,"),
            "All SCBs ALL AIFs and Mad houses"
        );
    }
}
