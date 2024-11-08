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
//! ```no_run
//! use std::env;
//! use newslookout::run_app;
//!
//! # fn main() {
//!     if env::args().len() < 2 {
//!         println!("Usage: newslookout_app <config_file>");
//!         panic!("Provide config file as parameter in the command line, (need 2 parameters, got {})",
//!                env::args().len()
//!         );
//!     }
//!
//!     let configfile = env::args().nth(1).unwrap();
//!
//!     run_app(configfile);
//! # }
//! ```
//! Refer to the README file for more information.

use std::env;

mod document;
mod plugins {
    pub(crate) mod mod_en_in_rbi;
    pub(crate) mod mod_en_in_business_standard;
    pub(crate) mod mod_offline_docs;
    pub(crate) mod mod_classify;
    pub(crate) mod mod_dataprep;
    pub(crate) mod mod_dedupe;
    pub(crate) mod mod_ollama;
    pub(crate) mod mod_solrsubmit;
}
mod network;
mod utils;
mod queue;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const CARGO_PKG_NAME: &str = env!("CARGO_PKG_NAME");


pub fn run_app(configfile: String){

    println!("{} application, v{}\nReading configuration from: {}", CARGO_PKG_NAME.to_uppercase(), VERSION, configfile);

    let config = utils::read_config(configfile);

    utils::init_application(&config);

    let count_docs = queue::start_pipeline(config.clone());

    log::info!("Completed processing {} documents.", count_docs);

    utils::shutdown_application(&config);
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_1() {
        // TODO: implement this
        assert_eq!(1, 1);
    }
}