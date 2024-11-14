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
use crate::plugins::{mod_en_in_business_standard, mod_en_in_rbi, mod_offline_docs, mod_classify, mod_dataprep, mod_dedupe, mod_ollama, mod_solrsubmit, mod_chatgpt, mod_gemini};
use crate::document::{DocInfo, Document};
use crate::utils::{extract_plugin_params, make_unique_filename, save_to_disk_as_json};

pub fn start_pipeline(config: config::Config) -> Vec<DocInfo> {

    let (retrieve_thread_tx, data_proc_pipeline_rx) = mpsc::channel();
    let (data_proc_pipeline_tx, processed_data_rx) = mpsc::channel();

    let thread_builder = thread::Builder::new()
        .name("data_processing_pipeline".into());
    // start data processing pipeline in a thread:
    let config_copy = config.clone();
    match thread_builder.spawn(
        move || run_data_proc_pipeline(data_proc_pipeline_tx, data_proc_pipeline_rx, config_copy)
    ){
        Result::Ok(handle) => info!("Launched data processing thread"),
        Err(e) => error!("Could not spawn thread for data processing plugin, error: {}", e)
    }

    let retriever_thread_handles = start_retrieval_plugins(&config, retrieve_thread_tx);

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
    return all_docs_processed;
}

/// Start the data retrieval plugins in pipeline which starts them in parallel in multiple threads
///
/// # Arguments
///
/// * `config`: The applicaiton configuration
/// * `tx`:
///
/// returns: Vec<JoinHandle<()>, Global>
pub fn start_retrieval_plugins(config: &Config, tx: Sender<document::Document>) -> Vec<JoinHandle<()>> {

    let mut task_run_handles: Vec<JoinHandle<()>> = Vec::new();

    info!("Reading the configuration and starting the plugins.");
    let mut plugins_configured = Vec::new();
    match config.get_array("plugins"){
        Ok(plugins) => plugins_configured = plugins,
        Err(e) => {
            error!("No plugins specified in configuration file! {}", e);
        }
    }

    for plugin in plugins_configured {

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
                        debug!("Ignoring disabled plugin: {}", plugin_name);
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


#[derive(Copy, Clone, Eq, PartialEq)]
struct PluginPriority{
    priority: isize,
    plugin_function: fn(Sender<Document>, Receiver<Document>, &Config)
}
// The priority queue depends on `Ord`.
// Explicitly implement the trait so the priority queue becomes a min-heap
impl Ord for PluginPriority {
    fn cmp(&self, other: &Self) -> Ordering {
        other.priority.cmp(&self.priority)
    }
}
// `PartialOrd` needs to be implemented as well.
impl PartialOrd for PluginPriority {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}


/// Run the data processing pipeline in which all data processing plugins are run in serial
/// manner in their order of priority.
///
/// # Arguments
///
/// * `dataproc_docs_output_tx`:
/// * `dataproc_docs_input_rx`:
/// * `config`: The application's configuration object
///
/// returns: ()
fn run_data_proc_pipeline(dataproc_docs_output_tx: Sender<document::DocInfo>, dataproc_docs_input_rx: Receiver<document::Document>, config: Config){

    let mut plugin_heap: BinaryHeap<PluginPriority> = BinaryHeap::new();
    let mut matched_data_proc_fn: fn(Sender<Document>, Receiver<Document>, &Config) = mod_dataprep::process_data;

    // write file to the folder specified in config file, e.g.: data_folder_name=/var/cache
    let mut data_folder_name = String::from("");
    match config.get_string("data_dir") {
        Ok(dirname) => data_folder_name = dirname,
        Err(e) => error!("When getting data folder name: {}", e)
    }

    info!("Data processing pipeline: Reading the configuration and starting the plugins.");
    let mut plugins_configured = Vec::new();
    match config.get_array("plugins"){
        Ok(plugins) => plugins_configured = plugins,
        Err(_) => {
            error!("No plugins specified in configuration file! Unable to start the data processing pipeline.");
            return;
        }
    }

    for plugin in plugins_configured {
        match plugin.into_table() {
            Ok(plugin_map) => {
                let (plugin_name, plugin_type, plugin_enabled, priority) = extract_plugin_params(plugin_map);
                if plugin_enabled & plugin_type.eq("data_processor") {
                    match plugin_name.as_str() {
                        "mod_dataprep" => {
                            info!("Starting the plugin: {}",plugin_name);
                            matched_data_proc_fn = mod_dataprep::process_data;
                        },
                        "mod_classify" => {
                            info!("Starting the plugin: {}",plugin_name);
                            matched_data_proc_fn = mod_classify::process_data;
                        },
                        "mod_dedupe" => {
                            info!("Starting the plugin: {}",plugin_name);
                            matched_data_proc_fn = mod_dedupe::process_data;
                        },
                        "mod_ollama" => {
                            info!("Starting the plugin: {}",plugin_name);
                            matched_data_proc_fn = mod_ollama::process_data;
                        },
                        "mod_chatgpt" => {
                            info!("Starting the plugin: {}",plugin_name);
                            matched_data_proc_fn = mod_chatgpt::process_data;
                        },
                        "mod_gemini" => {
                            info!("Starting the plugin: {}",plugin_name);
                            matched_data_proc_fn = mod_gemini::process_data;
                        },
                        "mod_solrsubmit" => {
                            info!("Starting the plugin: {}",plugin_name);
                            matched_data_proc_fn = mod_solrsubmit::process_data;
                        },
                        _ => {
                            error!("Cannot start unknown plugin: {}", plugin_name);
                            break;
                        }
                    }
                    // now add to heap:
                    plugin_heap.push(PluginPriority{priority: priority, plugin_function: matched_data_proc_fn})
                }
            }
            Err(e) => error!("When reading plugin specific config: {}", e)
        }
    }
    // Use threads for all plugins with following message processing setup:
    // dataproc_docs_input_rx --(raw doc)--> tx1 --> rx1 --> tx2 --> rx2 --(processed doc)--> dataproc_docs_output_tx
    // where: thread1 is given:  dataproc_docs_input_rx, tx1
    //        thread2 is given:                     rx1, tx2
    //        thread3 is given:                     rx2, dataproc_docs_output_tx
    let mut dataproc_thread_run_handles: Vec<JoinHandle<()>> = Vec::new();
    let mut previous_rx = dataproc_docs_input_rx;
    while let Some( PluginPriority{ priority, plugin_function }) = plugin_heap.pop() {
        // for each item i in plugin_heap:
        debug!("Starting data processing plugin thread with priority - {}", priority);
        // create a channel with txi, rxi
        let (txi, rxi) = mpsc::channel();
        // start a new thread with tx= txi and rx=previous_rx, and clone of config:
        let config_clone= config.clone();
        let handle = thread::spawn(move || plugin_function(txi, previous_rx, &config_clone));
        dataproc_thread_run_handles.push(handle);
        previous_rx = rxi;
    }

    // After loop, wait for documents on channel previous_rx and,
    // save to disk and then
    // transmit docinfo to dataproc_docs_output_tx
    for mut doc in previous_rx {
        info!("Saving processed document titled - {}", doc.title);
        // make full path by joining folder to unique filename
        let json_file_path = Path::new(data_folder_name.as_str()).join(make_unique_filename(&doc, "json"));
        doc.filename = String::from(json_file_path.as_path().to_str().expect("Not able to convert path to string"));
        let docinfo:DocInfo = save_to_disk_as_json(&doc, json_file_path.to_str().unwrap_or_default());
        dataproc_docs_output_tx.send(docinfo).expect("when sending doc via tx");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BinaryHeap;
    use crate::plugins::mod_dataprep;
    use crate::queue;
    use crate::queue::PluginPriority;

    #[test]
    fn test_1() {
        // TODO: implement this
        assert_eq!(1, 1);
    }

    #[test]
    fn test_priority_queue(){
        let mut plugin_heap: BinaryHeap<PluginPriority> = BinaryHeap::new();
        let plugin1 = crate::queue::PluginPriority{priority: 10, plugin_function: mod_dataprep::process_data};
        let plugin2 = crate::queue::PluginPriority{priority: -20, plugin_function: mod_dataprep::process_data};
        let plugin3 = crate::queue::PluginPriority{priority: 2, plugin_function: mod_dataprep::process_data};
        plugin_heap.push(plugin1);
        plugin_heap.push(plugin2);
        plugin_heap.push(plugin3);
        if let Some( PluginPriority{ priority, plugin_function }) = plugin_heap.pop() {
            println!("1st item, got priority = {}", priority);
            assert_eq!(priority, -20, "Invalid min heap/priority queue processing");
        }
        if let Some( PluginPriority{ priority, plugin_function }) = plugin_heap.pop() {
            println!("2nd item, got priority = {}", priority);
            assert_eq!(priority, 2, "Invalid min heap/priority queue processing");
        }
        if let Some( PluginPriority{ priority, plugin_function }) = plugin_heap.pop() {
            println!("3rd item, got priority = {}", priority);
            assert_eq!(priority, 10, "Invalid min heap/priority queue processing");
        }
    }
}
