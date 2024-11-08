// file: mod_offline_docs
// Purpose:


use std::collections::HashMap;
use {
    regex::Regex,
};


use std::sync::mpsc::Sender;
use std::thread::JoinHandle;
use config::Config;
use chrono::Utc;
use log::{debug, error, info};
use reqwest::blocking::Client;
use crate::{document, network};
use crate::document::Document;
use crate::network::make_http_client;
use crate::utils::{get_data_folder, get_network_params, get_urls_from_database};

pub(crate) const PLUGIN_NAME: &str = "mod_offline_docs";
const PUBLISHER_NAME: &str = "Read documents from disk";
const STARTER_URLS: [(&str, &str); 0] = [];

pub(crate) fn run_worker_thread(tx: Sender<document::Document>, app_config: Config) {
    info!("{}: Starting worker", PLUGIN_NAME);

    match get_data_folder(&app_config).to_str(){
        Some(data_folder_name) => {
            // read all json docs from data_folder_name and prepare vector of Documents
            let doc_count = get_and_send_docs_from_data_folder(data_folder_name, tx);
            info!("{}: processed {} documents.", PLUGIN_NAME, doc_count);
        },
        None => {
            error!("Got nothing when getting path to store data");
            panic!("Unable to determine path to store data files.");
        }
    };
}

fn get_and_send_docs_from_data_folder(data_folder_name: &str, tx: Sender<document::Document>) -> usize{

    let mut new_docs: Vec<document::Document> = Vec::new();

    // TODO: implement this:
    // get json file listing from data folder
    // for each file, read contents
    // de-serialize string to Document
    // for each document extracted run function custom_data_processing(mydoc)

    return new_docs.len();
}

fn custom_data_processing(mydoc: &mut document::Document){

    info!("{}: processing url document", PLUGIN_NAME);

}

#[cfg(test)]
mod tests {
    use crate::plugins::mod_offline_docs;

    #[test]
    fn test_run_worker_thread() {
        // TODO: implement this
        assert_eq!(1, 1);
    }
}
