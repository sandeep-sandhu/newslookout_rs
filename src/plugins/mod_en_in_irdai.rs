// file: mod_en_in_irdai
// Purpose: Retrieve data published by IRDAI (Insurance Regulatory and Development Authority of India)

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::panic;
use std::panic::AssertUnwindSafe;
use chrono::NaiveDate;
use log::{error, info, warn, debug};
use rand::{Rng, RngExt};
use regex::Regex;
use scraper::ElementRef;
use crate::{document, get_plugin_cfg};
use crate::cfg::{get_data_folder, get_database_filename, get_pdf_data_folder};
use crate::document::Document;
use crate::network::{NetworkParameters, http_get, make_http_client, read_network_parameters};
use crate::utils::{check_and_fix_url, clean_text, get_text_from_element, get_urls_from_database, load_pdf_content, make_unique_filename, to_local_datetime};

pub(crate) const PLUGIN_NAME: &str = "mod_en_in_irdai";
const PUBLISHER_NAME: &str = "Insurance Regulatory and Development Authority of India";
const BASE_URL: &str = "https://irdai.gov.in/";


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

    let mut rng = rand::rng();

    let mut max_pages: u64 = 1;
    let mut maxitemsinpage: u64 = 10;

    match get_plugin_cfg!(PLUGIN_NAME, "max_pages", &app_config) {
        Some(max_pages_str) => {
            match max_pages_str.parse::<u64>() {
                Ok(val) => max_pages = val,
                Err(e) => error!("{}: Could not parse max_pages: {}", PLUGIN_NAME, e),
            }
        },
        None => {}
    };
    match get_plugin_cfg!(PLUGIN_NAME, "items_per_page", &app_config) {
        Some(items_str) => {
            match items_str.parse::<u64>() {
                Ok(val) => maxitemsinpage = val,
                Err(e) => error!("{}: Could not parse items_per_page: {}", PLUGIN_NAME, e),
            }
        },
        None => {}
    };

    let listing_urls = vec![
        ("https://irdai.gov.in/circulars", "Circular"),
        ("https://irdai.gov.in/notifications", "Notifications"),
        ("https://irdai.gov.in/guidelines", "Guidelines"),
        ("https://irdai.gov.in/exposure-drafts", "Exposure Draft"),
        ("https://irdai.gov.in/updated-regulations", "Updated Regulations"),
        ("https://irdai.gov.in/consolidated-gazette-notified-regulations", "Gazette Notified Regulations"),
        ("https://irdai.gov.in/rules", "Rules"),
        ("https://irdai.gov.in/acts", "Acts"),
        ("https://irdai.gov.in/warnings-and-penalties", "Enforcement Actions"),
        ("https://irdai.gov.in/orders1", "Orders"),
        ("https://irdai.gov.in/web/guest/whats-new", "Whats New section"),
    ];

    for (starter_url, section_name) in listing_urls {

        info!("{}: identifying listing from section: {}", PLUGIN_NAME, section_name);

        for pageno in 1..(max_pages + 1) {
            let urlargs = format!(
                "?p_p_id=com_irdai_document_media_IRDAIDocumentMediaPortlet&p_p_lifecycle=0&p_p_state=normal&p_p_mode=view&_com_irdai_document_media_IRDAIDocumentMediaPortlet_delta={delta}&_com_irdai_document_media_IRDAIDocumentMediaPortlet_resetCur=false&_com_irdai_document_media_IRDAIDocumentMediaPortlet_cur={pageno}",
                delta = maxitemsinpage,
                pageno = pageno
            );

            let mut listing_url_with_args = String::from(starter_url);
            listing_url_with_args.push_str(&urlargs);

            // retrieve content from this url and extract vector of documents
            let content = http_get(
                &listing_url_with_args,
                &client,
                (&netw_params).retry_times,
                rng.random_range((&netw_params).wait_time_min..=((&netw_params).wait_time_min * 3))
            );

            let count_of_docs = get_docs_from_listing_page(
                content,
                &tx,
                &listing_url_with_args,
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

/// Retrieve documents from the page that lists multiple documents, extract relevant content and
/// return count of documents sent.
///
/// # Arguments
///
/// * `content`: HTML content of the listing page
/// * `tx`: Channel sender for dispatching documents
/// * `url_listing_page`: The URL of the listing page
/// * `section_name`: Name of this section of the website
/// * `already_retrieved_urls`: Set of URLs already retrieved, to be checked before getting new urls
/// * `client`: HTTP client to use for retrieving the documents
/// * `netw_params`: Network parameters for HTTP requests
/// * `data_folder`: Data folder to save the binary content associated with this url (e.g. PDF file)
///
/// returns: usize
pub fn get_docs_from_listing_page(
    content: String,
    tx: &Sender<document::Document>,
    url_listing_page: &String,
    section_name: &str,
    already_retrieved_urls: &mut HashSet<String>,
    client: &reqwest::blocking::Client,
    netw_params: &NetworkParameters,
    data_folder: &str,
    pdf_folder: &str) -> usize
{
    let mut counter: usize = 0;

    info!("{}: Retrieving url listing from: {}", PLUGIN_NAME, url_listing_page);
    let html_document = scraper::Html::parse_document(&content.as_str());

    let rows_selector = scraper::Selector::parse("tbody.table-data>tr").unwrap();

    'rows_loop: for row_each in html_document.select(&rows_selector) {
        let mut this_new_doc = extract_docinfo_from_row(row_each, url_listing_page);
        this_new_doc.module = PLUGIN_NAME.to_string();
        this_new_doc.plugin_name = PUBLISHER_NAME.to_string();
        this_new_doc.section_name = section_name.to_string();
        this_new_doc.source_author = PUBLISHER_NAME.to_string();
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

        populate_content_in_doc(&mut this_new_doc, client, netw_params);

        _ = already_retrieved_urls.insert(this_new_doc.url.clone());
        let filename = make_unique_filename(&this_new_doc, "json");
        let json_file_path = Path::new(data_folder).join(filename);
        this_new_doc.filename = String::from(
            json_file_path.as_path().to_str().expect("Not able to convert path to string")
        );

        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            load_pdf_content(&mut this_new_doc, &client, pdf_folder);
        }));
        if result.is_err() {
            if let Err(errvar) = result {
                error!("{}: When reading PDF of document '{}' the error was: {:?}", PLUGIN_NAME, this_new_doc.title, errvar);
            }
        }

        if this_new_doc.recipients.len() > 2 {
            this_new_doc.recipients = clean_recepients(this_new_doc.recipients.as_str());
        }

        match tx.send(this_new_doc) {
            Result::Ok(_res) => {
                counter += 1;
            },
            Err(e) => error!("{}: When sending document via channel: {}", PLUGIN_NAME, e)
        }
    }
    return counter;
}

fn extract_docinfo_from_row(row_each: ElementRef, source_url: &String) -> Document {
    let mut this_new_doc = Document::default();

    // Init document with default "others" categories in classification field.
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

    let alink_selector = scraper::Selector::parse("td.table-col-subTitle>a").unwrap();
    let date_selector = scraper::Selector::parse("td.table-col-lastUpdated").unwrap();
    let pdf_link_selector = scraper::Selector::parse("div.doc-download>a").unwrap();
    let doctitle_selector = scraper::Selector::parse("td.table-col-shortDesc").unwrap();
    let unique_id_selector = scraper::Selector::parse("td.table-col-referenceNumber").unwrap();

    for date_div_elem in row_each.select(&date_selector) {
        let date_str = clean_text(date_div_elem.inner_html());
        // IRDAI listing rows frequently use placeholders ("--", "", "N/A") when no date is
        // published yet. Treat these as "date not available" and skip silently rather than
        // emitting an error for every such row.
        let trimmed = date_str.trim();
        if trimmed.is_empty() || trimmed.chars().all(|c| !c.is_ascii_digit()) {
            debug!("{}: No usable publish date in row (value: '{}'), leaving date unset.", PLUGIN_NAME, trimmed);
            continue;
        }
        match NaiveDate::parse_from_str(trimmed, "%d-%m-%Y") {
            Ok(naive_date) => {
                this_new_doc.publish_date_ms = to_local_datetime(naive_date).timestamp();
                this_new_doc.publish_date = naive_date.format("%Y-%m-%d").to_string();
            },
            Err(date_err) => {
                warn!("{}: Could not parse date '{}', error: {}", PLUGIN_NAME, trimmed, date_err)
            }
        }
    }

    for uniqueid_elem in row_each.select(&unique_id_selector) {
        this_new_doc.unique_id = clean_text(get_text_from_element(uniqueid_elem));
    }

    // get url:
    for alink_elem in row_each.select(&alink_selector) {
        if let Some(href) = alink_elem.value().attr("href") {
            this_new_doc.url = href.parse().unwrap();
        }
    }

    for pdf_url_elem in row_each.select(&pdf_link_selector) {
        if let Some(href) = pdf_url_elem.value().attr("href") {
            this_new_doc.pdf_url = href.parse().unwrap();
        }
    }

    this_new_doc.links_inward = vec![source_url.to_string()];

    for title_span_elem in row_each.select(&doctitle_selector) {
        this_new_doc.title = clean_text(get_text_from_element(title_span_elem));
    }

    return this_new_doc;
}

/// Retrieves via HTTP GET the contents of this document from the url field of the document struct.
/// Populates the html_content and other attributes specific to this page.
///
/// # Arguments
///
/// * `this_new_doc`: The document to retrieve content for, updates this document
/// * `client`: The HTTP client to use for network retrieval via HTTP(S) GET protocol
/// * `netw_params`: The network parameters structure to be used for network fetch
///
/// returns: ()
fn populate_content_in_doc(this_new_doc: &mut Document, client: &reqwest::blocking::Client, netw_params: &NetworkParameters) {
    let mut rng = rand::rng();

    // to select div with class = "Notification-content-wrap"
    let whole_page_content_selector = scraper::Selector::parse("div.Notification-content-wrap").unwrap();

    // get content of web page:
    let html_content = http_get(
        &this_new_doc.url,
        &client,
        netw_params.retry_times,
        rng.random_range(netw_params.wait_time_min..=(netw_params.wait_time_max * 3))
    );

    // from the entire page's html content, save only the specified div element:
    let page_content = scraper::Html::parse_document(html_content.as_str());
    for page_div in page_content.select(&whole_page_content_selector) {
        this_new_doc.html_content = page_div.html();
    }
}

fn clean_recepients(recepients: &str) -> String {
    let letter_greeting_regex: Regex = Regex::new(
        r"([Dear ]*Madam[ ]*/[Dear ]*Sir|Dear Sir/|Dear Sir /|Madam / Dear Sir|Madam / Sir|Madam|Sir)"
    ).unwrap();
    // locate letter greeting regex, split text on this regex,
    // return only the first part (before any greeting)
    if let Some(substr) = letter_greeting_regex.split(recepients).next() {
        return substr.trim().to_string();
    }
    recepients.to_string()
}
