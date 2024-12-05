// file: mod_offline_docs
// Purpose:

use std::cmp::Ordering;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::ops::Add;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Sender, SendError};
use std::thread::JoinHandle;
use {
    regex::Regex,
};
use config::Config;
use chrono::{DateTime, Local, TimeDelta, Utc};
use log::{debug, error, info};
use reqwest::blocking::Client;
use scraper::Node::Document;
use crate::{document, network};
use crate::get_plugin_cfg;
use crate::network::make_http_client;
use crate::utils::{extract_text_from_pdf, get_files_listing_from_dir, get_urls_from_database};
use crate::cfg::{get_data_folder};

pub(crate) const PLUGIN_NAME: &str = "mod_offline_docs";
const PUBLISHER_NAME: &str = "Read documents from disk";

pub(crate) fn run_worker_thread(tx: Sender<document::Document>, app_config: Arc<config::Config>) {

    info!("{}: Starting worker", PLUGIN_NAME);
    // get parameter file_extension
    let mut file_extension = String::from("json");

    match get_plugin_cfg!(PLUGIN_NAME, "file_extension", &app_config) {
        Some(file_extension_str) => {
            file_extension =file_extension_str;
        }, None => {}
    };

    let mut published_in_past_days:usize = 30;
    match get_plugin_cfg!(PLUGIN_NAME, "published_in_past_days", &app_config) {
        Some(published_in_past_days_str) => {
            match published_in_past_days_str.parse::<usize>(){
                Result::Ok(configintvalue) => published_in_past_days =configintvalue, Err(e)=>{}
            }
        }, None => {}
    };

    // get parameter folder_name
    let mut data_folder_name = String::from("data");
    match get_plugin_cfg!(PLUGIN_NAME, "folder_name", &app_config) {
        Some(param_str) => {
            data_folder_name =param_str;
            // read all docs from data_folder_name and prepare vector of Documents
            let all_files_in_dir: Vec<PathBuf> = get_files_listing_from_dir(data_folder_name.as_str(), file_extension.as_str());

            let doc_count = get_and_send_docs_from_data_folder(all_files_in_dir, tx, file_extension.as_str(), published_in_past_days);

            info!("{}: processed {} documents.", PLUGIN_NAME, doc_count);
        }, None => {
            error!("{}: Could not read folder name from config file.", PLUGIN_NAME);
        }
    };

}


fn get_and_send_docs_from_data_folder(filepaths_in_dir: Vec<PathBuf>, tx: Sender<document::Document>, file_extension: &str, published_in_past_days: usize) -> usize{

    let mut new_docs_counter: usize = 0;
    let mut filename = String::new();

    // for each file, read contents:
    for doc_file_path in filepaths_in_dir {

        match doc_file_path.to_string_lossy().parse(){
            Ok(parsed_file_path) => filename = parsed_file_path,
            Err(e) => error!("Unable to convert file path to text: {}", e)
        }

        if let Some(mut doc_read_from_file) = load_document_from_file(
            doc_file_path, filename.clone(), file_extension
        ) {

            // change filename to present filename
            doc_read_from_file.filename = filename.clone();

            // for each document extracted, run this data processing function:
            custom_data_processing(&mut doc_read_from_file);

            // send only those files that were published within the past 'published_in_past_days' days:
            if check_pubdate_within_days(doc_read_from_file.publish_date_ms, published_in_past_days as i64) {
                // send forward processed document:
                match tx.send(doc_read_from_file) {
                    Ok(_) => {}
                    Err(e) => error!("When sending offline document: {}", e)
                }
                new_docs_counter += 1;
            } else {
                info!("{}: Ignoring document titled '{}' published more than {} days ago.",
                                        PLUGIN_NAME, doc_read_from_file.title, published_in_past_days);
            }
        }
    }
    return new_docs_counter;
}

fn load_document_from_file(doc_file_path: PathBuf, filename: String, file_extension: &str) -> Option<document::Document>{

    match fs::File::open(doc_file_path.clone()){
        Ok(open_file) => {
            // check and implement file extension processing for other file extensions:
            match file_extension{
                "json" => {
                    // de-serialize string to Document:
                    match serde_json::from_reader(open_file) {
                        Result::Ok(mut doc_loaded_from_file) => {
                            return Some(doc_loaded_from_file);
                        }
                        Err(e) => error!("When trying to read JSON file {}: {}", filename, e),
                    }
                },
                "pdf" => {
                    // read from pdf Documents in directory:
                    let filename = doc_file_path.to_string_lossy().to_string();
                    let txt_file_path = filename.replace(".pdf", ".txt");
                    let mut new_doc = document::Document::default();
                    new_doc.url = filename;
                    new_doc.module="PDF".to_string();
                    new_doc.text = extract_text_from_pdf(doc_file_path, PathBuf::from(txt_file_path));
                    Some(new_doc);
                },
                _ => error!("Cannot process unknown file extension: {}", file_extension)
            }
        }
        Err(e) => error!("When trying to open file {}: {}", filename, e),
    }
    return None;
}

/// Check whether the published date was within the given past number of days.
///
/// # Arguments
///
/// * `pubdate_ms`: Published date as times in seconds since epoch.
/// * `published_within_days`: No of days before the current time.
///
/// returns: bool
fn check_pubdate_within_days(pubdate_ms: i64, published_within_days: i64) -> bool {
    let timenow = chrono::Utc::now();
    match chrono::DateTime::from_timestamp(pubdate_ms,0){
        None => {
            error!("Counld not convert timestamp {} to date, hence ignoring file", pubdate_ms);
            return false;
        },
        Some(publish_date) => {
            let cutoff_date = timenow.add(TimeDelta::days(-1*published_within_days));
            match publish_date.cmp(&cutoff_date) {
                Ordering::Less => {
                    debug!("Publish date {} < Cutoff date {}", publish_date, cutoff_date);
                    return false;
                },
                Ordering::Equal => {
                    debug!("Publish date {} == Cutoff date {}", publish_date, cutoff_date);
                    return true;
                },
                Ordering::Greater => {
                    debug!("Publish date {} > Cutoff date {}", publish_date, cutoff_date);
                    return true;
                },
            }
        }
    }
}

fn custom_data_processing(mydoc: &mut document::Document){

    info!("{}: processing url document with title - '{}'", PLUGIN_NAME, mydoc.title);

    // implement any custom data processing here

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
        assert_eq!(example2_result, false);
    }
}
