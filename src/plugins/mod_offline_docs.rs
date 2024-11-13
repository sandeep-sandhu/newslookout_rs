// file: mod_offline_docs
// Purpose:


use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use {
    regex::Regex,
};


use std::sync::mpsc::{Sender, SendError};
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
    // TODO: get parameter load_pdf_files

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

    let mut new_docs_counter: usize = 0;

    let mut all_json_files: Vec<PathBuf> = Vec::new();
    // get json file listing from data folder
    match fs::read_dir(data_folder_name) {
        Err(e) => error!("When reading json files in data folder {}, error:{}", data_folder_name, e),
        Ok(dir_entries) => {
            all_json_files = dir_entries // Filter out all those directory entries which couldn't be read
                .filter_map(|res| res.ok())
                // Map the directory entries to paths
                .map(|dir_entry| dir_entry.path())
                // Filter out all paths with extensions other than `json`
                .filter_map(|path| {
                    if path.extension().map_or(false, |ext| ext == "json") {
                        Some(path)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
        }
    }
    // for each file, read contents:
    for json_file_path in all_json_files{

        let open_file = fs::File::open(json_file_path)
            .expect("JSON file should open read only");
        // de-serialize string to Document:
        let mut mydoc: Document = serde_json::from_reader(open_file).expect("JSON was not well-formatted");
        // for each document extracted run function
        custom_data_processing(&mut mydoc);
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
