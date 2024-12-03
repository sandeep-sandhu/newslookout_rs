
# Newslookout

[![build](https://github.com/sandeep-sandhu/newslookout_rs/actions/workflows/rust.yml/badge.svg)](https://github.com/sandeep-sandhu/newslookout_rs/actions) ![Crates.io Downloads (latest version)](https://img.shields.io/crates/dv/newslookout) ![Crate version](https://img.shields.io/crates/v/newslookout.svg)

A light-weight web scraping platform built for scanning and processing news and data. It is a rust port of the python [application of the same name](https://github.com/sandeep-sandhu/NewsLookout).

Here's an illustration of this multi-threaded data pipeline:

<svg width="125.30934mm" height="43.364216mm" viewBox="0 0 125.30934 43.364216" version="1.1" id="svg1" xmlns="http://www.w3.org/2000/svg" xmlns:svg="http://www.w3.org/2000/svg"> <defs id="defs1" /> <g id="layer1" transform="translate(-6.7743092,-48.133257)"> <rect style="fill:#cccccc;stroke-width:0.232031" id="rect1" width="29.065943" height="12.122448" x="6.7743096" y="48.133255" /> <text xml:space="preserve" style="font-size:3.175px;text-align:start;writing-mode:lr-tb;direction:ltr;text-anchor:start;fill:#000000;stroke-width:0.264583" x="12.835534" y="55.264103" id="text1"><tspan id="tspan1" style="fill:#000000;stroke-width:0.264583" x="12.835534" y="55.264103">Retriever 1</tspan></text> <rect style="fill:#cccccc;stroke-width:0.244907" id="rect1-7" width="32.381348" height="12.122448" x="59.523899" y="62.618053" /> <text xml:space="preserve" style="font-size:3.175px;text-align:start;writing-mode:lr-tb;direction:ltr;text-anchor:start;fill:#000000;stroke-width:0.264583" x="62.451218" y="68.082199" id="text1-6"><tspan id="tspan1-14" style="fill:#000000;stroke-width:0.264583" x="62.451218" y="68.082199">Data Processing</tspan><tspan style="fill:#000000;stroke-width:0.264583" x="62.451218" y="72.050949" id="tspan3"> Module 1</tspan></text> <rect style="fill:#cccccc;stroke-width:0.244907" id="rect1-7-2" width="32.381348" height="12.122448" x="99.702309" y="62.667603" /> <text xml:space="preserve" style="font-size:3.175px;text-align:start;writing-mode:lr-tb;direction:ltr;text-anchor:start;fill:#000000;stroke-width:0.264583" x="102.62962" y="68.131744" id="text1-6-1"><tspan id="tspan1-14-6" style="fill:#000000;stroke-width:0.264583" x="102.62962" y="68.131744">Data Processing</tspan><tspan style="fill:#000000;stroke-width:0.264583" x="102.62962" y="72.100494" id="tspan3-8"> Module 2</tspan></text> <rect style="fill:#cccccc;stroke-width:0.232031" id="rect1-8" width="29.065943" height="12.122448" x="6.8791666" y="63.500008" /> <text xml:space="preserve" style="font-size:3.175px;text-align:start;writing-mode:lr-tb;direction:ltr;text-anchor:start;fill:#000000;stroke-width:0.264583" x="12.940389" y="70.630852" id="text1-8"><tspan id="tspan1-2" style="fill:#000000;stroke-width:0.264583" x="12.940389" y="70.630852">Retriever 2</tspan></text> <rect style="fill:#cccccc;stroke-width:0.232031" id="rect1-1" width="29.065943" height="12.122448" x="6.8791666" y="79.375023" /> <text xml:space="preserve" style="font-size:3.175px;text-align:start;writing-mode:lr-tb;direction:ltr;text-anchor:start;fill:#000000;stroke-width:0.264583" x="12.940389" y="86.505867" id="text1-7"><tspan id="tspan1-1" style="fill:#000000;stroke-width:0.264583" x="12.940389" y="86.505867">Retriever 3</tspan></text> <path style="display:inline;fill:none;fill-rule:evenodd;stroke:#000000;stroke-width:0.228792px;stroke-linecap:butt;stroke-linejoin:miter;stroke-opacity:1" d="m 35.840252,54.194479 h 34.582654 c 2.645834,0 5.291667,2.645833 5.291667,5.291667 v 3.131907" id="path1" /> <path style="display:inline;fill:none;fill-rule:evenodd;stroke:#000000;stroke-width:0.079272px;stroke-linecap:butt;stroke-linejoin:miter;stroke-opacity:1" d="m 35.945109,69.325194 23.57879,-0.382955" id="path2" /> <path style="display:inline;fill:none;fill-rule:evenodd;stroke:#000000;stroke-width:0.264583px;stroke-linecap:butt;stroke-linejoin:miter;stroke-opacity:1" d="m 35.945109,85.436247 h 34.477797 c 2.645834,0 5.291667,-2.645833 5.291667,-5.291667 v -5.404079" id="path3" /> <path style="display:inline;fill:none;fill-rule:evenodd;stroke:#000000;stroke-width:0.264583px;stroke-linecap:butt;stroke-linejoin:miter;stroke-opacity:1" d="m 91.905247,68.699244 7.797062,0.0096" id="path4" /> </g> </svg>


## Architecture

This library sets up a web scraping pipeline and executes it as follows:
  - Starts the web retriever modules in its own separate thread that run parallely to get the content from the respective websites
  - Each page's content is populated into a document struct and transmitted by the web retriever module threads to the data processing chain.
  - Simultaneously the data processing modules are started (which form the data processing chain). The retrieved documents are passed to these threads in serial order, based on the priority configured for each data processing module.
  - Each data processing module processes the content and may add or modify the document it receives. It then passes it on to the next data processing thread in order of priority
  - Popular LLM services are supported by the data processing pipelines such as - **ChatGPT, Google Gemini** and self-hosted LLMs using **Ollama**. The relevant API keys need to be configured as environment variables before using these plugins. 
  - At then end, the document is written to disk as a json file
  - The retrieved URLs are saved to an SQLite database table to serve as a reference so these are not retrieved again in the next run.
  - Adequate wait times are configured during web retrieval to avoid overloading the target website. All events and actions are logged to a central log file. Multiple instances are prevented by writing and checking for a PID file. Although, if desired multiple instances can be launched by running the application with separate config files.

This package enables building a full-fledged multi-threaded web scraping solution that runs in batch mode with very meagre resources (e.g. single core CPU with less than 4GB RAM).

## Quick Start
Add this to your Cargo.toml:
[dependencies]
newslookout = "0.3.0"

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

## Create your own custom plugins and run these in the Pipeline

Declare custom retriever plugin and add these to the pipeline to fetch data using your customised logic.

```
fn run_pipeline(config: &config::Config) -> Vec<Document> {

    newslookout::init_logging(config);
    newslookout::init_pid_file(config);
    log::info!("Starting the custom pipeline");

    let mut retriever_plugins = newslookout::pipeline::load_retriever_plugins(config);
    let mut data_proc_plugins = newslookout::pipeline::load_dataproc_plugins(config);

    // add custom data retriever:
    retriever_plugins.push(my_plugin);
    let docs_retrieved = newslookout::pipeline::start_data_pipeline(
        retriever_plugins,
        data_proc_plugins,
        config
    );
    log::info!("Data pipeline completed processing {} documents.", docs_retrieved.len());
    // use docs_retrieved for any further custom processing.

    newslookout::cleanup_pid_file(&config);
}
```

Similarly, you can also declare and use custom data processing plugins, e.g.:
```
data_proc_plugins.push(my_own_data_processing);
```
Note that for data processing, these type of plugins are run in serial order of priority defined in the config file.

There are a few pre-built modules provided for a few websites.
These can be readily extended for other websites as required.

Refer to the source code of these in the plugins folder and roll out your own plugins.


## Configuration

The entire application is driven by its config file.
Refer to the example config file in the repository.
