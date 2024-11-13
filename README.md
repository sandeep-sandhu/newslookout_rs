
# Newslookout

[![build](https://github.com/sandeep-sandhu/newslookout_rs/actions/workflows/rust.yml/badge.svg)](https://github.com/sandeep-sandhu/newslookout_rs/actions) ![Crates.io Downloads (latest version)](https://img.shields.io/crates/dv/newslookout) ![Crate version](https://img.shields.io/crates/v/newslookout.svg)

A light-weight web scraping platform built for scanning and processing news and data. It is a rust port of the python [application of the same name](https://github.com/sandeep-sandhu/NewsLookout).

## Architecture

This library sets up a web scraping pipeline and executes it as follows:
  - Starts the web retriever modules in its own separate thread that run parallely to get the content from the respective websites
  - Each page's content is populated into a document struct and transmitted by the web retriever module threads to the data processing chain.
  - Simultaneously the data processing modules are started (which form the data processing chain). The retrieved documents are passed to these threads in serial order, based on the priority configured for each data processing module.
  - Each data processing module processes the content and may add or modify the document it receives. It then passes it on to the next data processing thread in order of priority
  - At then end, the document is written to disk as a json file
  - The retrieved URLs are saved to an SQLite database table to serve as a reference so these are not retrieved again in the next run.
  - Adequate wait times are configured during web retrieval to avoid overloading the target website. All events and actions are logged to a central log file. Multiple instances are prevented by writing and checking for a PID file. Although, if desired multiple instances can be launched by running the application with separate config files.

This package enables building a full-fledged multi-threaded web scraping solution that runs in batch mode with very meagre resources (e.g. single core CPU with less than 4GB RAM).

## Quick Start
Add this to your Cargo.toml:
[dependencies]
newslookout = "0.2.1"

## Usage

Get started with just a few lines of code, for example:

```
use std::env;
use config;
use newslookout;

fn main() {

    if env::args().len() < 2 {
        println!("Usage: newslookout_app <config_file>");
        panic!("Provide config file as a command line parameter, (expect 2 parameters, but got {})",
               env::args().len()
        );
    }

    let config_file: String = env::args().nth(1).unwrap();
    println!("Loading configuration from file: {}", config_file);
    let app_config: config::Config = newslookout::utils::read_config(config_file);

    let docs_retrieved: Vec<newslookout::document::DocInfo> = newslookout::run_app(app_config);
    // use this collection of retrieved document-information structs for any further custom processing

}

```

## Configuration

The entire application is driven by its config file.

There are a few pre-built modules provided for a few websites.
These can be readily extended for other websites as required.
Refer to the source code of these in the plugins folder and roll out your own plugins.
