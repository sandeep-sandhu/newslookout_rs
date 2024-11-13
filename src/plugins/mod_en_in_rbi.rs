// file: mod_en_in_rbi
// Purpose: Retrieve data published by RBI


use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::mpsc::Sender;

use config::{Config};
use chrono::{NaiveDate, Utc};
use log::{debug, error, info};
use nom::AsBytes;
use pdf_extract::extract_text_from_mem;
use rand::Rng;
use {
    regex::Regex,
};

use crate::{document, network};
use crate::document::{Document};
use crate::network::make_http_client;
use crate::utils::{extract_text_from_html, split_by_word_count, clean_text, get_data_folder, get_network_params, get_plugin_config, get_text_from_element, get_urls_from_database, make_unique_filename, to_local_datetime, get_database_filename};

pub(crate) const PLUGIN_NAME: &str = "mod_en_in_rbi";
const PUBLISHER_NAME: &str = "Reserve Bank of India";
const BASE_URL: &str = "https://website.rbi.org.in/";
const STARTER_URLS: [(&str,&str); 6] = [
    ("https://website.rbi.org.in/web/rbi/notifications/rbi-circulars", "Circular"),
    ("https://website.rbi.org.in/web/rbi/press-releases", "Press Release"),
    ("https://website.rbi.org.in/web/rbi/notifications/draft-notifications", "Draft Notifications"),
    ("https://website.rbi.org.in/web/rbi/notifications/master-directions", "Master Directions"),
    ("https://website.rbi.org.in/en/web/rbi/notifications/master-circulars", "Master Circulars"),
    ("https://website.rbi.org.in/web/rbi/notifications", "Notifications"),
];


pub(crate) fn run_worker_thread(tx: Sender<document::Document>, app_config: Config) {

    info!("{}: Getting configuration.", PLUGIN_NAME);
    let (fetch_timeout_seconds, retry_times, wait_time, user_agent, proxy_server) = get_network_params(&app_config);
    let client = make_http_client(fetch_timeout_seconds, user_agent.as_str(), BASE_URL.to_string(), proxy_server);
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

            let _count_docs = get_url_listing(
                STARTER_URLS,
                database_filename.as_str(),
                &client,
                tx,
                data_folder_name,
                retry_times,
                wait_time,
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
///
/// # Examples
///
/// ```
///
/// ```
fn get_url_listing(
    starter_urls: [(&str, &str); 6],
    database_filename: &str,
    client: &reqwest::blocking::Client,
    tx: Sender<document::Document>,
    data_folder: &str,
    retry_times: u64,
    wait_time:u64,
    maxitemsinpage: u64,
    max_pages: u64
) -> u32 {

    info!("Plugin {}: Getting url listing for ", PLUGIN_NAME);
    let mut already_retrieved_urls = get_urls_from_database(database_filename, PLUGIN_NAME);
    info!("For module {}: Got {} previously retrieved urls from table.", PLUGIN_NAME, already_retrieved_urls.len());

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
            let mut new_docs = get_docs_from_listing_page(
                content,
                &listing_url_with_args,
                section_name,
                &mut already_retrieved_urls,
                client,
                retry_times,
                wait_time,
                data_folder
            );

            for mut doc_to_process in new_docs {
                custom_data_processing(&mut doc_to_process);
                match tx.send(doc_to_process) {
                    Result::Ok(_res) => {
                        counter += 1;
                        debug!("Sent another document for processing, count so far = {}", counter);
                    },
                    Err(e) => error!("When sending document via channel: {}", e)
                }
            }
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
fn get_docs_from_listing_page(content: String, url_listing_page: &String, section_name: &str, already_retrieved_urls: &mut HashSet<String>, client: &reqwest::blocking::Client, retry_times:u64, wait_time:u64, data_folder: &str) -> Vec<document::Document>{

    let mut new_docs: Vec<document::Document> = Vec::new();
    let mut rng = rand::thread_rng();

    let rows_selector = scraper::Selector::parse("div.notifications-row-wrapper>div>div").unwrap();
    let alink_selector = scraper::Selector::parse("a.mtm_list_item_heading").unwrap();
    let date_selector = scraper::Selector::parse("div.notification-date>span").unwrap();
    let doctitle_selector = scraper::Selector::parse("span.mtm_list_item_heading").unwrap();
    let pdf_link_selector = scraper::Selector::parse("a.matomo_download").unwrap();
    let description_snippet_selector = scraper::Selector::parse("div.notifications-description p").unwrap();
    let whole_page_content_selector = scraper::Selector::parse("div.Notification-content-wrap").unwrap();

    // get url list:
    info!("{}: Retrieving url listing from: {}", PLUGIN_NAME, url_listing_page);
    // parse content using scraper
    let html_document = scraper::Html::parse_document(&content.as_str());

    'rows_loop: for row_each in html_document.select(&rows_selector){

        // prepare empty document:
        let mut this_new_doc = document::new_document();
        // set values
        this_new_doc.module = PLUGIN_NAME.to_string();
        this_new_doc.plugin_name = PUBLISHER_NAME.to_string();
        this_new_doc.section_name = section_name.to_string();
        this_new_doc.source_author = PUBLISHER_NAME.to_string();
        // init document with default "others" categories in classification field.
        this_new_doc.classification = HashMap::from( [
            ("channel".to_string(),"other".to_string()),
            ("customer_type".to_string(), "other".to_string()),
            ("function".to_string(),"other".to_string()),
            ("market_type".to_string(),"other".to_string()),
            ("occupation".to_string(),"other".to_string()),
            ("product_type".to_string(),"other".to_string()),
            ("risk_type".to_string(),"other".to_string()),
            // document type:
            ("doc_type".to_string(),"regulatory-notification".to_string()),
        ]);
        let mut date_str = String::from("");

        let snippet_regex: Regex = Regex::new(
            r"(RBI[/A-Z]+\d{4}-\d{2,4}/\d*)(.+\d{4}-\d{2,4}[ ]*)((January|February|March|April|May|June|July|August|September|October|November|December)[\d ]+,[\d ]+)(.+)(Madam|Madam[ ]*/[ ]*Dear Sir|Dear Sir/|Dear Sir /|Madam / Dear Sir|Madam / Sir|$)"
        ).unwrap();

        for alink_elem in row_each.select(&alink_selector) {
            if let Some(href) = alink_elem.value().attr("href") {
                this_new_doc.url = href.parse().unwrap();

                debug!("{}: checking url: {} if already retrieved? {}", PLUGIN_NAME, this_new_doc.url, already_retrieved_urls.contains(this_new_doc.url.as_str()));

                if already_retrieved_urls.contains(&this_new_doc.url){
                    info!("{}: Ignoring already retrieved url: {}", PLUGIN_NAME, this_new_doc.url);
                    continue 'rows_loop;
                }

                // get content:
                let html_content = network::http_get(&this_new_doc.url, &client, retry_times, rng.gen_range(wait_time..=(wait_time*3)));

                // from the entire page's html content, save only the div element with class = "Notification-content-wrap"
                let page_content = scraper::Html::parse_document(html_content.as_str());
                for page_div in page_content.select(&whole_page_content_selector){
                    this_new_doc.html_content = page_div.html();
                }

                debug!("From listing page {}: got the document url {}, and its content of size: {}",
                    url_listing_page, this_new_doc.url, this_new_doc.html_content.len());

                _ = already_retrieved_urls.insert(this_new_doc.url.clone());

                debug!("count of urls in set after adding url: {}", already_retrieved_urls.len());
            }
        }

        for date_div_elem in row_each.select(&date_selector) {
            date_str = clean_text(date_div_elem.inner_html());
            match NaiveDate::parse_from_str(date_str.as_str(), "%b %d, %Y"){
                Ok(naive_date) => {
                    this_new_doc.publish_date_ms = to_local_datetime(naive_date).timestamp();
                    this_new_doc.publish_date = naive_date.format("%Y-%m-%d").to_string();
                },
                Err(date_err) => {
                    error!("Could not parse date '{}', error: {}", date_str.as_str(), date_err)
                }
            }
            debug!("From url {} , identified date = {}, timestamp = {}", this_new_doc.url, date_str, this_new_doc.publish_date_ms);
        }

        for title_span_elem in row_each.select(&doctitle_selector) {
            this_new_doc.title = clean_text(get_text_from_element(title_span_elem));
            this_new_doc.links_inward = vec![url_listing_page.clone()];
            debug!("{}: Identified title: '{}' for url {}", PLUGIN_NAME, this_new_doc.title, this_new_doc.url);
        }

        let mut snippet_text = String::from(" ");
        for snippet_elem in row_each.select(&description_snippet_selector) {
            let description_snippet = clean_text(
                get_text_from_element(snippet_elem)
                )
                .replace("\r\n", " ")
                .replace("\n", " ");
            snippet_text.push_str(" ");
            snippet_text.push_str(description_snippet.as_str());
            debug!(" ---Decoding Snippet---: {}", snippet_text);
            if let Some(caps) = snippet_regex.captures(snippet_text.as_str()) {
                let id_prefix = caps.get(1).unwrap().as_str();
                this_new_doc.unique_id = clean_text(caps.get(2).unwrap().as_str().to_string());
                let pubdate_longformat_str = caps.get(3).unwrap().as_str();
                this_new_doc.recipients = caps.get(5).unwrap().as_str().to_string();
                debug!("\tid_prefix: {},\n unique_id: {},\n pubdate_longformat_str: {},\n recipients: {}",
                    id_prefix, this_new_doc.unique_id, pubdate_longformat_str, this_new_doc.recipients);
            }
        }

        for pdf_url_elem in row_each.select(&pdf_link_selector) {
            if let Some(href) = pdf_url_elem.value().attr("href") {
                this_new_doc.pdf_url = href.parse().unwrap();

                // get pdf content,
                let pdf_data = network::http_get_binary(&this_new_doc.pdf_url, client);
                debug!("From url {}: retrieved pdf file from link: {} of length {} bytes", this_new_doc.url, this_new_doc.pdf_url, pdf_data.len());

                if pdf_data.len() > 1 {
                    let pdf_filename = make_unique_filename(&this_new_doc, "pdf");
                    // save to file in data_folder, make full path by joining folder to unique filename
                    let pdf_file_path = Path::new(data_folder).join(&pdf_filename);
                    // persist to disk
                    match File::create(&pdf_file_path) {
                        Ok(mut pdf_file) => {
                            debug!("Created pdf file: {:?}, now starting to write data for: '{}' ", pdf_file_path, this_new_doc.title);
                            match pdf_file.write_all(pdf_data.as_bytes()) {
                                Ok(_write_res) => info!("From url {}, retrieved pdf file {} and wrote {} bytes to file: {}", this_new_doc.url, this_new_doc.pdf_url, pdf_data.len(), pdf_file_path.as_os_str().to_str().unwrap()),
                                Err(write_err) => error!("When writing PDF file to disk: {}", write_err)
                            }
                        },
                        Err(file_err) => {
                            error!("When creating pdf file: {}", file_err);
                        }
                    }
                    // convert to text, populate text field
                    match extract_text_from_mem(pdf_data.as_bytes()) {
                        Result::Ok(plaintext) => {
                            this_new_doc.text = plaintext;
                        },
                        Err(outerr) => {
                            error!("When converting pdf content into text: {}", outerr);
                        }
                    }
                }
            }
        }

        new_docs.push(this_new_doc);
    }
    return new_docs;
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

    info!("{}: Splitting document '{}' into parts.", PLUGIN_NAME, doc_to_process.title);
    let mut text_part_counter: usize = 1;
    for text_part in split_by_word_count(doc_to_process.text.as_str(), 600, 15){
        doc_to_process.text_parts.push(HashMap::from([("id".to_string(), text_part_counter.to_string()), ("text".to_string(),text_part)]));
        text_part_counter += 1;
    }
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
