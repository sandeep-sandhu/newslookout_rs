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
            let content = get_sebi_urllist(&client, sid.to_string(), ssid.to_string(), smid.to_string(), section_name, pageno.to_string());
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

fn get_sebi_urllist(
    client: &Client,
    sid: String,
    ssid: String,
    smid: String,
    sub_section_name: &str,
    page_no: String
) -> String {

    let listing_url = "https://www.sebi.gov.in/sebiweb/ajax/home/getnewslistinfo.jsp";

    // Make payload
    let params = [
        ("nextValue","1"),
        ("next", "n"),
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
        ("doDirect", page_no.as_str()),
    ];

    // get response
    match client
        .post(listing_url)
        .json(&params)
        .send()
    {
        Ok(resp) => {
            match resp.text() {
                Ok(resptext) => return resptext,
                Err(_) => {}
            }
        }
        Err(e) => error!("{}: When getting list of urls: {}", PLUGIN_NAME, e)
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

    let rows_selector = scraper::Selector::parse("table.dataTable>tbody>tr").unwrap();
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

}
