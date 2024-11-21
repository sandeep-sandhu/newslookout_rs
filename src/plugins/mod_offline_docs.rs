// file: mod_offline_docs
// Purpose:

use std::cmp::Ordering;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::ops::Add;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, SendError};
use std::thread::JoinHandle;
use {
    regex::Regex,
};
use config::Config;
use chrono::{DateTime, Local, TimeDelta, Utc};
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
    match get_plugin_config(&app_config, PLUGIN_NAME, "file_extension"){
        Some(file_extension_str) => {
            file_extension =file_extension_str;
        }, None => {}
    };

    let mut published_in_past_days:usize = 30;
    match get_plugin_config(&app_config, PLUGIN_NAME, "published_in_past_days"){
        Some(published_in_past_days_str) => {
            match published_in_past_days_str.parse::<usize>(){
                Result::Ok(configintvalue) => published_in_past_days =configintvalue, Err(e)=>{}
            }
        }, None => {}
    };

    match get_data_folder(&app_config).to_str(){
        Some(data_folder_name) => {

            // read all json docs from data_folder_name and prepare vector of Documents
            let mut all_json_files: Vec<PathBuf> = get_files_listing_from_dir(data_folder_name, file_extension.as_str());

            let doc_count = get_and_send_docs_from_data_folder(all_json_files, tx, published_in_past_days);

            info!("{}: processed {} documents.", PLUGIN_NAME, doc_count);
        },
        None => {
            error!("Got nothing when getting path to store data");
            panic!("Unable to determine path to store data files.");
        }
    };
}


fn get_and_send_docs_from_data_folder(filepaths_in_dir: Vec<PathBuf>, tx: Sender<document::Document>, published_in_past_days: usize) -> usize{

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

        // for each document extracted, run this function:
        custom_data_processing(&mut mydoc);

        // send only those files that were published within the past 'published_in_past_days' days:
        if check_pubdate_within_days(mydoc.publish_date_ms, published_in_past_days as i64) {
            // send forward processed document:
            match tx.send(mydoc) {
                Ok(_) => {}
                Err(e) => { error! {"When sending offline document: {}", e} }
            }
            new_docs_counter += 1;
        }
        else{
            info!("{}: not processing document titled '{}' published before {} days.",
                PLUGIN_NAME, mydoc.title, published_in_past_days);
        }
    }
    return new_docs_counter;
}

fn check_pubdate_within_days(pubdate_ms: i64, published_within_days: i64) -> bool {
    let timenow = chrono::Utc::now();
    let publish_date = chrono::DateTime::from_timestamp(pubdate_ms,0).unwrap();
    let cutoff_date = timenow.add(TimeDelta::days(-1*published_within_days));
    match publish_date.cmp(&cutoff_date) {
        Ordering::Less => {
            println!("{} < {}", publish_date, cutoff_date);
            return false;
        },
        Ordering::Equal => {
            println!("{} == {}", publish_date, cutoff_date);
            return true;
        },
        Ordering::Greater => {
            println!("{} > {}", publish_date, cutoff_date);
            return true;
        },
    }
}

fn custom_data_processing(mydoc: &mut document::Document){

    info!("{}: processing url document with title - '{}'", PLUGIN_NAME, mydoc.title);

    // implement any custom data processing

}

#[cfg(test)]
mod tests {
    use std::ops::Add;
    use chrono::{DateTime, FixedOffset, Local, TimeDelta};
    use chrono::format::Fixed::TimezoneName;
    use crate::plugins::mod_offline_docs;
    use crate::plugins::mod_offline_docs::check_pubdate_within_days;

    #[test]
    fn test_check_pubdate_within() {
        let timenow : DateTime<Local> = Local::now();
        //
        let example1_pubdate_ms = 1385663400;
        let example1_days_within = 30;
        let example1_result = check_pubdate_within_days(example1_pubdate_ms, example1_days_within);
        // println!("check: {} - {} > {}  ? = {}", timenow, example1_days_within, example1_pubdate_ms, example1_result);
        assert_eq!(example1_result, false);
        //
        let example2_pubdate_ms = 1722277800;
        let example2_days_within = 115;
        let example2_result = check_pubdate_within_days(example2_pubdate_ms, example2_days_within);
        // println!("check: {} - {} > {:?}  ? = {}", timenow.format("%Y-%m-%d"), example2_days_within, chrono::DateTime::from_timestamp(example2_pubdate_ms,0).expect("need timestamp").format("%Y-%m-%d"), example2_result);
        assert_eq!(example2_result, true);
    }
}
