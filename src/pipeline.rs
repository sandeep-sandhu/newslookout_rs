// file: pipeline.rs
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
use crate::plugins::{mod_en_in_business_standard, rbi, mod_offline_docs, mod_classify, split_text, mod_dedupe, mod_solrsubmit, mod_summarize, mod_persist_data, mod_vectorstore, mod_cmdline};
use crate::document::{Document};
use crate::utils::{make_unique_filename, save_to_disk_as_json};


#[derive(Copy, Clone, Eq, PartialEq)]
pub enum PluginType{
    Retriever,
    DataProcessor
}

#[derive(Copy, Clone, Eq, PartialEq)]
pub struct DataProcPlugin {
    pub priority: isize,
    pub enabled: bool,
    pub method: fn(Sender<document::Document>, Receiver<Document>, &config::Config)
}
impl Ord for DataProcPlugin {
    fn cmp(&self, other: &Self) -> Ordering {
        other.priority.cmp(&self.priority)
    }
}
// `PartialOrd` needs to be implemented as well.
impl PartialOrd for DataProcPlugin {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}


#[derive(Clone, Eq, PartialEq)]
pub struct RetrieverPlugin {
    pub name: String,
    pub priority: isize,
    pub enabled: bool,
    pub method: fn(Sender<document::Document>, config::Config)
}


/// Extract the plugin's parameters from its entry in the application's config file.
///
/// # Arguments
///
/// * `plugin_map`: The plugin map of all plugins
///
/// returns: (String, PluginType, bool, isize)
pub fn extract_plugin_params(plugin_map: Map<String, Value>) -> (String, PluginType, bool, isize) {

    let mut plugin_enabled: bool = false;
    let mut plugin_priority: isize = 99;
    let mut plugin_name = String::from("");
    let mut plugin_type = PluginType::Retriever;

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
            match enabled_str.clone().into_bool(){
                Result::Ok(plugin_enabled_bool) => plugin_enabled = plugin_enabled_bool,
                Err(e) => error!("For plugin {}, in config, fix the invalid value of plugin state, value should be either true or false: {}", plugin_name, e)
            }
        },
        None => {
            error!("For plugin {}, could not interpret whether enabled state is true or false ", plugin_name)
        }
    }
    match plugin_map.get("type") {
        Some(plugin_type_str) => {
            match plugin_type_str.to_string().as_str() {
                "retriever" => plugin_type = PluginType::Retriever,
                "data_processor" => plugin_type = PluginType::DataProcessor,
                _ => info!("For plugin {} Unable to identify plugin type {}, assuming it is a retriever.", plugin_name, plugin_type_str)
            }
        }
        None => {
            error!("For plugin {}, Invalid/missing plugin type in config, assuming default as a retriever", plugin_name);
        }
    }
    match plugin_map.get("priority") {
        Some(&ref priority_str) => {
            match priority_str.clone().into_int(){
                Result::Ok(priority_int ) => plugin_priority = priority_int as isize,
                Err(e) => error!("In config file, for plugin {}, fix the priority value of plugin state; value should be positive integer: {}", plugin_name, e)
            }
        },
        None => {
            error!("Could not interpret priority for plugin {}", plugin_name)
        }
    }
    return (plugin_name, plugin_type, plugin_enabled, plugin_priority)
}

/// Loads the configuration for each retriever plugin from the application configuration
///
/// # Arguments
///
/// * `app_config`: The application configuration
///
/// returns: Vec<RetrieverPlugin, Global>
pub fn load_retriever_plugins(app_config: &Config) -> Vec<RetrieverPlugin> {

    let mut retriever_plugins: Vec<RetrieverPlugin> = Vec::new();
    let mut plugins_configured = Vec::new();

    info!("Reading the configuration and starting the plugins.");
    match app_config.get_array("plugins"){
        Ok(plugins) => plugins_configured = plugins,
        Err(e) => {
            error!("No plugins specified in configuration file! {}", e);
        }
    }

    for plugin in plugins_configured {

        match plugin.into_table() {

            Ok(plugin_map) => {

                let (plugin_name, plugin_type, plugin_enabled, priority) =
                    extract_plugin_params(plugin_map);

                // check value of plugin to invoke the relevant module:
                match plugin_name.as_str() {
                    rbi::PLUGIN_NAME => {
                        retriever_plugins.push(
                            RetrieverPlugin {
                                name: plugin_name,
                                priority: priority,
                                enabled: plugin_enabled,
                                method: rbi::run_worker_thread,
                            }
                        );
                        continue;
                    },
                    mod_en_in_business_standard::PLUGIN_NAME => {
                        retriever_plugins.push(
                            RetrieverPlugin {
                                name: plugin_name,
                                priority: priority,
                                enabled: plugin_enabled,
                                method: mod_en_in_business_standard::run_worker_thread,
                            }
                        );
                        continue;
                    },
                    mod_offline_docs::PLUGIN_NAME => {
                        retriever_plugins.push(
                            RetrieverPlugin {
                                name: plugin_name,
                                priority: priority,
                                enabled: plugin_enabled,
                                method: mod_offline_docs::run_worker_thread,
                            }
                        );
                        continue;
                    },
                    // add additional retrievers here:
                    _ => {
                        debug!("Unknown plugin specified in config file: {}", plugin_name.as_str())
                    }
                }
            }
            Err(e) => {error!("When loading retriever plugin from config, error was: {}", e)}
        }
    }
    return retriever_plugins;
}

/// Loads the configuration for each data processing plugin
///
/// # Arguments
///
/// * `app_config`: The application configuration
///
/// returns: BinaryHeap<DataProcPlugin, Global>
pub fn load_dataproc_plugins(app_config: &Config) -> BinaryHeap<DataProcPlugin> {

    let mut plugin_heap: BinaryHeap<DataProcPlugin> = BinaryHeap::new();
    // default value:
    let matched_data_proc_fn: fn(Sender<Document>, Receiver<Document>, &Config) = split_text::process_data;

    info!("Data processing pipeline: Reading the configuration and starting the plugins.");
    let mut plugins_configured = Vec::new();
    match app_config.get_array("plugins"){
        Ok(plugins) => plugins_configured = plugins,
        Err(_) => {
            error!("No plugins specified in configuration file! Unable to start the data processing pipeline.");
        }
    }

    for plugin in plugins_configured {
        match plugin.into_table() {
            Ok(plugin_map) => {
                let (plugin_name, plugin_type, plugin_enabled, priority) = extract_plugin_params(plugin_map);
                if plugin_enabled && plugin_type == PluginType::DataProcessor {
                    match plugin_name.as_str() {
                        "split_text" => {
                            debug!("Loading the data processing plugin: {}",plugin_name);
                            plugin_heap.push(
                                DataProcPlugin {
                                    priority: priority,
                                    enabled: plugin_enabled,
                                    method: split_text::process_data,
                                }
                            );
                        },
                        "mod_classify" => {
                            debug!("Loading the plugin: {}",plugin_name);
                            plugin_heap.push(
                                DataProcPlugin {
                                    priority: priority,
                                    enabled: plugin_enabled,
                                    method: mod_classify::process_data,
                                }
                            );
                        },
                        "mod_dedupe" => {
                            debug!("Loading the plugin: {}",plugin_name);
                            plugin_heap.push(
                                DataProcPlugin {
                                    priority: priority,
                                    enabled: plugin_enabled,
                                    method: mod_dedupe::process_data,
                                }
                            );
                        },
                        "mod_summarize" => {
                            debug!("Loading the plugin: {}",plugin_name);
                            plugin_heap.push(
                                DataProcPlugin {
                                    priority: priority,
                                    enabled: plugin_enabled,
                                    method: mod_summarize::process_data,
                                }
                            );
                        },
                        "mod_vectorstore" => {
                            debug!("Loading the plugin: {}",plugin_name);
                            plugin_heap.push(
                                DataProcPlugin {
                                    priority: priority,
                                    enabled: plugin_enabled,
                                    method: mod_vectorstore::process_data,
                                }
                            );
                        },
                        "mod_persist_data" => {
                            debug!("Loading the plugin: {}",plugin_name);
                            plugin_heap.push(
                                DataProcPlugin {
                                    priority: priority,
                                    enabled: plugin_enabled,
                                    method:  mod_persist_data::process_data,
                                }
                            );
                        },
                        "mod_solrsubmit" => {
                            debug!("Loading the plugin: {}",plugin_name);
                            plugin_heap.push(
                                DataProcPlugin {
                                    priority: priority,
                                    enabled: plugin_enabled,
                                    method: mod_solrsubmit::process_data,
                                }
                            );
                        },
                        "mod_cmdline" => {
                            debug!("Loading the plugin: {}",plugin_name);
                            plugin_heap.push(
                                DataProcPlugin {
                                    priority: priority,
                                    enabled: plugin_enabled,
                                    method: mod_cmdline::process_data,
                                }
                            );
                        },
                        // add any new plugins here:
                        _ => {
                            info!("Unable to load unknown data processing plugin: {}", plugin_name);
                        }
                    }
                }
            }
            Err(e) => error!("When reading plugin specific config: {}", e)
        }
    }
    return plugin_heap;
}


/// Starts each of the data processing plugins in their order of priority.
/// Each plugin is taken one by one from the binary heap.
/// For each plugin, a thread is started with following message processing setup:
///
/// ```pipeline input --(doc)--> tx1 --> rx1 --> tx2 --> rx2 --(processed doc)--> pipeline output```
///
/// Where:
///   - thread1 is given:```  dataproc input queue, tx1```
///   - thread2 is given:```                     rx1, tx2```
///   - thread3 is given:```                     rx2, output queue```
///
/// That is, for each item i in plugin_heap,
/// start a new thread with tx= txi and rx=previous_rx, and pass on a clone of the config object.
/// Wait for documents on channel previous_rx and, transmit them onwards to the output queue.
///
/// # Arguments
///
/// * `plugin_heap`: The binary heap with Data processing plugin structs
/// * `dataproc_docs_input_rx`: The receive channel of the pipeline
/// * `dataproc_docs_output_tx`: The transmit channel of the pipeline
/// * `config`:
///
/// returns: ()
pub fn data_processing_pipeline(
    mut plugin_heap: BinaryHeap<DataProcPlugin>,
    dataproc_docs_input_rx: Receiver<document::Document>,
    dataproc_docs_output_tx: Sender<document::Document>,
    config: &Config
) {
    // Start a thread for each plugin with following message processing setup:
    // dataproc_docs_input_rx --(raw doc)--> tx1 --> rx1 --> tx2 --> rx2 --(processed doc)--> dataproc_docs_output_tx
    // where: thread1 is given:  dataproc_docs_input_rx, tx1
    //        thread2 is given:                     rx1, tx2
    //        thread3 is given:                     rx2, dataproc_docs_output_tx
    let mut dataproc_thread_run_handles: Vec<JoinHandle<()>> = Vec::new();
    let mut previous_rx = dataproc_docs_input_rx;

    while let Some( DataProcPlugin{ priority, enabled, method }) = plugin_heap.pop() {
        if enabled == true {
            // for each item i in plugin_heap:
            info!("Starting data processing thread with priority #{}", priority);
            // create a channel with txi, rxi
            let (txi, rxi) = mpsc::channel();
            // start a new thread with tx= txi and rx=previous_rx, and clone of config:
            let config_clone = config.clone();
            let handle = thread::spawn(move || method(txi, previous_rx, &config_clone));
            dataproc_thread_run_handles.push(handle);
            previous_rx = rxi;
        } else{
            info!("Ignoring disabled data processing thread with priority #{}", priority);
        }
    }

    // Wait for documents on channel previous_rx and,
    // transmit them onwards to the output queue.
    for doc in previous_rx {
        debug!("Received document titled - '{}' at end of data processing pipeline ", doc.title);
        dataproc_docs_output_tx.send(doc).expect("when sending doc via tx");
    }

}

/// Start the complete data pipeline:
///   - It executes each retriever plugin in its own thread, all executing in parallel and sending their
/// output into the data processing pipeline queue.
///
///   - It executes each data processing plugin in its own thread but chained to each other in serial
/// order based on the priority of the plugin.
///
///   - The document output by the pipeline and received at the output channel, is then
/// written to the table 'completed_urls' for reference in the next pipeline run so previously
/// retrieved plugins are not retrieved again.
///
/// # Arguments
///
/// * `retriever_plugins`: The vector of web retriever plugins
/// * `data_proc_plugins`: The binary heap collection of the data processing plugins
/// * `app_config`: The application's config object
///
/// returns: Vec<Document, Global>
pub fn start_data_pipeline(
    retriever_plugins: Vec<RetrieverPlugin>,
    data_proc_plugins: BinaryHeap<DataProcPlugin>,
    app_config: &Config
) -> Vec<Document> {

    // start the inter-thread message queues
    let (retrieve_thread_tx, data_proc_pipeline_rx) = mpsc::channel();
    let (data_proc_pipeline_tx, processed_data_rx) = mpsc::channel();

    // start the data processing plugins
    let thread_builder = thread::Builder::new()
        .name("data_processing_pipeline".into());
    // start data processing pipeline in its own thread:
    let config_copy = app_config.clone();
    match thread_builder.spawn(
        move || data_processing_pipeline(
            data_proc_plugins,
            data_proc_pipeline_rx,
            data_proc_pipeline_tx,
            &config_copy)
    ){
        Ok(_handle) => info!("Launched data processing thread"),
        Err(e) => error!("Could not spawn thread for data processing plugin, error: {}", e)
    }

    // start the retriever plugin threads: they all send via transmit
    let _retriever_thread_handles = start_retrieval_pipeline(retriever_plugins, retrieve_thread_tx, app_config);

    // get all processed documents from the output of the data processing pipeline and
    // write these to the database table
    let mut all_docs_processed: Vec<document::Document> = Vec::new();
    let mut last_written: usize = 0;

    for processed_docinfo in processed_data_rx {
        all_docs_processed.push(processed_docinfo);
        // write to database after every 100 urls:
        if all_docs_processed.len() % 100 == 0 {
            let current_idx = all_docs_processed.len() - 1;
            let written_rows = utils::insert_urls_info_to_database(
                app_config, &all_docs_processed[last_written..current_idx]
            );
            info!("Wrote {} retrieved urls into the 'completed_urls' table.", written_rows);
            if written_rows < (current_idx - last_written) {
                error!("Could not write all {} of the retrieved urls into the table, wrote {}.",
                    all_docs_processed.len(), written_rows);
            }
            last_written = current_idx;
        }
    }
    let current_idx = if all_docs_processed.len() ==0 {0} else {all_docs_processed.len() - 1};
    // write remaining urls, and then get count of successfully written to compare with list given:
    if utils::insert_urls_info_to_database(app_config, &all_docs_processed[last_written..current_idx]) < (current_idx - last_written) {
        error!("Could not write all of the retrieved urls into database table.");
    }
    return all_docs_processed;
}

fn start_retrieval_pipeline(plugins: Vec<RetrieverPlugin>, tx: Sender<document::Document>, config: &Config) -> Vec<JoinHandle<()>>{

    let mut task_run_handles: Vec<JoinHandle<()>> = Vec::new();

    for plugin in plugins {

        let msg_tx = tx.clone();
        let config_clone= config.clone();

        if plugin.enabled == true {
            let thread_builder = thread::Builder::new()
                .name(plugin.name.as_str().into());
            let plugin_retrieve_function = plugin.method;
            match thread_builder.spawn(
                move ||  plugin_retrieve_function(msg_tx, config_clone)
            ) {
                Result::Ok(handle) => {
                    task_run_handles.push(handle);
                    info!("Started thread for plugin: {}", plugin.name);
                },
                Err(e) => error!("Could not spawn thread for plugin {}, error: {}", plugin.name, e)
            }
        }
    }

    return task_run_handles;
}


#[cfg(test)]
mod tests {
    use std::collections::BinaryHeap;
    use crate::plugins::split_text;
    use crate::pipeline;
    use crate::pipeline::{DataProcPlugin};

    #[test]
    fn test_1() {
        // TODO: implement this
        assert_eq!(1, 1);
    }

    #[test]
    fn test_priority_queue(){
        let mut plugin_heap: BinaryHeap<DataProcPlugin> = BinaryHeap::new();
        let plugin1 = DataProcPlugin{priority: 10, enabled: true, method: split_text::process_data};
        let plugin2 = DataProcPlugin{priority: -20, enabled: true, method: split_text::process_data};
        let plugin3 = DataProcPlugin{priority: 2, enabled: true, method: split_text::process_data};
        plugin_heap.push(plugin1);
        plugin_heap.push(plugin2);
        plugin_heap.push(plugin3);
        if let Some( DataProcPlugin{ priority, enabled, method }) = plugin_heap.pop() {
            println!("1st item, got priority = {}", priority);
            assert_eq!(priority, -20, "Invalid min heap/priority queue processing");
        }
        if let Some( DataProcPlugin{ priority, enabled, method }) = plugin_heap.pop() {
            println!("2nd item, got priority = {}", priority);
            assert_eq!(priority, 2, "Invalid min heap/priority queue processing");
        }
        if let Some( DataProcPlugin{ priority, enabled, method }) = plugin_heap.pop() {
            println!("3rd item, got priority = {}", priority);
            assert_eq!(priority, 10, "Invalid min heap/priority queue processing");
        }
    }
}
