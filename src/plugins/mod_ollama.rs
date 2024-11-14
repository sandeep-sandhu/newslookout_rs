// file: mod_ollama.rs

use std::collections::HashMap;
use std::error::Error;
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;
use config::Config;
use log::{error, warn, info, debug};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use crate::{document, network};
use crate::llm::update_doc;
use crate::network::{build_llm_api_client};
use crate::utils::{clean_text, get_contexts_from_config, get_network_params, get_plugin_config, get_text_from_element, to_local_datetime, save_to_disk_as_json, get_data_folder, make_unique_filename, build_llm_prompt};

pub const PLUGIN_NAME: &str = "mod_ollama";
pub const PUBLISHER_NAME: &str = "LLM Text processing using Ollama";


/// Executes this function of the module in the separate thread launched by the pipeline to
/// process documents received on channel rx and,
/// transmit the updated documents to tx.
///
/// # Arguments
///
/// * `tx`: Queue transmitter for the next thread
/// * `rx`: Queue receiver for this thread
/// * `config`: The application's configuration object
///
/// returns: ()
///
pub(crate) fn process_data(tx: Sender<document::Document>, rx: Receiver<document::Document>, app_config: &Config){

    info!("{}: Getting configuration specific to the module.", PLUGIN_NAME);

    // get fetch timeout config parameter
    let mut fetch_timeout: u64 = 150;
    match get_plugin_config(&app_config, PLUGIN_NAME, "fetch_timeout"){
        Some(param_val_str) => {
            match param_val_str.trim().parse(){
                Result::Ok(param_int) => fetch_timeout = param_int,
                Err(e) => error!("When parsing parameter 'fetch_timeout' as integer value: {}", e)
            }
        }, None => error!("Could not get parameter 'fetch_timeout', using default value of: {}", fetch_timeout)
    };
    // set a low connect timeout:
    let connect_timeout: u64 = 15;
    // prepare the http client for the REST service
    let ollama_client = build_llm_api_client(connect_timeout, fetch_timeout, None, None);

    // process each document received and return back to next handler:
    for doc in rx {

        info!("{}: Started processing document titled - '{}',  with #{} parts.",
            PLUGIN_NAME, doc.title, doc.text_parts.len()
        );
        let updated_doc:document::Document = update_doc(
            &ollama_client,
            doc,
            PLUGIN_NAME,
            &app_config,
            generate_using_llm
        );

        //for each document received in channel queue, transmit it to next queue
        match tx.send(updated_doc) {
            Result::Ok(_) => {},
            Err(e) => error!("{}: When sending processed doc via tx: {}", PLUGIN_NAME, e)
        }
    }

    info!("{}: Completed processing all data.", PLUGIN_NAME);
}


pub fn generate_using_llm(ollama_svc_base_url: &str, ollama_client: &reqwest::blocking::Client, model_name: &str, summary_part_prompt: &str, app_config: &Config) -> String {
    debug!("Calling ollama service with prompt: \n{}", summary_part_prompt);

    let json_payload = prepare_payload(summary_part_prompt, model_name, 8192, 8192, 0);
    debug!("{:?}", json_payload);

    let llm_output = http_post_json_ollama(ollama_svc_base_url, &ollama_client, json_payload);
    debug!("Model response:\n{}", llm_output);
    return llm_output;
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct OllamaPayload {
    pub model: String,
    pub taskID: usize,
    pub keep_alive: String,
    pub options: HashMap<String, usize>, //"temperature": 0, "num_predict": 8192, "num_ctx": 8192,
    pub prompt: String,
    pub stream: bool,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct OllamaResponse{
    pub model: String,
    pub created_at: String,
    pub response: String,
    pub done: bool,
    pub context: Vec<usize>,
    pub total_duration: usize,
    pub load_duration: usize,
    pub prompt_eval_count: usize,
    pub prompt_eval_duration: usize,
    pub eval_count: usize,
    pub eval_duration: usize,
}

pub fn prepare_payload(prompt: &str, model: &str, num_context: usize, max_tok_gen: usize, temperature: usize) -> OllamaPayload {
    // put the parameters into the structure
    let json_payload = OllamaPayload {
        model: model.to_string(),
        taskID: 42, // what else!
        keep_alive: String::from("10m"),
        options: HashMap::from([("temperature".to_string(), temperature), ("num_predict".to_string(), max_tok_gen), ("num_ctx".to_string(), num_context)]),
        prompt: prompt.to_string(),
        stream: false,
    };
    return json_payload;
}


/// Posts the json payload to Ollama REST service and retrieves back the result.
///
/// # Arguments
///
/// * `service_url`:
/// * `client`:
/// * `json_payload`:
///
/// returns: String
///
/// # Examples
///
/// ```
///
/// ```
pub fn http_post_json_ollama(service_url: &str, client: &reqwest::blocking::Client, json_payload: crate::plugins::mod_ollama::OllamaPayload) -> String{
    // add json payload to body
    match client.post(service_url)
        .json(&json_payload)
        .send() {
        Result::Ok(resp) => {
            match resp.json::<OllamaResponse>(){
                Result::Ok( json ) => {
                    return json.response;
                },
                Err(e) => {
                    error!("When retrieving json from response: {}", e);
                    if let Some(err_source) = e.source(){
                        error!("Caused by: {}", err_source);
                    }
                }
            }
        },
        Err(e) => {
            error!("When posting json payload to service: {}", e);
            if let Some(err_source) = e.source(){
                error!("Caused by: {}", err_source);
            }
        }
    }
    return String::from("");
}


#[cfg(test)]
mod tests {
    use crate::plugins::mod_ollama;

    #[test]
    fn test_run_worker_thread() {
        // TODO: implement this
        assert_eq!(1, 1);
    }

    #[test]
    fn test_prepare_payload(){
        // TODO: implement this
        assert_eq!(1, 1);
    }

}
