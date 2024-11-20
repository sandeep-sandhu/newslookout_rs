// file: mod_en_in_rbi
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
use scraper::ElementRef;
use serde_json::{json, Value};
use {
    regex::Regex,
};

use crate::{document, network, utils};
use crate::document::{Document, new_document};
use crate::html_extract::{extract_text_from_html, extract_doc_from_row};
use crate::llm::check_and_split_text;
use crate::network::{read_network_parameters, make_http_client, NetworkParameters};
use crate::utils::{split_by_word_count, clean_text, get_data_folder, get_plugin_config, get_text_from_element, get_urls_from_database, make_unique_filename, to_local_datetime, get_database_filename, retrieve_pdf_content, extract_text_from_pdf};

pub(crate) const PLUGIN_NAME: &str = "mod_en_in_rbi";
const PUBLISHER_NAME: &str = "Reserve Bank of India";
const BASE_URL: &str = "https://website.rbi.org.in/";
const STARTER_URLS: [(&str,&str); 15] = [
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
    ("https://website.rbi.org.in/web/rbi/publications/publications-by-frequency", "Reports"),
    ("https://website.rbi.org.in/web/rbi/publications/reports/financial_stability_reports", "Reports"),
    ("https://website.rbi.org.in/web/rbi/publications/articles", "Articles"),
];


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

    match get_data_folder(&app_config).to_str(){
        Some(data_folder_name) => {

            let _count_docs = retrieve_data(
                STARTER_URLS,
                database_filename.as_str(),
                &client,
                tx,
                data_folder_name,
                netw_params.retry_times,
                netw_params.wait_time_min,
                maxitemsinpage,
                maxpages
            );

        },
        None => {
            error!("Unable to determine path to store data files.");
            panic!("Unable to determine path to store data files.");
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
    starter_urls: [(&str, &str); 15],
    database_filename: &str,
    client: &reqwest::blocking::Client,
    tx: Sender<document::Document>,
    data_folder: &str,
    retry_times: usize,
    wait_time:usize,
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
            let content = network::http_get(&listing_url_with_args, &client, retry_times, rng.gen_range(wait_time..=(wait_time*3)));
            let count_of_docs = get_docs_from_listing_page(
                content,
                &tx,
                &listing_url_with_args,
                section_name,
                &mut already_retrieved_urls,
                client,
                retry_times,
                wait_time,
                data_folder
            );
            counter += count_of_docs;

        }
    }
    return counter;
}


/// Extract content of the associated PDF file mentioned in the document's pdf_url attribute
/// Converts to text and saves it to the document 'text' attribute.
/// The PDF content is saved as a file in the data_folder.
///
/// # Arguments
///
/// * `input_doc`: The document to read
/// * `client`: The HTTP(S) client for fectching the web content via GET requests
/// * `data_folder`: The data folder where the PDF files are to be saved.
///
/// returns: ()
pub fn load_pdf_content(
    mut input_doc: &mut Document,
    client: &reqwest::blocking::Client,
    data_folder: &str)
{
    if input_doc.pdf_url.len() > 4 {
        let pdf_filename = make_unique_filename(&input_doc, "pdf");
        // save to file in data_folder, make full path by joining folder to unique filename
        let pdf_file_path = Path::new(data_folder).join(&pdf_filename);
        // check if pdf already exists, if so, do not retrieve again:
        if Path::exists(pdf_file_path.as_path()){
            info!("Not retrieving PDF since it already exists: {:?}", pdf_file_path);
            if input_doc.text.len() > 1 {
                let txt_filename = make_unique_filename(&input_doc, "txt");
                let txt_file_path = Path::new(data_folder).join(&txt_filename);
                input_doc.text = extract_text_from_pdf(pdf_file_path, txt_file_path);
            }
        }else {
            // get pdf content, and its plaintext output
            let (pdf_data, plaintext) = retrieve_pdf_content(&input_doc.pdf_url, client);
            input_doc.text = plaintext;
            debug!("From url {}: retrieved pdf file from link: {} of length {} bytes",
                input_doc.url, input_doc.pdf_url, pdf_data.len()
            );
            if pdf_data.len() > 1 {
                // persist to disk
                match File::create(&pdf_file_path) {
                    Ok(mut pdf_file) => {
                        debug!("Created pdf file: {:?}, now starting to write data for: '{}' ",
                            pdf_file_path, input_doc.title
                        );
                        match pdf_file.write_all(pdf_data.as_bytes()) {
                            Ok(_write_res) => info!("From url {} wrote {} bytes to file: {}",
                                input_doc.pdf_url, pdf_data.len(),
                                pdf_file_path.as_os_str().to_str().unwrap()),
                            Err(write_err) => error!("When writing PDF file to disk: {}",
                                write_err)
                        }
                    },
                    Err(file_err) => {
                        error!("When creating pdf file: {}", file_err);
                    }
                }
            }
        }
    }
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
    retry_times:usize,
    wait_time:usize,
    data_folder: &str) -> usize
{
    let mut counter: usize=0;
    let mut rng = rand::thread_rng();

    let rows_selector = scraper::Selector::parse("div.notifications-row-wrapper>div>div").unwrap();
    // to select div with class = "Notification-content-wrap"
    let whole_page_content_selector = scraper::Selector::parse("div.Notification-content-wrap").unwrap();

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

        debug!("{}: checking url: {} if already retrieved? {}",
            PLUGIN_NAME,
            this_new_doc.url,
            already_retrieved_urls.contains(this_new_doc.url.as_str())
        );
        if already_retrieved_urls.contains(&this_new_doc.url){
            info!("{}: Ignoring already retrieved url: {}", PLUGIN_NAME, this_new_doc.url);
            continue 'rows_loop;
        }

        // get content of web page:
        let html_content = network::http_get(
            &this_new_doc.url,
            &client,
            retry_times,
            rng.gen_range(wait_time..=(wait_time*3))
        );
        // from the entire page's html content, save only the specified div element
        let page_content = scraper::Html::parse_document(html_content.as_str());
        for page_div in page_content.select(&whole_page_content_selector){
            this_new_doc.html_content = page_div.html();
        }
        debug!("From listing page {}: got the document url {}, and its content of size: {}",
                    url_listing_page, this_new_doc.url, this_new_doc.html_content.len());

        _ = already_retrieved_urls.insert(this_new_doc.url.clone());

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

    doc_to_process.classification.insert("doc_type".to_string(), get_doc_type(doc_to_process.title.as_str()));

    check_and_split_text(doc_to_process);

}



fn get_doc_type(title: &str) -> String {
    let mut doctype: String = String::from("regulatory-notification");
    // TODO: mark doctype based on patterns:

    return doctype;
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
    use crate::plugins::mod_en_in_rbi;
    use crate::plugins::mod_en_in_rbi::clean_recepients;

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
