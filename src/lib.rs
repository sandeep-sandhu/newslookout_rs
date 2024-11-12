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
//!    let docs_retrieved: Vec<newslookout::document::DocInfo> = newslookout::run_app(app_config);<br/>
//!    // use this collection of retrieved documents information for any further custom processing<br/>
//! }<br/>
//! </tt>
//! Refer to the README file for more information.

use std::env;
use crate::document::DocInfo;

mod document;
pub mod plugins {
    pub(crate) mod mod_en_in_rbi;
    pub(crate) mod mod_en_in_business_standard;
    pub(crate) mod mod_offline_docs;
    pub(crate) mod mod_classify;
    pub(crate) mod mod_dataprep;
    pub(crate) mod mod_dedupe;
    pub mod mod_ollama;
    pub mod mod_chatgpt;
    pub mod mod_gemini;
    pub(crate) mod mod_solrsubmit;
    pub(crate) mod mod_save_to_disk;
}

pub mod network;
pub mod utils;
mod queue;

const VERSION: &str = env!("CARGO_PKG_VERSION");
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
pub fn run_app(config: config::Config) -> Vec<DocInfo> {

    utils::init_logging(&config);
    utils::init_pid_file(&config);
    log::info!("{} application, v{}", CARGO_PKG_NAME.to_uppercase(), VERSION);

    let docs_retrieved = queue::start_pipeline(config.clone());

    log::info!("Completed processing {} documents.", docs_retrieved.len());

    utils::cleanup_pid_file(&config);

    return docs_retrieved;
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_1() {
        assert_eq!(1, 1);
    }
}