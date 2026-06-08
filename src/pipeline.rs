// file: pipeline.rs
// Purpose:
// Manage the work Queue

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, mpsc, Mutex};
use std::sync::mpsc::{Receiver, Sender};
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use config::{Config, Map, Value};
use chrono::Utc;
use log::{debug, error, info, warn};
use rusqlite;

use crate::document;
use crate::network;
use crate::utils;
use crate::plugins::{
    mod_en_in_business_standard, mod_en_in_rbi, mod_offline_docs, split_text,
    mod_dedupe, mod_solrsubmit, mod_summarize, mod_persist_data, mod_vectorstore, mod_cmdline,
    mod_mentions, mod_extract_quant, mod_themes, mod_tone, mod_geocode, mod_ner,
    mod_entity_graph, mod_emit_graph, mod_emit_tables,
    mod_en_in_thehindu, mod_en_in_livemint, mod_en_in_moneycontrol,
    mod_en_in_timesofindia, mod_en_in_forbes, mod_en_bbc, mod_en_guardian,
    mod_en_ap_news, mod_en_in_indianexpress,
    mod_en_in_hindustan_times, mod_en_in_news18, mod_en_aljazeera,
    mod_en_nhk_world, mod_en_arab_news, mod_en_gulf_news, mod_en_khaleej_times,
    mod_en_the_national, mod_en_punch_ng, mod_en_allafrica,
    mod_en_cnn, mod_en_foxnews,
    mod_en_cnbc, mod_en_business_insider,
    mod_en_latimes, mod_en_chicago_tribune,
    mod_en_fortune, mod_en_techcrunch, mod_en_wired,
    mod_en_theverge, mod_en_arstechnica, mod_en_cnet,
    mod_en_sg_straitstimes, mod_en_sg_cna, mod_en_th_bangkokpost,
    mod_en_ca_cbc, mod_en_ca_globeandmail, mod_en_au_smh, mod_en_au_abc,
    mod_en_in_irdai, mod_en_in_sebi,
    mod_doc_type, mod_filter, mod_metadata,
};
use crate::document::{Document};
use crate::utils::{make_unique_filename, save_to_disk_as_json};
use crate::web_api::SharedStatus;


#[derive(Copy, Clone, Eq, PartialEq)]
pub enum PluginType{
    Retriever,
    DataProcessor,
    /// A periodic structured-dataset feed run by the batch subsystem (src/feeds/), not the
    /// news pipeline. See `crate::feeds`.
    BatchFeed,
}

pub struct DataProcPlugin {
    pub name: String,
    pub priority: isize,
    pub enabled: bool,
    pub api_mutexes: HashMap<String, Arc<Mutex<isize>>>,
    pub method: fn(Sender<document::Document>, Receiver<Document>, &config::Config, &mut HashMap<String, Arc<Mutex<isize>>>)
}

impl Clone for DataProcPlugin {
    fn clone(&self) -> DataProcPlugin {
        let mut other_mutex: HashMap<String, Arc<Mutex<isize>>> = HashMap::new();
        // clone mutex by - Arc::clone(&old_arc_mutex);
        other_mutex.extend(self.api_mutexes.iter().map(|(k,v)| (k.clone(), Arc::clone(v) )));
        return DataProcPlugin {
            name: self.name.clone(),
            priority: self.priority,
            enabled: self.enabled,
            api_mutexes: other_mutex,
            method: self.method,
        };
    }
}


impl PartialEq for DataProcPlugin {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other. priority
    }
}

impl Eq for DataProcPlugin {}

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
    pub method: fn(Sender<document::Document>, Arc<config::Config>)
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
                "batch_feed" => plugin_type = PluginType::BatchFeed,
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
pub fn load_retriever_plugins(app_config: Arc<config::Config>) -> Vec<RetrieverPlugin> {

    let mut retriever_plugins: Vec<RetrieverPlugin> = Vec::new();
    let mut plugins_configured = Vec::new();

    info!("Reading the configuration and starting the plugins.");
    match app_config.get_array("plugins"){
        Ok(plugins) => plugins_configured = plugins,
        Err(e) => {
            error!("No plugins specified in configuration file! {}", e);
        }
    }

    // Dispatch table mapping each retriever plugin's config name to its worker entry point.
    // Replaces ~500 lines of identical match arms. To add a retriever, add one line here.
    let registry: &[(&str, fn(Sender<document::Document>, Arc<config::Config>))] = &[
        (mod_en_in_rbi::PLUGIN_NAME, mod_en_in_rbi::run_worker_thread),
        (mod_en_in_business_standard::PLUGIN_NAME, mod_en_in_business_standard::run_worker_thread),
        (mod_offline_docs::PLUGIN_NAME, mod_offline_docs::run_worker_thread),
        (mod_en_in_thehindu::PLUGIN_NAME, mod_en_in_thehindu::run_worker_thread),
        (mod_en_in_livemint::PLUGIN_NAME, mod_en_in_livemint::run_worker_thread),
        (mod_en_in_moneycontrol::PLUGIN_NAME, mod_en_in_moneycontrol::run_worker_thread),
        (mod_en_in_timesofindia::PLUGIN_NAME, mod_en_in_timesofindia::run_worker_thread),
        (mod_en_in_forbes::PLUGIN_NAME, mod_en_in_forbes::run_worker_thread),
        (mod_en_bbc::PLUGIN_NAME, mod_en_bbc::run_worker_thread),
        (mod_en_guardian::PLUGIN_NAME, mod_en_guardian::run_worker_thread),
        (mod_en_ap_news::PLUGIN_NAME, mod_en_ap_news::run_worker_thread),
        (mod_en_in_indianexpress::PLUGIN_NAME, mod_en_in_indianexpress::run_worker_thread),
        (mod_en_in_hindustan_times::PLUGIN_NAME, mod_en_in_hindustan_times::run_worker_thread),
        (mod_en_in_news18::PLUGIN_NAME, mod_en_in_news18::run_worker_thread),
        (mod_en_aljazeera::PLUGIN_NAME, mod_en_aljazeera::run_worker_thread),
        (mod_en_nhk_world::PLUGIN_NAME, mod_en_nhk_world::run_worker_thread),
        (mod_en_arab_news::PLUGIN_NAME, mod_en_arab_news::run_worker_thread),
        (mod_en_gulf_news::PLUGIN_NAME, mod_en_gulf_news::run_worker_thread),
        (mod_en_khaleej_times::PLUGIN_NAME, mod_en_khaleej_times::run_worker_thread),
        (mod_en_the_national::PLUGIN_NAME, mod_en_the_national::run_worker_thread),
        (mod_en_punch_ng::PLUGIN_NAME, mod_en_punch_ng::run_worker_thread),
        (mod_en_allafrica::PLUGIN_NAME, mod_en_allafrica::run_worker_thread),
        (mod_en_cnn::PLUGIN_NAME, mod_en_cnn::run_worker_thread),
        (mod_en_foxnews::PLUGIN_NAME, mod_en_foxnews::run_worker_thread),
        (mod_en_cnbc::PLUGIN_NAME, mod_en_cnbc::run_worker_thread),
        (mod_en_business_insider::PLUGIN_NAME, mod_en_business_insider::run_worker_thread),
        (mod_en_latimes::PLUGIN_NAME, mod_en_latimes::run_worker_thread),
        (mod_en_chicago_tribune::PLUGIN_NAME, mod_en_chicago_tribune::run_worker_thread),
        (mod_en_theverge::PLUGIN_NAME, mod_en_theverge::run_worker_thread),
        (mod_en_arstechnica::PLUGIN_NAME, mod_en_arstechnica::run_worker_thread),
        (mod_en_cnet::PLUGIN_NAME, mod_en_cnet::run_worker_thread),
        (mod_en_sg_straitstimes::PLUGIN_NAME, mod_en_sg_straitstimes::run_worker_thread),
        (mod_en_sg_cna::PLUGIN_NAME, mod_en_sg_cna::run_worker_thread),
        (mod_en_th_bangkokpost::PLUGIN_NAME, mod_en_th_bangkokpost::run_worker_thread),
        (mod_en_fortune::PLUGIN_NAME, mod_en_fortune::run_worker_thread),
        (mod_en_techcrunch::PLUGIN_NAME, mod_en_techcrunch::run_worker_thread),
        (mod_en_wired::PLUGIN_NAME, mod_en_wired::run_worker_thread),
        (mod_en_ca_cbc::PLUGIN_NAME, mod_en_ca_cbc::run_worker_thread),
        (mod_en_ca_globeandmail::PLUGIN_NAME, mod_en_ca_globeandmail::run_worker_thread),
        (mod_en_au_smh::PLUGIN_NAME, mod_en_au_smh::run_worker_thread),
        (mod_en_au_abc::PLUGIN_NAME, mod_en_au_abc::run_worker_thread),
        (mod_en_in_irdai::PLUGIN_NAME, mod_en_in_irdai::run_worker_thread),
        (mod_en_in_sebi::PLUGIN_NAME, mod_en_in_sebi::run_worker_thread),
        // NOTE: mod_in_nse / mod_in_bse are intentionally NOT registered as news retrievers.
        // Their market-data (bhavcopy) download moved to the batch-feed subsystem
        // (src/feeds/feed_nse_bhavcopy.rs, feed_bse_bhavcopy.rs) per roadmap point 2g.
    ];

    for plugin in plugins_configured {
        match plugin.into_table() {
            Ok(plugin_map) => {
                let (plugin_name, _plugin_type, plugin_enabled, priority) =
                    extract_plugin_params(plugin_map);
                match registry.iter().find(|(name, _)| *name == plugin_name.as_str()) {
                    Some((_, method)) => {
                        retriever_plugins.push(RetrieverPlugin {
                            name: plugin_name,
                            priority,
                            enabled: plugin_enabled,
                            method: *method,
                        });
                    }
                    None => debug!("Unknown or non-retriever plugin in config (skipped here): {}", plugin_name),
                }
            }
            Err(e) => { error!("When loading retriever plugin from config, error was: {}", e) }
        }
    }
    return retriever_plugins;
}

pub fn create_api_mutexes() -> HashMap<String, Arc<Mutex<isize>>> {
    let mut all_api_mutexes: HashMap<String, Arc<Mutex<isize>>> = HashMap::new();
    all_api_mutexes.insert(String::from("ollama"), Arc::new(Mutex::new(0)));
    all_api_mutexes.insert(String::from("chatgpt"), Arc::new(Mutex::new(0)));
    all_api_mutexes.insert(String::from("gemini"), Arc::new(Mutex::new(0)));
    all_api_mutexes.insert(String::from("google_genai"), Arc::new(Mutex::new(0)));
    return all_api_mutexes;
}

/// Loads the configuration for each data processing plugin
///
/// # Arguments
///
/// * `app_config`: The application configuration
///
/// returns: BinaryHeap<DataProcPlugin, Global>
/// Signature of a data-processing plugin's worker entry point.
type ProcFn = fn(Sender<document::Document>, Receiver<Document>, &config::Config, &mut HashMap<String, Arc<Mutex<isize>>>);

pub fn load_dataproc_plugins(app_config: Arc<config::Config>, all_api_mutexes: HashMap<String, Arc<Mutex<isize>>>) -> BinaryHeap<DataProcPlugin> {

    let mut plugin_heap: BinaryHeap<DataProcPlugin> = BinaryHeap::new();

    info!("Data processing pipeline: Reading the configuration and starting the plugins.");
    let mut plugins_configured = Vec::new();
    match app_config.get_array("plugins"){
        Ok(plugins) => plugins_configured = plugins,
        Err(_) => {
            error!("No plugins specified in configuration file! Unable to start the data processing pipeline.");
        }
    }

    // Dispatch table mapping each data-processing plugin's config name to its worker entry point.
    // Replaces ~150 lines of identical match arms. To add a data processor, add one line here.
    // NOTE: order of execution is set by each plugin's `priority` in config, not by this list.
    // NOTE: `split_text` is intentionally NOT registered — text splitting/chunking now happens
    // inside `mod_vectorstore` immediately before embedding (roadmap point 1a). Configs that
    // still list `split_text` are harmlessly skipped as an unknown plugin.
    let registry: &[(&str, ProcFn)] = &[
        ("mod_dedupe", mod_dedupe::process_data),
        (mod_mentions::PLUGIN_NAME, mod_mentions::process_data),
        (mod_extract_quant::PLUGIN_NAME, mod_extract_quant::process_data),
        (mod_themes::PLUGIN_NAME, mod_themes::process_data),
        (mod_tone::PLUGIN_NAME, mod_tone::process_data),
        (mod_geocode::PLUGIN_NAME, mod_geocode::process_data),
        (mod_ner::PLUGIN_NAME, mod_ner::process_data),
        (mod_entity_graph::PLUGIN_NAME, mod_entity_graph::process_data),
        (mod_emit_graph::PLUGIN_NAME, mod_emit_graph::process_data),
        (mod_emit_tables::PLUGIN_NAME, mod_emit_tables::process_data),
        ("mod_summarize", mod_summarize::process_data),
        ("mod_vectorstore", mod_vectorstore::process_data),
        ("mod_persist_data", mod_persist_data::process_data),
        ("mod_solrsubmit", mod_solrsubmit::process_data),
        ("mod_cmdline", mod_cmdline::process_data),
        (mod_doc_type::PLUGIN_NAME, mod_doc_type::process_data),
        (mod_filter::PLUGIN_NAME, mod_filter::process_data),
        (mod_metadata::PLUGIN_NAME, mod_metadata::process_data),
    ];

    for plugin in plugins_configured {
        match plugin.into_table() {
            Ok(plugin_map) => {
                let (plugin_name, plugin_type, plugin_enabled, priority) = extract_plugin_params(plugin_map);
                if plugin_enabled && plugin_type == PluginType::DataProcessor {
                    match registry.iter().find(|(name, _)| *name == plugin_name.as_str()) {
                        Some((_, method)) => {
                            debug!("Loading the plugin: {}", plugin_name);
                            plugin_heap.push(
                                DataProcPlugin {
                                    name: plugin_name,
                                    priority,
                                    enabled: plugin_enabled,
                                    api_mutexes: all_api_mutexes.clone(),
                                    method: *method,
                                }
                            );
                        },
                        None => {
                            debug!("Unable to load unknown data processing plugin: {}", plugin_name);
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

    while let Some(mut data_plugin) = plugin_heap.pop() {
        if data_plugin.enabled == true {
            // for each item i in plugin_heap:
            info!("Starting data processing thread {} with priority #{}", data_plugin.name, data_plugin.priority);
            // create a channel with txi, rxi
            let (txi, rxi) = mpsc::channel();
            // start a new thread with tx= txi and rx=previous_rx, and clone of config:
            let config_clone = config.clone();
            let proc_function = data_plugin.method;
            let handle = thread::spawn(move || proc_function(txi, previous_rx, &config_clone, &mut data_plugin.api_mutexes));
            dataproc_thread_run_handles.push(handle);
            previous_rx = rxi;
        } else{
            info!("Ignoring disabled data processing thread with priority #{}", data_plugin.priority);
        }
    }

    // Wait for documents on channel previous_rx and,
    // transmit them onwards to the output queue.
    for doc in previous_rx {
        debug!("Received document titled - '{}' at end of data processing pipeline ", doc.title);
        // Log-and-continue rather than panic: if the receiver has gone away (e.g. during
        // shutdown) we must not bring down the pipeline thread.
        if let Err(e) = dataproc_docs_output_tx.send(doc) {
            error!("data_processing_pipeline: receiver dropped while forwarding doc: {}", e);
            break;
        }
    }

    // Drop our end of the output channel so the collector loop terminates, then join the
    // per-plugin worker threads for a clean shutdown (each exits when its input channel
    // closes and it has flushed its work).
    drop(dataproc_docs_output_tx);
    for handle in dataproc_thread_run_handles {
        if let Err(e) = handle.join() {
            error!("data_processing_pipeline: a data-processor thread panicked: {:?}", e);
        }
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
    app_config: Arc<config::Config>,
    status_tracker: Option<SharedStatus>,
) -> Vec<Document> {

    // record counts in shared status before kicking off threads
    if let Some(ref st) = status_tracker {
        if let Ok(mut s) = st.lock() {
            s.is_running = true;
            s.retrievers_total = retriever_plugins.len();
            s.retrievers_enabled = retriever_plugins.iter().filter(|p| p.enabled).count();
            s.data_processors_total = data_proc_plugins.len();
            s.data_processors_enabled = data_proc_plugins.iter().filter(|p| p.enabled).count();
            s.retriever_plugin_names = retriever_plugins.iter()
                .filter(|p| p.enabled)
                .map(|p| p.name.clone())
                .collect();
            s.data_processor_plugin_names = data_proc_plugins.iter()
                .filter(|p| p.enabled)
                .map(|p| p.name.clone())
                .collect();
        }
    }

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
    let retriever_thread_handles = start_retrieval_pipeline(
        retriever_plugins,
        retrieve_thread_tx,
        app_config.clone()
    );

    // get all processed documents from the output of the data processing pipeline and
    // write these to the database table
    let mut all_docs_processed: Vec<document::Document> = Vec::new();
    let mut last_written: usize = 0;

    for processed_docinfo in processed_data_rx {
        all_docs_processed.push(processed_docinfo);
        // update shared status counter
        if let Some(ref st) = status_tracker {
            if let Ok(mut s) = st.lock() {
                s.docs_processed = all_docs_processed.len();
            }
        }
        // write to database after every 20 urls:
        if all_docs_processed.len() % 20 == 0 {
            let current_idx = all_docs_processed.len();
            let written_rows = utils::insert_urls_info_to_database(
                app_config.clone(),
                &all_docs_processed[last_written..current_idx]
            );
            info!("Wrote {} retrieved urls into the 'completed_urls' table.", written_rows);
            if written_rows < (current_idx - last_written) {
                error!("Could not write all {} of the retrieved urls into the table, wrote {}.",
                    all_docs_processed.len(), written_rows);
            }
            last_written = current_idx;
        }
    }
    let current_idx = all_docs_processed.len();
    if utils::insert_urls_info_to_database(app_config, &all_docs_processed[last_written..current_idx]) < (current_idx - last_written) {
        error!("Could not write all of the retrieved urls into database table.");
    }
    // Join retriever threads for a clean shutdown. By the time the collector loop above has
    // ended, every retriever has dropped its sender, so these joins return promptly.
    for handle in retriever_thread_handles {
        if let Err(e) = handle.join() {
            error!("start_data_pipeline: a retriever thread panicked: {:?}", e);
        }
    }

    // mark pipeline as finished
    if let Some(ref st) = status_tracker {
        if let Ok(mut s) = st.lock() {
            s.is_running = false;
            s.docs_processed = all_docs_processed.len();
        }
    }
    return all_docs_processed;
}

fn start_retrieval_pipeline(plugins: Vec<RetrieverPlugin>, tx: Sender<document::Document>, config: Arc<config::Config>) -> Vec<JoinHandle<()>>{

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
    fn test_priority_queue(){
        let mut plugin_heap: BinaryHeap<DataProcPlugin> = BinaryHeap::new();
        let plugin1 = DataProcPlugin{ name: "plugin1".to_string(), priority: 10, enabled: true, api_mutexes: Default::default(), method: split_text::process_data};
        let plugin2 = DataProcPlugin{ name: "plugin2".to_string(), priority: -20, enabled: true, api_mutexes: Default::default(), method: split_text::process_data};
        let plugin3 = DataProcPlugin{ name: "plugin3".to_string(), priority: 2, enabled: true, api_mutexes: Default::default(), method: split_text::process_data};
        plugin_heap.push(plugin1);
        plugin_heap.push(plugin2);
        plugin_heap.push(plugin3);
        if let Some( DataProcPlugin{ priority, enabled, method, api_mutexes, name }) = plugin_heap.pop() {
            println!("1st item, got priority = {}", priority);
            assert_eq!(priority, -20, "Invalid min heap/priority queue processing");
        }
        if let Some( DataProcPlugin{ priority, enabled, method, api_mutexes, name }) = plugin_heap.pop() {
            println!("2nd item, got priority = {}", priority);
            assert_eq!(priority, 2, "Invalid min heap/priority queue processing");
        }
        if let Some( DataProcPlugin{ priority, enabled, method, api_mutexes, name }) = plugin_heap.pop() {
            println!("3rd item, got priority = {}", priority);
            assert_eq!(priority, 10, "Invalid min heap/priority queue processing");
        }
    }
}
