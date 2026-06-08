//! # Ready-to-use web-scraping, data processing and NLP pipelines
//!
//! Rust-native state-of-the-art library for simplifying web scraping of news and public data. Port of a previous python application [NewsLookout Package](https://github.com/sandeep-sandhu/NewsLookout).
//!
//! This package enables building a full-fledged multi-threaded web scraping solution that runs in batch mode with very meagre resources (e.g. single core CPU with less than 4GB RAM).
//! It is primarily driven by configuration specified in a config file and intended to be invoked in batch mode.
//!
//! This library is the main entry point for the package, it loads the config, initialises the workers and starts the scraping pipeline.
//!
//! Here is an illustration of this multi-threaded pipeline:
//!
//!<svg
//!    width="125.30934mm"
//!    height="43.364216mm"
//!    viewBox="0 0 125.30934 43.364216"
//!    version="1.1"
//!    id="svg1"
//!    xmlns="http://www.w3.org/2000/svg"
//!    xmlns:svg="http://www.w3.org/2000/svg">
//!   <defs
//!      id="defs1" />
//!   <g
//!      id="layer1"
//!      transform="translate(-6.7743092,-48.133257)">
//!     <rect
//!        style="fill:#cccccc;stroke-width:0.232031"
//!        id="rect1"
//!        width="29.065943"
//!        height="12.122448"
//!        x="6.7743096"
//!        y="48.133255" />
//!     <text
//!        xml:space="preserve"
//!        style="font-size:3.175px;text-align:start;writing-mode:lr-tb;direction:ltr;text-anchor:start;fill:#000000;stroke-width:0.264583"
//!        x="12.835534"
//!        y="55.264103"
//!        id="text1"><tspan
//!          id="tspan1"
//!          style="fill:#000000;stroke-width:0.264583"
//!          x="12.835534"
//!          y="55.264103">Retriever 1</tspan></text>
//!     <rect
//!        style="fill:#cccccc;stroke-width:0.244907"
//!        id="rect1-7"
//!        width="32.381348"
//!        height="12.122448"
//!        x="59.523899"
//!        y="62.618053" />
//!     <text
//!        xml:space="preserve"
//!        style="font-size:3.175px;text-align:start;writing-mode:lr-tb;direction:ltr;text-anchor:start;fill:#000000;stroke-width:0.264583"
//!        x="62.451218"
//!        y="68.082199"
//!        id="text1-6"><tspan
//!          id="tspan1-14"
//!          style="fill:#000000;stroke-width:0.264583"
//!          x="62.451218"
//!          y="68.082199">Data Processing</tspan><tspan
//!          style="fill:#000000;stroke-width:0.264583"
//!          x="62.451218"
//!          y="72.050949"
//!          id="tspan3"> Module  1</tspan></text>
//!     <rect
//!        style="fill:#cccccc;stroke-width:0.244907"
//!        id="rect1-7-2"
//!        width="32.381348"
//!        height="12.122448"
//!        x="99.702309"
//!        y="62.667603" />
//!     <text
//!        xml:space="preserve"
//!        style="font-size:3.175px;text-align:start;writing-mode:lr-tb;direction:ltr;text-anchor:start;fill:#000000;stroke-width:0.264583"
//!        x="102.62962"
//!        y="68.131744"
//!        id="text1-6-1"><tspan
//!          id="tspan1-14-6"
//!          style="fill:#000000;stroke-width:0.264583"
//!          x="102.62962"
//!          y="68.131744">Data Processing</tspan><tspan
//!          style="fill:#000000;stroke-width:0.264583"
//!          x="102.62962"
//!          y="72.100494"
//!          id="tspan3-8"> Module  2</tspan></text>
//!     <rect
//!        style="fill:#cccccc;stroke-width:0.232031"
//!        id="rect1-8"
//!        width="29.065943"
//!        height="12.122448"
//!        x="6.8791666"
//!        y="63.500008" />
//!     <text
//!        xml:space="preserve"
//!        style="font-size:3.175px;text-align:start;writing-mode:lr-tb;direction:ltr;text-anchor:start;fill:#000000;stroke-width:0.264583"
//!        x="12.940389"
//!        y="70.630852"
//!        id="text1-8"><tspan
//!          id="tspan1-2"
//!          style="fill:#000000;stroke-width:0.264583"
//!          x="12.940389"
//!          y="70.630852">Retriever 2</tspan></text>
//!     <rect
//!        style="fill:#cccccc;stroke-width:0.232031"
//!        id="rect1-1"
//!        width="29.065943"
//!        height="12.122448"
//!        x="6.8791666"
//!        y="79.375023" />
//!     <text
//!        xml:space="preserve"
//!        style="font-size:3.175px;text-align:start;writing-mode:lr-tb;direction:ltr;text-anchor:start;fill:#000000;stroke-width:0.264583"
//!        x="12.940389"
//!        y="86.505867"
//!        id="text1-7"><tspan
//!          id="tspan1-1"
//!          style="fill:#000000;stroke-width:0.264583"
//!          x="12.940389"
//!          y="86.505867">Retriever 3</tspan></text>
//!     <path
//!        style="display:inline;fill:none;fill-rule:evenodd;stroke:#000000;stroke-width:0.228792px;stroke-linecap:butt;stroke-linejoin:miter;stroke-opacity:1"
//!        d="m 35.840252,54.194479 h 34.582654 c 2.645834,0 5.291667,2.645833 5.291667,5.291667 v 3.131907"
//!        id="path1" />
//!     <path
//!        style="display:inline;fill:none;fill-rule:evenodd;stroke:#000000;stroke-width:0.079272px;stroke-linecap:butt;stroke-linejoin:miter;stroke-opacity:1"
//!        d="m 35.945109,69.325194 23.57879,-0.382955"
//!        id="path2" />
//!     <path
//!        style="display:inline;fill:none;fill-rule:evenodd;stroke:#000000;stroke-width:0.264583px;stroke-linecap:butt;stroke-linejoin:miter;stroke-opacity:1"
//!        d="m 35.945109,85.436247 h 34.477797 c 2.645834,0 5.291667,-2.645833 5.291667,-5.291667 v -5.404079"
//!        id="path3" />
//!     <path
//!        style="display:inline;fill:none;fill-rule:evenodd;stroke:#000000;stroke-width:0.264583px;stroke-linecap:butt;stroke-linejoin:miter;stroke-opacity:1"
//!        d="m 91.905247,68.699244 7.797062,0.0096"
//!        id="path4" />
//!   </g>
//! </svg>
//!
//!
//! ## Architecture
//!
//! This library sets up a web scraping pipeline and executes it as follows:
//! - Starts the web retriever modules in its own separate thread that run parallely to get the content from the respective websites
//! - Each page's content is populated into a document struct and transmitted by the web retriever module threads to the data processing chain.
//! - Simultaneously the data processing modules are started in their own threads (which form the data processing chain). The retrieved documents are passed to these threads in serial order, based on the priority configured for each data processing module.
//! - Each data processing module processes the content and may add or modify the document it receives. It then passes it on to the next data processing thread in order of priority
//! - Popular LLM services are supported by the data processing pipelines such as - **ChatGPT, Google Gemini** and self-hosted LLMs using **Ollama**. The relevant API keys need to be configured as environment variables before using these plugins.
//! - At then end, the document is written to disk as a json file
//! - The retrieved URLs are saved to an SQLite database table to serve as a reference so these are not retrieved again in the next run.
//! - Adequate wait times are configured during web retrieval to avoid overloading the target website. All events and actions are logged to a central log file. Multiple instances are prevented by writing and checking for a PID file. Although, if desired multiple instances can be launched by running the application with separate config files.
//!
//!  ## Get Started
//! Get started using this crate in just a few lines of code, for example:
//!
//! <tt>
//! use std::env;<br/>
//! use newslookout::run_app;<br/>
//!<br/>
//! fn main() {<br/>
//!     if env::args().len() < 2 {<br/>
//!         println!("Usage: newslookout_app <config_file>");<br/>
//!         panic!("Provide config file as parameter in the command line, (need 2 parameters, got {})",<br/>
//!                env::args().len()<br/>
//!         );<br/>
//!     }<br/>
//!<br/>
//!     let configfile = env::args().nth(1).unwrap();<br/>
//!<br/>
//!    println!("Loading configuration from file: {}", config_file);<br/>
//!    let app_config: config::Config = newslookout::utils::read_config(config_file);<br/>
//!<br/>
//!    let docs_retrieved: Vec &lt; newslookout::document::DocInfo &gt; = newslookout::run_app(app_config);<br/>
//!    // use this collection of retrieved documents information for any further custom processing<br/>
//! }<br/>
//! </tt>
//!
//!
//! ## Create your own custom plugins and run these in the Pipeline
//! 
//! Declare custom retriever plugin and add these to the pipeline to fetch data using your custom logic.
//! 
//! <tt>
//! fn run_pipeline(config: &config::Config) -> Vec<Document> {<br/>
//! <br/>
//!     newslookout::init_logging(config);<br/>
//!     newslookout::init_pid_file(config);<br/>
//!     log::info!("Starting the custom pipeline");<br/>
//! <br/>
//!     let mut retriever_plugins = newslookout::pipeline::load_retriever_plugins(config);<br/>
//!     let mut data_proc_plugins = newslookout::pipeline::load_dataproc_plugins(config);<br/>
//! <br/>
//!     // add custom data retriever:<br/>
//!     retriever_plugins.push(my_plugin);<br/>
//!     let docs_retrieved = newslookout::pipeline::start_data_pipeline(<br/>
//!         retriever_plugins,<br/>
//!         data_proc_plugins,<br/>
//!         config<br/>
//!     );<br/>
//!     log::info!("Data pipeline completed processing {} documents.", docs_retrieved.len());<br/>
//!     // use docs_retrieved for any further custom processing.<br/>
//! <br/>
//!     newslookout::cleanup_pid_file(&config);<br/>
//! }<br/>
//! </tt>
//!
//!
//! Similarly, you can also declare and use custom data processing plugins, e.g.:
//!
//! <tt>
//!     data_proc_plugins.push(my_own_data_processing);
//! </tt>
//!
//! Note that all data processing plugins are run in the serial order of priority as defined in the config file.
//!
//!
//! There are a few pre-built modules provided for a few websites.
//! These can be readily extended for other websites as required.
//! 
//! Refer to the README file and the source code of these in the plugins folder and roll out your own plugins.
//!

use std::collections::HashMap;
use std::env;
use std::io::Write;
use std::sync::{Arc, Mutex};
use ::config::Config;
use log::{info, LevelFilter};
use log4rs::append::rolling_file::policy::compound::CompoundPolicy;
use log4rs::append::rolling_file::policy::compound::roll::fixed_window::FixedWindowRoller;
use log4rs::append::rolling_file::policy::compound::trigger::size::SizeTrigger;
use log4rs::append::rolling_file::RollingFileAppender;
use log4rs::encode::pattern::PatternEncoder;
use log4rs::config::{Appender, Logger, Root};
use log4rs::filter::threshold::ThresholdFilter;
use crate::pipeline::{load_dataproc_plugins, load_retriever_plugins, RetrieverPlugin, start_data_pipeline, create_api_mutexes};

pub mod plugins {
    pub mod html_news;
    pub mod mod_en_in_indiankanoon;
    pub(crate) mod mod_en_in_rbi;
    pub(crate) mod mod_en_in_business_standard;
    pub mod mod_offline_docs;
    pub mod split_text;
    pub mod mod_dedupe;
    pub mod mod_mentions;
    pub mod mod_extract_quant;
    pub mod mod_themes;
    pub mod mod_tone;
    pub mod mod_geocode;
    pub mod mod_ner;
    pub mod mod_entity_graph;
    pub mod mod_emit_graph;
    pub mod mod_emit_tables;
    pub mod mod_vectorstore;
    pub mod mod_summarize;
    pub mod mod_solrsubmit;
    pub mod mod_persist_data;
    pub mod mod_cmdline;
    pub mod mod_en_in_thehindu;
    pub mod mod_en_in_livemint;
    pub mod mod_en_in_moneycontrol;
    pub mod mod_en_in_timesofindia;
    pub mod mod_en_in_forbes;
    pub mod mod_en_bbc;
    pub mod mod_en_guardian;
    pub mod mod_en_ap_news;
    pub mod mod_en_in_indianexpress;
    pub mod mod_en_in_generic_retriever;
    pub mod mod_en_in_hindustan_times;
    pub mod mod_en_in_news18;
    pub mod mod_en_aljazeera;
    pub mod mod_en_nhk_world;
    pub mod mod_en_arab_news;
    pub mod mod_en_gulf_news;
    pub mod mod_en_khaleej_times;
    pub mod mod_en_the_national;
    pub mod mod_en_punch_ng;
    pub mod mod_en_allafrica;
    pub mod mod_en_cnn;
    pub mod mod_en_foxnews;
    pub mod mod_en_cnbc;
    pub mod mod_en_business_insider;
    pub mod mod_en_latimes;
    pub mod mod_en_chicago_tribune;
    pub mod mod_en_fortune;
    pub mod mod_en_techcrunch;
    pub mod mod_en_wired;
    pub mod mod_en_theverge;
    pub mod mod_en_arstechnica;
    pub mod mod_en_cnet;
    pub mod mod_en_sg_straitstimes;
    pub mod mod_en_sg_cna;
    pub mod mod_en_th_bangkokpost;
    pub mod mod_en_ca_cbc;
    pub mod mod_en_ca_globeandmail;
    pub mod mod_en_au_smh;
    pub mod mod_en_au_abc;
    pub mod mod_en_in_irdai;
    pub mod mod_en_in_sebi;
    pub mod mod_in_nse;
    pub(crate) mod mod_in_bse;
    pub mod mod_doc_type;
    pub mod mod_filter;
    pub mod mod_metadata;
}

pub mod network;
pub mod discovery;
pub mod utils;
pub mod llm;
pub mod document;
pub mod analysis;
pub mod store;
pub mod metrics;
pub mod feeds;
pub mod pipeline;
pub mod cfg;
pub mod content_extraction;
pub mod web_api;
pub mod market_data;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const CARGO_PKG_NAME: &str = env!("CARGO_PKG_NAME");


/// Runs the web scraping application plugins. Refers to the config object passed as the parameter.
/// Initialises the logging, PID and multi-threaded web scraping modules as well as the data
/// processing modules of the pipeline. All of these are configured and enabled via the config
/// file.
///
/// # Arguments
///
/// * `config`: The configuration object loaded from the configuration file.
///
/// returns: ()
///
/// # Examples
///
/// use newslookout;<br/>
/// let config = utils::read_config(configfile);<br/>
/// newslookout::run_app(config);</tt>
///
pub fn load_and_run_pipeline(config: Config) -> Vec<document::Document> {
    let configref: Arc<config::Config> = Arc::new(config);

    init_pid_file(configref.clone());
    init_logging(configref.clone());

    log::info!("Starting the data pipeline, library v{}", VERSION);
    let all_api_mutexes: HashMap<String, Arc<Mutex<isize>>> = create_api_mutexes();

    let retriever_plugins = load_retriever_plugins(configref.clone());
    let data_proc_plugins = load_dataproc_plugins(configref.clone(), all_api_mutexes);

    let docs_retrieved = start_data_pipeline(
        retriever_plugins,
        data_proc_plugins,
        configref.clone(),
        None,
    );

    log::info!("Data pipeline completed processing {} documents.", docs_retrieved.len());

    cleanup_pid_file(configref);

    return docs_retrieved;
}


/// Initialise the application by configuring the log file, and
/// setting the PID file to prevent duplicate instances form running simultaneously.
///
/// # Arguments
///
/// * `config`: The application's configuration object
///
/// returns: ()
///
pub fn init_logging(config: Arc<config::Config>){
    // setup logging:
    match config.get_string("log_file"){
        Ok(logfile) =>{

            //set loglevel parameter from log file:
            let mut app_loglevel = LevelFilter::Info;
            match config.get_string("log_level"){
                Ok(loglevel_str) =>{
                    match loglevel_str.as_str() {
                        "DEBUG" => app_loglevel = LevelFilter::Debug,
                        "INFO" => app_loglevel = LevelFilter::Info,
                        "WARN" => app_loglevel = LevelFilter::Warn,
                        "ERROR" => app_loglevel = LevelFilter::Error,
                        _other => println!("Unknown log level '{}' in config, defaulting to INFO", _other)
                    }
                },
                Err(e) => println!("Could not read log_level from config: {}", e)
            }

            // Create parent log directory if it does not exist
            if let Some(parent) = std::path::Path::new(&logfile).parent() {
                if !parent.as_os_str().is_empty() && !parent.exists() {
                    match std::fs::create_dir_all(parent) {
                        Ok(_) => println!("Created log directory: {}", parent.display()),
                        Err(e) => {
                            println!("ERROR: Could not create log directory '{}': {}. Logging to stderr only.", parent.display(), e);
                            return;
                        }
                    }
                }
            }

            // read parameter from config file : max_logfile_size
            let mut size_limit: i64 = 10 * 1024 * 1024; // 10 MB
            match config.get_int("max_logfile_size") {
                Ok(max_logfile_size) => size_limit = max_logfile_size,
                Err(_) => {}
            }
            let mut backup_count: u32 = 10;
            match config.get_int("logfile_backup_count") {
                Ok(count) => backup_count = count as u32,
                Err(_) => {}
            }

            // rolling file appender: base = logfile, backup pattern = logfile.{N}
            let roller_pattern = format!("{}.{{}}", logfile);
            let fixed_window_roller = match FixedWindowRoller::builder()
                .build(roller_pattern.as_str(), backup_count)
            {
                Ok(r) => r,
                Err(e) => { println!("ERROR: Could not create log file roller: {}", e); return; }
            };
            let size_trigger = SizeTrigger::new(size_limit as u64);
            let compound_policy = CompoundPolicy::new(
                Box::new(size_trigger),
                Box::new(fixed_window_roller)
            );
            let rolling_appender = match RollingFileAppender::builder()
                .encoder(Box::new(PatternEncoder::new("{d(%Y-%m-%d %H:%M:%S)(local)} {i} [{l}] - {m}{n}")))
                .build(logfile.clone(), Box::new(compound_policy))
            {
                Ok(a) => a,
                Err(e) => { println!("ERROR: Could not create rolling log appender for '{}': {}", logfile, e); return; }
            };

            // Noisy third-party crates emit WARN-level messages during HTML parsing
            // (e.g. html5ever's "foster parenting not implemented", which accounted for
            // ~79% of all warnings in production logs). Cap these dependencies at ERROR
            // so they never reach the appender, regardless of the app log level.
            let noisy_modules = ["html5ever", "markup5ever", "selectors", "html5ever::tree_builder"];

            let mut logconfig_builder = log4rs::config::Config::builder()
                .appender(
                    Appender::builder()
                        .filter(Box::new(ThresholdFilter::new(app_loglevel)))
                        .build("logfile", Box::new(rolling_appender))
                );
            for module in noisy_modules {
                logconfig_builder = logconfig_builder.logger(
                    Logger::builder().build(module, LevelFilter::Error)
                );
            }

            let logconfig = match logconfig_builder
                .build(
                    Root::builder()
                        .appender("logfile")
                        .build(app_loglevel)
                )
            {
                Ok(c) => c,
                Err(e) => { println!("ERROR: Could not build logging config: {}", e); return; }
            };

            match log4rs::init_config(logconfig) {
                Ok(_) => {},
                Err(e) => { println!("ERROR: Could not initialize logging: {}", e); return; }
            }
            log::info!("Started application.");
            println!("Started logging to file: {}", logfile);
        }
        Err(e) => {
            println!("ERROR: Could not read 'log_file' from config: {}. No log file will be written.", e);
        }
    }
}

pub fn init_pid_file(config: Arc<config::Config>){
    // setup PID file:
    match config.get_string("pid_file"){
        Ok(pidfile_name) =>{
            let pid_path = std::path::Path::new(&pidfile_name);
            // Report the resolved absolute path of the PID file so the operator knows
            // exactly which file is in use (config values are often relative).
            let pid_abs = std::path::absolute(pid_path)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| pidfile_name.clone());
            println!("Using PID file: {}", pid_abs);
            // Create parent directory if needed
            if let Some(parent) = pid_path.parent() {
                if !parent.as_os_str().is_empty() && !parent.exists() {
                    let _ = std::fs::create_dir_all(parent);
                }
            }

            if pid_path.exists() {
                // Read the stored PID and check whether that process is still alive
                let stale = match std::fs::read_to_string(&pidfile_name) {
                    Ok(contents) => {
                        match contents.trim().parse::<u32>() {
                            Ok(stored_pid) => {
                                // On Linux, /proc/<pid> exists iff the process is alive
                                let alive = std::path::Path::new(&format!("/proc/{}", stored_pid)).exists();
                                if alive {
                                    println!("ERROR: Another instance is already running (PID {}). PID file: {}", stored_pid, pidfile_name);
                                    std::process::exit(1);
                                }
                                true // process is gone — PID file is stale
                            }
                            Err(_) => true // unreadable PID — treat as stale
                        }
                    }
                    Err(_) => true // can't read file — treat as stale
                };

                if stale {
                    println!("WARNING: Removing stale PID file: {}", pidfile_name);
                    let _ = std::fs::remove_file(&pidfile_name);
                }
            }

            // Write current PID
            let pid = std::process::id();
            match std::fs::File::create(&pidfile_name) {
                Ok(output) => {
                    match write!(&output, "{}", pid) {
                        Ok(_) => info!("Initialised PID file: {}, process id={}", pidfile_name, pid),
                        Err(err) => {
                            println!("ERROR: Could not write to PID file '{}': {}", pidfile_name, err);
                            std::process::exit(1);
                        }
                    }
                }
                Err(e) => {
                    println!("ERROR: Cannot create PID file '{}': {}", pidfile_name, e);
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            println!("WARNING: Could not read 'pid_file' from config: {}. Skipping PID file.", e);
        }
    }
}

/// Shuts down the application by performing any cleanup required.
///
/// # Arguments
///
/// * `config`: The application's configuration object
///
/// returns: ()
///
pub fn cleanup_pid_file(config: Arc<config::Config>){
    match config.get_string("pid_file"){
        Ok(pidfile) =>{
            match std::fs::remove_file(&pidfile) {
                Ok(_result) => {
                    log::debug!("Cleaning PID file: {:?}", pidfile);
                }
                Err(e) => {
                    log::error!("Could not remove PID: {}", e);
                }
            }
        }
        Err(e) => {
            log::error!("Could not remove PID: {}", e);
        }
    }
    log::info!("Shutting down the application.");
}

#[cfg(test)]
mod tests {
    use std::io::empty;
    use crate::load_and_run_pipeline;

    #[test]
    fn test_1() {
        let empty_cfg = config::Config::builder().build().unwrap();
        let docs = load_and_run_pipeline(empty_cfg);
        assert_eq!(docs.len(), 0);
    }
}
