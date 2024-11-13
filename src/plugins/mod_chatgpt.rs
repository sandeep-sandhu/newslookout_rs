use std::sync::mpsc::{Receiver, Sender};
use config::Config;
use log::{error, info};
use reqwest::blocking::Client;
use crate::document;
use crate::network::build_llm_api_client;
use crate::utils::get_plugin_config;

pub const PLUGIN_NAME: &str = "mod_chatgpt";
pub const PUBLISHER_NAME: &str = "LLM Processing via ChatGPT API Service";

pub(crate) fn process_data(tx: Sender<document::Document>, rx: Receiver<document::Document>, app_config: &Config) {
    info!("{}: Getting configuration.", PLUGIN_NAME);

    let mut model_name: String = String::from("llama3.1");
    match get_plugin_config(&app_config, crate::plugins::mod_ollama::PLUGIN_NAME, "model_name") {
        Some(param_val_str) => {
            model_name = param_val_str;
        },
        None => {}
    };

    let mut ollama_svc_base_url: String = String::from("http://127.0.0.1/");
    match get_plugin_config(&app_config, crate::plugins::mod_ollama::PLUGIN_NAME, "ollama_svc_base_url") {
        Some(param_val_str) => {
            ollama_svc_base_url = format!("{}api/generate", param_val_str);
        },
        None => {}
    };

    let mut temperature: f64 = 0.0;
    match get_plugin_config(&app_config, crate::plugins::mod_ollama::PLUGIN_NAME, "temperature") {
        Some(param_val_str) => {
            match param_val_str.trim().parse() {
                Result::Ok(param_float) => temperature = param_float,
                Err(e) => error!("When parsing parameter 'temperature' as float value: {}", e)
            }
        },
        None => error!("Could not get parameter 'temperature', using default value of: {}", temperature)
    };

    let mut overwrite: bool = false;
    match get_plugin_config(&app_config, PLUGIN_NAME, "overwrite"){
        Some(param_val_str) => {
            match param_val_str.trim().parse(){
                Result::Ok(param_bool) => overwrite = param_bool,
                Err(e) => error!("When parsing parameter 'overwrite' as boolean value: {}", e)
            }
        }, None => error!("Could not get parameter 'overwrite', using default as false")
    };

    let connect_timeout: u64 = 15;
    let fetch_timeout: u64 = 30;
    let chatgpt_http_client = build_llm_api_client(connect_timeout, fetch_timeout);

    for doc in rx {
        info!("Saving processed document titled - {}", doc.title);
        let updated_doc: document::Document = update_doc(&chatgpt_http_client, doc, model_name.as_str(), ollama_svc_base_url.as_str(), overwrite);
        //for each document received in channel queue
        match tx.send(updated_doc) {
            Result::Ok(_) => {},
            Err(e) => error!("{}: When sending processed doc via tx: {}", PLUGIN_NAME, e)
        }
    }

    info!("{}: Completed processing all data.", PLUGIN_NAME);
}

pub fn update_doc(ollama_client: &Client, mut raw_doc: document::Document, model_name: &str, ollama_svc_base_url: &str, overwrite: bool) -> document::Document {

    info!("{}: Starting to process parts of document - '{}'", PLUGIN_NAME, raw_doc.title);

    return raw_doc;
}
