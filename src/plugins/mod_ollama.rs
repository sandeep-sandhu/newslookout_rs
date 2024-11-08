// file: mod_ollama.rs

use std::collections::HashMap;
use std::error::Error;
use log::{error, info};
use serde::{Deserialize, Serialize};
use crate::{document, network};
use crate::document::Document;
use crate::utils::{clean_text, get_network_params, get_plugin_config, get_text_from_element, to_local_datetime};

pub(crate) const PLUGIN_NAME: &str = "mod_ollama";
const PUBLISHER_NAME: &str = "LLM Processing via Ollama Service";

pub(crate) fn process_data(doc: &document::Document, app_config: &config::Config){

    info!("{}: Getting configuration.", PLUGIN_NAME);
    let (fetch_timeout_seconds, retry_times, wait_time, user_agent) = get_network_params(&app_config);

    let mut model_name: String = String::from("llama3_1_8b");
    match get_plugin_config(&app_config, PLUGIN_NAME, "model_name"){
        Some(param_val_str) => {
            model_name =param_val_str;
        }, None => {}
    };

    let mut ollama_service_host_port: String = String::from("http://127.0.0.1/");
    match get_plugin_config(&app_config, PLUGIN_NAME, "host_port"){
        Some(param_val_str) => {
            ollama_service_host_port =param_val_str;
        }, None => {}
    };

    // TODO: for each text_part, prepare payload
    // call service with payload
    // store results of service into text_part

    info!("{}: Processed document: {} with model {} and service at {}", PLUGIN_NAME, doc.title, model_name, ollama_service_host_port);

}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct OllamaPayload {
    model: String,
    taskID: usize,
    keep_alive: String,
    options: HashMap<String, usize>, //"temperature": 0, "num_predict": 8192, "num_ctx": 8192,
    prompt: String
}

fn prepare_payload(prompt: String, model: String, num_context: usize, max_tok_gen: usize, temperature: usize) -> String {
    // put the parameters into the structure
    let json_payload = OllamaPayload {
        model: model,
        taskID: 42, // what else!
        keep_alive: String::from("10m"),
        options: HashMap::from([("temperature".to_string(), temperature), ("num_predict".to_string(), max_tok_gen), ("num_ctx".to_string(), num_context)]),
        prompt: prompt
    };
    // serialise to string
    match serde_json::to_string(&json_payload){
        Result::Ok(json_str) => return json_str,
        Err(e) => {
            log::error!("When serializing json payload: {}", e);
            return String::new();
        }
    }
}

fn send_payload_retrieve_result(json_payload: String, service_host_port: String) -> String {
    let response = String::new();
    // TODO : implement this

    return response;
}

fn prepare_prompt(input_text: String, system_context: String, user_context: String) -> String {
    // system context: You are an expert in reading financial news and generating their summaries for financial services professionals.
    // user context: Read the text below published by {news_publisher} titled '{doc_title}' and prepare a concise summary in less than 200 words. NO other text MUST be included.
    let mut prompt = format!("<|begin_of_text|><|start_header_id|>system<|end_header_id|>{}<|eot_id|><|start_header_id|>user<|end_header_id|>{}\n\n{}<|eot_id|> <|start_header_id|>assistant<|end_header_id|>", system_context, user_context, input_text);
    return prompt;
}

fn generate_summary(model_name: String, input_text: String, system_context: String, user_context: String, service_host_port: String) -> String{
    // prepare prompt
    let prompt = prepare_prompt(input_text, system_context, user_context);
    let json_string = prepare_payload(prompt, model_name, 8192, 8192, 0);
    if json_string.len() < 1{
        error!("Could not geneerate sumary");
        return json_string;
    }
    info!("Calling ollama service for summarisation.");
    let summary = send_payload_retrieve_result(json_string, service_host_port);
    return summary;
}

#[cfg(test)]
mod tests {
    use crate::plugins::mod_ollama;

    #[test]
    fn test_run_worker_thread() {
        // TODO: implement this
        assert_eq!(1, 1);
    }
}
