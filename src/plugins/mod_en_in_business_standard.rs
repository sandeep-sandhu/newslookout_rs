// file: mod_en_in_business_standard
// Purpose:

use std::collections::HashMap;
use {
    regex::Regex,
};
use std::sync::mpsc::Sender;
use config::Config;
use chrono::Utc;
use log::{debug, error, info};
use reqwest::blocking::Client;

use crate::{document, network};
use crate::document::Document;
use crate::network::{make_http_client, read_network_parameters};
use crate::utils::{get_urls_from_database};
use crate::cfg::{get_data_folder, get_database_filename};

pub(crate) const PLUGIN_NAME: &str = "mod_en_in_business_std";
const PUBLISHER_NAME: &str = "Business Standard";
const BASE_URL: &str = "https://www.business-standard.com/";
const STARTER_URLS: [(&str, &str); 1] = [
    ("https://www.business-standard.com/", "main"),
];


/// Executes this function of the web-retriever module in the separate thread launched by
/// the pipeline/queue module
///
/// # Arguments
///
/// * `tx`: The channel to transmit newly identified or web scraped documents
/// * `app_config`: The application configuration object to be used to get various config parameters
///
/// returns: ()
pub(crate) fn run_worker_thread(tx: Sender<document::Document>, app_config: Config) {

    info!("{}: Starting worker", PLUGIN_NAME);

    let network_params = read_network_parameters(&app_config);
    let client = make_http_client(&network_params);

    let database_filename = get_database_filename(&app_config);
    let mut already_retrieved_urls = get_urls_from_database(database_filename.as_str(), PLUGIN_NAME);

    match get_data_folder(&app_config).to_str(){
        Some(data_folder_name) => {

            let mut new_docs: Vec<Document> = get_url_listing(
                STARTER_URLS,
                &client,
                data_folder_name,
                network_params.retry_times,
                network_params.wait_time_min
            );

            for mut doc_to_process in new_docs {
                custom_data_processing(&mut doc_to_process);
                tx.send(doc_to_process).unwrap();
            }

        },
        None => {
            error!("Unable to determine path to store data files.");
            panic!("Unable to determine path to store data files.");
        }
    };
}

fn get_url_listing(
    starter_urls: [(&str, &str); 1],
    client: &reqwest::blocking::Client,
    data_folder: &str,
    retry_times: usize,
    wait_time: usize
) -> Vec<Document>{

    info!("Plugin {}: Getting url listing for {}", PLUGIN_NAME, PUBLISHER_NAME);

    let maxitemsinpage: u16 = 10;
    let pageno: u16 = 1;

    let mut all_docs_from_plugin: Vec<Document> = Vec::new();

    for (starter_url, section_name) in starter_urls {

        let urlargs = format!("?delta={}&start={}", maxitemsinpage, pageno);
        let mut listing_url_with_args = String::from(starter_url);
        listing_url_with_args.push_str(&urlargs);

        // retrieve content from this url and extract vector of documents, mainly individual urls to retrieve.
        let mut new_docs = get_docs_from_listing_page(
            &listing_url_with_args,
            section_name,
            client,
            retry_times,
            wait_time,
            data_folder
        );

        all_docs_from_plugin.append(&mut new_docs);
    }
    return all_docs_from_plugin;
}

fn get_docs_from_listing_page(url_listing_page: &String, section_name: &str, client: &reqwest::blocking::Client, retry_times:usize, wait_time:usize, data_folder: &str) -> Vec<document::Document>{

    let mut new_docs: Vec<document::Document> = Vec::new();

    // TODO: implement this
    // get all links from url_listing_page
    // for each link:
    // if absolute, then filter links by domain
    // if relative, make these links absolute
    // check against urls already retrieved
    // fetch url content
    // put content into document
    // add to vector

    return new_docs;
}

fn custom_data_processing(mydoc: &mut document::Document){

    info!("{}: processing url document '{}'", PLUGIN_NAME, mydoc.title);

}

#[cfg(test)]
mod tests {
    use crate::plugins::mod_en_in_business_standard;

    #[test]
    fn test_run_worker_thread() {
        // TODO: implement this
        assert_eq!(1, 1);
    }
}
