// file: queue.rs
// Purpose:
// Manage the work Queue

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, mpsc};
use std::sync::mpsc::{Receiver, Sender};
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

use config::{Config, Map, Value};
use chrono::Utc;
use log::{debug, error, info, warn};
use rusqlite;

use crate::document;
use crate::network;
use crate::utils;
use crate::plugins::{
    mod_en_in_business_standard,
    mod_en_in_rbi,
    mod_offline_docs,
    mod_classify,
    mod_dataprep,
    mod_dedupe,
    mod_ollama,
    mod_solrsubmit};
use crate::document::{DocInfo, Document};

pub fn start_pipeline(config: config::Config) -> usize{

    let (retrieve_thread_tx, data_proc_pipeline_rx) = mpsc::channel();
    let (data_proc_pipeline_tx, processed_data_rx) = mpsc::channel();

    let thread_builder = thread::Builder::new()
        .name("data_processing_pipeline".into());
    // start data processing pipeline in a thread:
    let config_copy = config.clone();
    match thread_builder.spawn(
        move || run_data_proc_pipeline(data_proc_pipeline_tx, data_proc_pipeline_rx, config_copy)
    ){
        Result::Ok(handle) => info!("Launched data processing thread with handle {:?}", handle.thread().name()),
        Err(e) => error!("Could not spawn thread for data processing plugin, error: {}", e)
    }

    let retriever_thread_handles = start_retrieval_plugins(&config, retrieve_thread_tx);

    for task_handle in retriever_thread_handles {
        let thread = &task_handle.thread();
        let thread_id = thread.id();
        let mut th_name = String::from("");
        match thread.name() {
            Some(thread_name_str) => {
                th_name = thread_name_str.to_string();
                info!("Waiting for thread: {:?}, name: {:?}", thread_id, th_name);
            },
            None => info!("Waiting for thread: {:?}", thread_id),
        }
        match task_handle.join() {
            Ok(_result) => log::info!("Retriever thread {:?} {} finished", thread_id, th_name),
            Err(e) => log::error!("Error joining retriever thread {:?} to queue: {:?}", thread_id, e)
        }
    }

    let mut all_docs_processed: Vec<document::DocInfo> = Vec::new();
    for processed_docinfo in processed_data_rx {
        all_docs_processed.push(processed_docinfo);
        // write to database after every 100 urls:
        if all_docs_processed.len() >= 100 {
            let written_rows = utils::insert_urls_info_to_database(&config, &all_docs_processed);
            info!("Wrote {} rows of the retrieved urls into database table.", written_rows);
            if written_rows < all_docs_processed.len() as u64 {
                error!("Could not write all {} of the retrieved urls into database table, wrote {}.", all_docs_processed.len(), written_rows);
            }
            all_docs_processed.clear();
        }
    }
    if utils::insert_urls_info_to_database(&config, &all_docs_processed) < all_docs_processed.len() as u64 {
        error!("Could not write all of the retrieved urls into database table.");
    }
    return all_docs_processed.len();
}

fn save_to_disk(mut received: Document, data_folder_name: &String) -> DocInfo {

    debug!("Writing document from url: {:?}", received.url);
    let mut docinfo_for_sql = DocInfo{
        plugin_name: received.plugin_name.clone(),
        url: received.url.clone(),
        pdf_url: received.pdf_url.clone(),
        title: received.title.clone(),
        unique_id: received.unique_id.clone(),
        publish_date_ms: received.publish_date_ms,
        filename: received.filename.clone(),
        section_name: received.section_name.clone(),
    };

    // serialize json to string
    match serde_json::to_string_pretty(&received){
        Ok(json_data) => {
            let json_filename = utils::make_unique_filename(&received, "json");
            debug!("Writing document to file: {}", json_filename);
            // make full path by joining folder to unique filename
            let json_file_path = Path::new(data_folder_name.as_str()).join(&json_filename);
            received.filename = String::from(json_file_path.as_path().to_str().expect("Not able to convert path to string"));
            // persist to json
            match File::create(&json_file_path){
                Ok(mut file) => {
                    match file.write_all(json_data.as_bytes()) {
                        Ok(_write_res) => {
                            debug!("Wrote document from {}, titled '{}' to file: {:?}", received.url, received.title, json_file_path);
                            docinfo_for_sql.filename = received.filename.clone();
                            return docinfo_for_sql;
                        },
                        Err(write_err) => error!("When writing file to disk: {}", write_err)
                    }
                },
                Err(file_err)=> {
                    error!("When writing document to json file: {}", file_err);
                }
            }
        },
        Err(serderr) => error!("When serialising document to JSON text: {}", serderr)
    }
    return docinfo_for_sql;
}

pub fn start_retrieval_plugins(config: &Config, tx: Sender<document::Document>) -> Vec<JoinHandle<()>> {

    let mut task_run_handles: Vec<JoinHandle<()>> = Vec::new();

    info!("Reading the configuration and starting the plugins.");
    let plugins = config.get_array("plugins").expect("No plugins specified in configuration file!");

    for plugin in plugins {

        let msg_tx = tx.clone();
        let config_clone= config.clone();

        match plugin.into_table(){
            Ok(plugin_map ) => {
                let (plugin_name, plugin_type, plugin_enabled, _priority) = extract_plugin_params(plugin_map);
                if plugin_type.eq("retriever") {
                    if plugin_enabled {
                        let thread_builder = thread::Builder::new()
                            .name(plugin_name.as_str().into());

                        // check value of plugin to invoke the relevant module:
                        match plugin_name.as_str() {
                            mod_en_in_rbi::PLUGIN_NAME => {
                                // start thread with function of matched plugin:
                                match thread_builder.spawn(
                                    move || mod_en_in_rbi::run_worker_thread(msg_tx, config_clone)
                                ) {
                                    Result::Ok(handle) => task_run_handles.push(handle),
                                    Err(e) => error!("Could not spawn thread for plugin {}, error: {}", mod_en_in_rbi::PLUGIN_NAME, e)
                                }
                            },
                            mod_en_in_business_standard::PLUGIN_NAME => {
                                match thread_builder.spawn(
                                    move || mod_en_in_business_standard::run_worker_thread(msg_tx, config_clone)
                                ) {
                                    Result::Ok(handle) => task_run_handles.push(handle),
                                    Err(e) => error!("Could not spawn thread for plugin {}, error: {}", mod_en_in_rbi::PLUGIN_NAME, e)
                                }
                            },
                            mod_offline_docs::PLUGIN_NAME => {
                                match thread_builder.spawn(
                                    move || mod_offline_docs::run_worker_thread(msg_tx, config_clone)
                                ) {
                                    Result::Ok(handle) => task_run_handles.push(handle),
                                    Err(e) => error!("Could not spawn thread for plugin {}, error: {}", mod_en_in_rbi::PLUGIN_NAME, e)
                                }
                            },
                            _ => {
                                error!("Unknown plugin specified in config file: {}", plugin_name.as_str())
                            }
                        }
                    } else {
                        info!("Ignoring disabled plugin: {}", plugin_name);
                    }
                }
            }
            Err(e) => {
                error!("When reading plugin parameters from config file: {}", e);
            }
        }
    }
    return task_run_handles;
}

fn extract_plugin_params(plugin_map: Map<String, Value>) -> (String, String, bool, i64) {
    let mut plugin_enabled: bool = false;
    let mut plugin_priority: i64 = 99;
    let mut plugin_name = String::from("");
    let mut plugin_type = String::from("retriever");

    match plugin_map.get("name") {
        Some(name_str) => {
            plugin_name = name_str.to_string();
        },
        None => {
            error!("Unble to get plugin name from the config! Using default value of '{}'", plugin_name);
        }
    }
    match plugin_map.get("enabled") {
        Some(&ref enabled_str) => {
            plugin_enabled = enabled_str
                .clone()
                .into_bool()
                .expect(
                    "In config file, fix the invalid value of plugin state, value should be either true or false"
                );
        },
        None => {
            error!("Could not interpret whether enabled state is true or false for plugin {}", plugin_name)
        }
    }
    match plugin_map.get("type") {
        Some(plugin_type_str) => {
            plugin_type = plugin_type_str.to_string();
        }
        None => {
            error!("Invalid/missing plugin type in config, Using default value = '{}'",
                            plugin_type);
        }
    }

    match plugin_map.get("priority") {
        Some(&ref priority_str) => {
            plugin_priority = priority_str
                .clone()
                .into_int()
                .expect(
                    "In config file, fix the priority value of plugin state; value should be positive integer"
                );
        },
        None => {
            error!("Could not interpret priority for plugin {}", plugin_name)
        }
    }
    return (plugin_name, plugin_type, plugin_enabled, plugin_priority)
}

fn run_data_proc_pipeline(tx: Sender<document::DocInfo>, rx: Receiver<document::Document>, config: Config){

    // TODO: change to use threads for all plugins with initialisation and then message processing

    let mut data_proc_funcs: Vec<fn(&Document, &Config)> = Vec::new();

    info!("Data processing thread: Reading the configuration and starting the plugins.");
    let plugins = config.get_array("plugins").expect("No plugins specified in configuration file!");

    for plugin in plugins {
        let config_clone = config.clone();
        match plugin.into_table() {
            Ok(plugin_map) => {
                let (plugin_name, plugin_type, plugin_enabled, priority) = extract_plugin_params(plugin_map);
                if plugin_enabled & plugin_type.eq("data_processor") {
                    match plugin_name.as_str() {
                        "mod_dataprep" => {
                            info!("Starting the plugin: {}",plugin_name);
                            data_proc_funcs.push(mod_dataprep::process_data);
                        },
                        "mod_classify" => {
                            info!("Starting the plugin: {}",plugin_name);
                            data_proc_funcs.push(mod_classify::process_data);
                        },
                        "mod_dedupe" => {
                            info!("Starting the plugin: {}",plugin_name);
                            data_proc_funcs.push(mod_dedupe::process_data);
                        },
                        "mod_ollama" => {
                            info!("Starting the plugin: {}",plugin_name);
                            data_proc_funcs.push(mod_ollama::process_data);
                        },
                        "mod_solrsubmit" => {
                            info!("Starting the plugin: {}",plugin_name);
                            data_proc_funcs.push(mod_solrsubmit::process_data);
                        },
                        _ => {}
                    }
                }
            }
            Err(e) => error!("When reading plugin specific config: {}", e)
        }
    }

    // write file to the folder specified in config file, e.g.: data_folder_name=/var/cache
    let mut data_folder_name = String::from("");
    match config.get_string("data_dir") {
        Ok(dirname) => data_folder_name = dirname,
        Err(e) => error!("When getting data folder name: {}", e)
    }

    info!("Loaded {} plugins for data processing pipeline", data_proc_funcs.len());
    for doc in rx{
        info!("Data pipeline processing doc titled - {}", doc.title);
        for data_proc_fn in &data_proc_funcs{
            let _ = data_proc_fn(&doc, &config);
        }
        let docinfo:DocInfo = save_to_disk(doc, &data_folder_name);
        tx.send(docinfo).expect("when sending doc via tx");
    }
}

#[cfg(test)]
mod tests {
    use crate::queue;

    #[test]
    fn test_1() {
        // TODO: implement this
        assert_eq!(1, 1);
    }
}
