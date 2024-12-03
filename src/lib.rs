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


use std::env;
use std::io::Write;
use ::config::Config;
use log::{error, info, LevelFilter};
use log4rs::append::file::FileAppender;
use log4rs::append::rolling_file::policy::compound::CompoundPolicy;
use log4rs::append::rolling_file::policy::compound::roll::fixed_window::FixedWindowRoller;
use log4rs::append::rolling_file::policy::compound::trigger::size::SizeTrigger;
use log4rs::append::rolling_file::RollingFileAppender;
use log4rs::encode::pattern::PatternEncoder;
use log4rs::config::{Appender, Root};
use log4rs::filter::threshold::ThresholdFilter;
use crate::pipeline::{load_dataproc_plugins, load_retriever_plugins, RetrieverPlugin, start_data_pipeline};

pub mod plugins {
    pub mod mod_en_in_indiankanoon;
    pub(crate) mod rbi;
    pub(crate) mod mod_en_in_business_standard;
    pub mod mod_offline_docs;
    pub mod mod_classify;
    pub mod split_text;
    pub mod mod_dedupe;
    pub mod mod_vectorstore;
    pub mod mod_summarize;
    pub mod mod_solrsubmit;
    pub mod mod_persist_data;
    pub mod mod_cmdline;
}

pub mod network;
pub mod utils;
pub mod llm;
pub mod document;
pub mod html_extract;
pub mod pipeline;
pub mod cfg;

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
        Ok(mut logfile) =>{

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
            // let size_trigger = SizeTrigger::new(size_limit as u64);
            // let compound_policy = CompoundPolicy::new(Box::new(size_trigger),Box::new(fixed_window_roller));
            // let config = log4rs::config::Config::builder().appender(
            //         Appender::builder()
            //             .filter(Box::new(ThresholdFilter::new(LevelFilter::Info)))
            //             .build(
            //                 "logfile",
            //                 Box::new(
            //                     RollingFileAppender::builder()
            //                         .encoder(Box::new(PatternEncoder::new("{d} {l}::{m}{n}")))
            //                         .build(logfile.clone(), Box::new(compound_policy)),
            //                 ),
            //             ),
            //     )
            //     .build(
            //         Root::builder()
            //             .appender("logfile")
            //             .build(LevelFilter::Info),
            //     ).expect("Valid log configuration.");

            let logfile = FileAppender::builder()
                .encoder(Box::new(PatternEncoder::new("{d(%Y-%m-%d %H:%M:%S)(local)} {i} [{l}] - {m}{n}")))
                .build(logfile.clone())
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
