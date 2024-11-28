// file: rbi
// Purpose: Retrieve data published by RBI


use std::collections::{HashMap, HashSet};
use std::fs;
use std::fs::File;
use std::io::{BufWriter, Bytes, Read, Write};
use std::path::Path;
use std::sync::mpsc::Sender;
use std::borrow::BorrowMut;

use config::{Config};
use chrono::{NaiveDate, Utc};
use log::{debug, error, info};
use nom::AsBytes;
use pdf_extract::extract_text_from_mem;
use rand::Rng;
use reqwest;
use scraper::ElementRef;
use serde_json::{json, Value};
use {
    regex::Regex,
};

use crate::{document, network, utils};
use crate::document::{Document, new_document};
use crate::cfg::{get_plugin_config, get_data_folder, get_database_filename};
use crate::html_extract::{extract_text_from_html, extract_doc_from_row};
use crate::network::{read_network_parameters, make_http_client, NetworkParameters};
use crate::utils::{clean_text, get_text_from_element, get_urls_from_database, make_unique_filename, to_local_datetime, retrieve_pdf_content, extract_text_from_pdf, load_pdf_content, check_and_fix_url};

pub(crate) const PLUGIN_NAME: &str = "rbi";
const PUBLISHER_NAME: &str = "Reserve Bank of India";
const BASE_URL: &str = "https://website.rbi.org.in/";



/// Executes this function of the module in the separate thread launched by the pipeline/queue module
///
/// # Arguments
///
/// * `tx`: The channel to transmit newly identified or web scraped documents
/// * `app_config`: The application configuration object to be used to get various config parameters
///
/// returns: ()
pub(crate) fn run_worker_thread(tx: Sender<document::Document>, app_config: Config) {

    info!("{}: Reading plugin specific configuration.", PLUGIN_NAME);
    let mut netw_params = read_network_parameters(&app_config);
    netw_params.referrer_url = Some(BASE_URL.to_string());
    let client = make_http_client(&netw_params);
    let database_filename = get_database_filename(&app_config);

    let mut maxitemsinpage = 1;
    let mut maxpages = 1;
    match get_plugin_config(&app_config, PLUGIN_NAME, "maxpages"){
        Some(maxpages_str) => {
            match maxpages_str.parse::<u64>(){
                Result::Ok(configintvalue) => maxpages =configintvalue, Err(e)=>{}
            }
        }, None => {}
    };
    match get_plugin_config(&app_config, PLUGIN_NAME, "items_per_page"){
        Some(maxitemsinpage_str) => {
            match maxitemsinpage_str.parse::<u64>(){
                Result::Ok(configintvalue) => maxitemsinpage =configintvalue, Err(e)=>{}
            }
        }, None => {}
    };
    info!("{} using parameters: maxitemsinpage={}, maxpages={}", PLUGIN_NAME, maxitemsinpage, maxpages);

    let starter_urls: Vec<(&str, &str)> = vec![
        ("https://website.rbi.org.in/web/rbi/notifications/rbi-circulars", "Circular"),
        ("https://website.rbi.org.in/web/rbi/press-releases", "Press Release"),
        ("https://website.rbi.org.in/web/rbi/notifications/draft-notifications", "Draft Notifications"),
        ("https://website.rbi.org.in/web/rbi/notifications/master-directions", "Master Directions"),
        ("https://website.rbi.org.in/en/web/rbi/notifications/master-circulars", "Master Circulars"),
        ("https://website.rbi.org.in/web/rbi/notifications", "Notifications"),
        ("https://website.rbi.org.in/web/rbi/about-us/legal-framework/act", "Acts"),
        ("https://website.rbi.org.in/web/rbi/about-us/legal-framework/rules", "Rules"),
        ("https://website.rbi.org.in/web/rbi/about-us/legal-framework/regulations", "Regulations"),
        ("https://website.rbi.org.in/web/rbi/about-us/legal-framework/schemes", "Schemes"),
        ("https://website.rbi.org.in/web/rbi/speeches", "Speeches"),
        ("https://website.rbi.org.in/web/rbi/interviews", "Interviews and Media Interactions"),
        ("https://website.rbi.org.in/web/rbi/publications/reports/reports_list", "Reports"),
        ("https://website.rbi.org.in/web/rbi/publications/rbi-bulletin", "Bulletin"),
        ("https://website.rbi.org.in/web/rbi/publications/reports/financial_stability_reports", "Reports"),
        ("https://website.rbi.org.in/web/rbi/publications/chapters?category=24927745", "Report on Currency and Finance"),
        ("https://website.rbi.org.in/web/rbi/publications/articles?category=24927873", "Monetary Policy Report"),
    ];

    match get_data_folder(&app_config).to_str(){
        Some(data_folder_name) => {
            let _count_docs = retrieve_data(
                starter_urls,
                database_filename.as_str(),
                &client,
                tx,
                data_folder_name,
                netw_params,
                maxitemsinpage,
                maxpages
            );
        },
        None => {
            error!("Unable to determine path to store data files.");
        }
    };
}

/// Retrieves the documents published on the starter urls.
///
/// # Arguments
///
/// * `starter_urls`: The url for each section of the site where articles or documents are listed
/// * `already_retrieved_urls`: The set of urls already retrieved earlier, so these are excluded
/// * `client`: The HTTP client used for this thread
/// * `tx`:
/// * `data_folder`:
/// * `retry_times`:
/// * `wait_time`:
/// * `maxitemsinpage`:
/// * `max_pages`:
///
/// returns: u32
fn retrieve_data(
    starter_urls: Vec<(&str, &str)>,
    database_filename: &str,
    client: &reqwest::blocking::Client,
    tx: Sender<document::Document>,
    data_folder: &str,
    netw_params: NetworkParameters,
    maxitemsinpage: u64,
    max_pages: u64
) -> usize {

    debug!("Plugin {}: Getting url listing for already retrieved urls", PLUGIN_NAME);
    let mut already_retrieved_urls = get_urls_from_database(database_filename, PLUGIN_NAME);
    info!("For Plugin {}: Got {} previously retrieved urls from table.", PLUGIN_NAME, already_retrieved_urls.len());

    let mut rng = rand::thread_rng();
    let mut counter = 0;

    for (starter_url, section_name) in starter_urls {
        // loop through all pages
        for pageno in 1..(max_pages+1){
            // create the url using template
            let urlargs = format!("?delta={}&start={}", maxitemsinpage, pageno);
            let mut listing_url_with_args = String::from(starter_url);
            listing_url_with_args.push_str(&urlargs);

            // retrieve content from this url and extract vector of documents, mainly individual urls to retrieve.
            let content = network::http_get(&listing_url_with_args, &client, (&netw_params).retry_times, rng.gen_range((&netw_params).wait_time_min..=((&netw_params).wait_time_min*3)));
            let count_of_docs = get_docs_from_listing_page(
                content,
                &tx,
                &listing_url_with_args,
                section_name,
                &mut already_retrieved_urls,
                client,
                &netw_params,
                data_folder
            );
            counter += count_of_docs;

        }
    }
    return counter;
}


/// Retrieve documents from the page that lists multiple documents, extract relevant content and
/// return back vector of documents.
///
/// # Arguments
///
/// * `url_listing_page`:
/// * `section_name`: Name of this section of the website
/// * `already_retrieved_urls`: Set of URLs already retrieved, to be checked before getting new urls
/// * `client`: HTTP client to use for retrieving the documents
/// * `retry_times`: Number of times to retry upon failure
/// * `wait_time`: Number of seconds to wait before trying HTTP get
/// * `data_folder`: Data folder to save the binary content associated with this url (e.g. PDF file)
///
/// returns: Vec<Document, Global>
///
/// # Examples
///
///
/// let new_docs = get_docs_from_listing_page(
///                 "https://www.website.com/section1/index.html",
///                 "section1",
///                 already_retrieved_urls_set,
///                 http_client,
///                 3,
///                 10,
///                 "/var/cache/newslookout"
///             );
///
pub fn get_docs_from_listing_page(
    content: String,
    tx: &Sender<document::Document>,
    url_listing_page: &String,
    section_name: &str,
    already_retrieved_urls: &mut HashSet<String>,
    client: &reqwest::blocking::Client,
    netw_params: &NetworkParameters,
    data_folder: &str) -> usize
{
    let mut counter: usize=0;

    let rows_selector = scraper::Selector::parse("div.notifications-row-wrapper>div>div").unwrap();

    // get url list:
    info!("{}: Retrieving url listing from: {}", PLUGIN_NAME, url_listing_page);

    // parse content using scraper
    let html_document = scraper::Html::parse_document(&content.as_str());

    'rows_loop: for row_each in html_document.select(&rows_selector){
        //Create document from row contents:
        let mut this_new_doc = extract_doc_from_row(row_each, url_listing_page);
        // set module specific values:
        this_new_doc.module = PLUGIN_NAME.to_string();
        this_new_doc.plugin_name = PUBLISHER_NAME.to_string();
        this_new_doc.section_name = section_name.to_string();
        this_new_doc.source_author = PUBLISHER_NAME.to_string();
        this_new_doc.data_proc_flags = document::DATA_PROC_CLASSIFY_INDUSTRY |
            document::DATA_PROC_CLASSIFY_MARKET | document::DATA_PROC_CLASSIFY_PRODUCT |
            document::DATA_PROC_EXTRACT_NAME_ENTITY | document::DATA_PROC_SUMMARIZE |
            document::DATA_PROC_EXTRACT_ACTIONS;

        // check if url was already retrieved?
        if already_retrieved_urls.contains(&this_new_doc.url){
            info!("{}: Ignoring already retrieved url: {}", PLUGIN_NAME, this_new_doc.url);
            continue 'rows_loop;
        }

        // check url is valid before retrieving it:
        if let Some(proper_url) = check_and_fix_url(this_new_doc.url.as_str(), BASE_URL){
            this_new_doc.url = proper_url;
        }else{
            // ignore this url:
            info!("{}: Ignoring invalid url: {}", PLUGIN_NAME, this_new_doc.url);
            continue 'rows_loop;
        }

        populate_content_in_doc(&mut this_new_doc, client, netw_params);

        _ = already_retrieved_urls.insert(this_new_doc.url.clone());
        debug!("From listing page {}: got the document url {}, and its content of size: {}",
                    url_listing_page, this_new_doc.url, this_new_doc.html_content.len());

        // make full path by joining folder to unique filename
        let filename = make_unique_filename(&this_new_doc, "json");
        let json_file_path = Path::new(data_folder).join(filename);
        this_new_doc.filename = String::from(
            json_file_path.as_path().to_str().expect("Not able to convert path to string")
        );

        load_pdf_content(&mut this_new_doc, &client, data_folder);

        // perform any custom processing of the data in the document:
        custom_data_processing(&mut this_new_doc);

        // send to queue for data processing pipeline:
        match tx.send(this_new_doc) {
            Result::Ok(_res) => {
                counter += 1;
                debug!("Sent another document for processing, count so far = {}", counter);
            },
            Err(e) => error!("When sending document via channel: {}", e)
        }
    }
    return counter;
}

/// Retrieves via HTTP GET the contents of this document from the url field of the document struct
/// Populates the content and other attributes specific to this page
///
/// # Arguments
///
/// * `this_new_doc`: The document to retrieve content for, updates this document
/// * `client`: The HTTP client to use for network retrieval via HTTP(S) GET protocol
/// * `netw_params`: The network parameters structure to be used for network fetch
///
/// returns: ()
fn populate_content_in_doc(this_new_doc: &mut Document, client: &reqwest::blocking::Client, netw_params: &NetworkParameters) {

    let mut rng = rand::thread_rng();

    // to select div with class = "Notification-content-wrap"
    let whole_page_content_selector = scraper::Selector::parse("div.Notification-content-wrap").unwrap();

    // get content of web page:
    let html_content = network::http_get(
        &this_new_doc.url,
        &client,
        netw_params.retry_times,
        rng.gen_range(netw_params.wait_time_min..=(netw_params.wait_time_max*3))
    );

    // TODO: apply logic for press release introducing new notification
    // in that case, move html_content into referrer_text, and download the new url identified
    // from the referrer text, replace url attribute with new url

    // from the entire page's html content, save only the specified div element:
    let page_content = scraper::Html::parse_document(html_content.as_str());
    for page_div in page_content.select(&whole_page_content_selector){
        this_new_doc.html_content = page_div.html();
    }

}

fn custom_data_processing(doc_to_process: &mut document::Document){

    // check if text content is available, if not, then extract from html
    if doc_to_process.text.len() < 1 {
        info!("{}: Extracting text from HTML content.", PLUGIN_NAME);
        doc_to_process.text = extract_text_from_html(doc_to_process.html_content.as_str())
    }

    // clean-up recipient text at boundary - dear madam/sir, etc.
    if doc_to_process.recipients.len() > 2 {
        // overwrite field with cleaned value
        doc_to_process.recipients = clean_recepients(doc_to_process.recipients.as_str());
    }


}


fn clean_recepients(recepients: &str) -> String{
    let letter_greeting_regex: Regex = Regex::new(
        r"([Dear ]*Madam[ ]*/[Dear ]*Sir|Dear Sir/|Dear Sir /|Madam / Dear Sir|Madam / Sir|Madam|Sir)"
    ).unwrap();
    // locate letter greeting regex, split text on this regex
    // remove part matching regex and after that
    for substr in letter_greeting_regex.split(recepients){
        return substr.trim().to_string();
    }
    return recepients.to_string();
}

#[cfg(test)]
mod tests {
    use crate::plugins::rbi;
    use crate::plugins::rbi::clean_recepients;

    #[test]
    fn test_clean_recepients() {
        let example_1 = "All SCBs ALL AIFs   Dear Madam/Sir,";
        let expected_result_1 = "All SCBs ALL AIFs";
        assert_eq!(clean_recepients(example_1), expected_result_1, "Recipient text not cleaned correctly");
        //
        let example_2 = "All SCBs ALL AIFs and Mad houses   Madam/Sir,";
        let expected_result_2 = "All SCBs ALL AIFs and Mad houses";
        assert_eq!(clean_recepients(example_2), expected_result_2, "Recipient text not cleaned correctly");
    }
}
