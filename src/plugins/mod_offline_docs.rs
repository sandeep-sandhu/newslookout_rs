// file: mod_offline_docs
// Purpose:


use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, SendError};
use std::thread::JoinHandle;
use {
    regex::Regex,
};
use config::Config;
use chrono::Utc;
use log::{debug, error, info};
use reqwest::blocking::Client;
use crate::{document, network};
use crate::document::Document;
use crate::network::make_http_client;
use crate::utils::{get_data_folder, get_files_listing_from_dir, get_plugin_config, get_urls_from_database};

pub(crate) const PLUGIN_NAME: &str = "mod_offline_docs";
const PUBLISHER_NAME: &str = "Read documents from disk";
const STARTER_URLS: [(&str, &str); 0] = [];

pub(crate) fn run_worker_thread(tx: Sender<document::Document>, app_config: Config) {

    info!("{}: Starting worker", PLUGIN_NAME);
    // get parameter file_extension
    let mut file_extension = String::from("json");
    match get_plugin_config(&app_config, crate::plugins::mod_en_in_rbi::PLUGIN_NAME, "file_extension"){
        Some(file_extension_str) => {
            file_extension =file_extension_str;
        }, None => {}
    };

    match get_data_folder(&app_config).to_str(){
        Some(data_folder_name) => {
            // read all json docs from data_folder_name and prepare vector of Documents
            let mut all_json_files: Vec<PathBuf> = get_files_listing_from_dir(data_folder_name, file_extension.as_str());
            let doc_count = get_and_send_docs_from_data_folder(all_json_files, tx);
            info!("{}: processed {} documents.", PLUGIN_NAME, doc_count);
        },
        None => {
            error!("Got nothing when getting path to store data");
            panic!("Unable to determine path to store data files.");
        }
    };
}


fn get_and_send_docs_from_data_folder(filepaths_in_dir: Vec<PathBuf>, tx: Sender<document::Document>) -> usize{

    let mut new_docs_counter: usize = 0;

    // for each file, read contents:
    for doc_file_path in filepaths_in_dir {

        let open_file = fs::File::open(doc_file_path.clone())
            .expect("File should open read only");

        // TODO: check and implement file extension processing for other file extensions:
        // de-serialize string to Document:
        let mut mydoc: Document = serde_json::from_reader(open_file).expect("JSON was not well-formatted");

        // change filename to present filename
        mydoc.filename = doc_file_path.to_string_lossy().parse().unwrap();

        // for each document extracted run function
        custom_data_processing(&mut mydoc);

        // send forward processed document:
        match tx.send(mydoc) {
            Ok(_) => {}
            Err(e) => {error!{"When sending offline document: {}", e}}
        }
        new_docs_counter += 1;
    }

    return new_docs_counter;
}

fn custom_data_processing(mydoc: &mut document::Document){

    info!("{}: processing url document with title - '{}'", PLUGIN_NAME, mydoc.title);

    // implement any custom data processing

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
