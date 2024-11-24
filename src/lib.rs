//! # Ready-to-use web-scraping, data processing and NLP pipelines
//!
//! Rust-native state-of-the-art library for simplifying web scraping of news and public data. Port of a previous python application [NewsLookout Package](https://github.com/sandeep-sandhu/NewsLookout).
//!
//! This rust crate contains the newslookout package.
//! It is primarily driven by configuration specified in a config file and intended to be invoked in batch mode.
//!
//! This library is the main entry point for the package, it loads the config, initialises the workers and starts the scraping pipeline.
//!
//! Get started with using this in just a few lines of code:
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
//! Refer to the README file for more information.

use std::env;
use std::io::Write;
use config::Config;
use log::{error, info, LevelFilter};
use log4rs::append::file::FileAppender;
use log4rs::config::{Appender, Root};
use log4rs::encode::pattern::PatternEncoder;
use crate::pipeline::{load_dataproc_plugins, load_retriever_plugins, RetrieverPlugin, start_data_pipeline};

pub mod plugins {
    pub(crate) mod rbi;
    pub(crate) mod mod_en_in_business_standard;
    pub mod mod_offline_docs;
    pub mod mod_classify;
    pub(crate) mod split_text;
    pub mod mod_dedupe;
    pub mod mod_vectorstore;
    pub mod mod_summarize;
    pub mod mod_solrsubmit;
    pub mod mod_persist_data;
}

pub mod network;
pub mod utils;
pub mod llm;
pub mod document;
pub mod html_extract;
pub mod pipeline;


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
pub fn load_and_run_pipeline(config: config::Config) -> Vec<document::Document> {

    init_logging(&config);
    init_pid_file(&config);
    log::info!("Starting the data pipeline, library v{}", VERSION);

    let retriever_plugins = load_retriever_plugins(&config);
    let data_proc_plugins = load_dataproc_plugins(&config);

    let docs_retrieved = start_data_pipeline(retriever_plugins,
                                                       data_proc_plugins,
                                                       &config);

    log::info!("Data pipeline completed processing {} documents.", docs_retrieved.len());

    cleanup_pid_file(&config);

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
pub fn init_logging(config: &Config){
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
                        _other => error!("Unknown log level configured in logfile: {}", _other)
                    }
                },
                Err(e) => error!("When getting the log level: {}", e)
            }
            println!("Logging to file: {:?}", logfile);

            // read parameter from config file : max_logfile_size
            let mut size_limit =10 * 1024 * 1024; // 10 MB
            match config.get_int("max_logfile_size") {
                Ok(max_logfile_size) => size_limit = max_logfile_size,
                Err(e) => error!("When reading max logfile size: {}", e)
            }
            // TODO: implement log rotation
            // logfile.push_str("{}");
            // let window_size = 10;
            // let fixed_window_roller =
            //     FixedWindowRoller::builder().build(logfile.as_str(), window_size).unwrap();
            // let size_trigger = SizeTrigger::new(size_limit);
            // let compound_policy = CompoundPolicy::new(Box::new(size_trigger),Box::new(fixed_window_roller));
            // let config = log4rs::config::Config::builder().appender(
            //         Appender::builder()
            //             .filter(Box::new(ThresholdFilter::new(LevelFilter::Info)))
            //             .build(
            //                 "logfile",
            //                 Box::new(
            //                     RollingFileAppender::builder()
            //                         .encoder(Box::new(PatternEncoder::new("{d} {l}::{m}{n}")))
            //                         .build(logfile, Box::new(compound_policy)),
            //                 ),
            //             ),
            //     )
            //     .build(
            //         Root::builder()
            //             .appender("logfile")
            //             .build(LevelFilter::Info),
            //     )?;

            let logfile = FileAppender::builder()
                .encoder(Box::new(PatternEncoder::new("{d(%Y-%m-%d %H:%M:%S)(local)} {i} [{l}] - {m}{n}")))
                .build(logfile)
                .expect("Cound not init log file appender.");

            let logconfig = log4rs::config::Config::builder()
                .appender(Appender::builder().build("logfile", Box::new(logfile)))
                .build(Root::builder()
                    .appender("logfile")
                    .build(app_loglevel))
                .expect("Cound not build a logging config.");

            log4rs::init_config(logconfig).expect("Cound not initialize logging.");
            log::info!("Started application.");
        }
        Err(e) => {
            println!("Could not start logging {}", e);
        }
    }
}

pub fn init_pid_file(config: &Config){
    // setup PID file:
    match config.get_string("pid_file"){
        Ok(pidfile_name) =>{
            //get process id
            let pid = std::process::id();
            // check file exists
            let file_exists = std::path::Path::new(&pidfile_name).exists();
            // write pid if it does not exist
            if file_exists==false {
                match std::fs::File::create(&pidfile_name) {
                    Ok(output) => {
                        match write!(&output, "{:?}", pid) {
                            Ok(_res) => info!("Initialised PID file: {:?}, with process id={}", pidfile_name, pid),
                            Err(err) => panic!("Could not write to PID file: {:?}, error: {}", pidfile_name, err)
                        }
                    }
                    Err(e) => {
                        error!("Cannot initialise PID file: {:?}, error: {}", pidfile_name, e);
                        std::process::exit(0x0100);
                    }
                }
            }
            else{
                // throw panic if it exists
                panic!("Cannot initialise application since the PID file {:?} already exists", pidfile_name);
            }
        }
        Err(e) => {
            println!("Could not init PID: {}", e);
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
pub fn cleanup_pid_file(config: &Config){
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
