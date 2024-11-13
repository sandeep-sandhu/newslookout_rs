// file: mod_ollama.rs

use std::collections::HashMap;
use std::error::Error;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;
use config::Config;
use log::{error, warn, info, debug};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use crate::{document, network};
use crate::network::{build_llm_api_client};
use crate::utils::{clean_text, get_contexts_from_config, get_network_params, get_plugin_config, get_text_from_element, to_local_datetime};

pub const PLUGIN_NAME: &str = "mod_ollama";
pub const PUBLISHER_NAME: &str = "LLM Processing via Ollama Service";


/// Process documents received on channel rx and,
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

    let mut model_name: String = String::from("llama3.1");
    match get_plugin_config(&app_config, PLUGIN_NAME, "model_name"){
        Some(param_val_str) => {
            model_name =param_val_str;
        }, None => {}
    };

    let mut ollama_svc_base_url: String = String::from("http://127.0.0.1/");
    match get_plugin_config(&app_config, PLUGIN_NAME, "ollama_svc_base_url"){
        Some(param_val_str) => {
            ollama_svc_base_url = format!("{}api/generate", param_val_str);
        }, None => {}
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

    let mut temperature: f64 = 0.0;
    match get_plugin_config(&app_config, PLUGIN_NAME, "temperature"){
        Some(param_val_str) => {
            match param_val_str.trim().parse(){
                Result::Ok(param_float) => temperature = param_float,
                Err(e) => error!("When parsing parameter 'temperature' as float value: {}", e)
            }
        }, None => error!("Could not get parameter 'temperature', using default value of: {}", temperature)
    };

    let mut fetch_timeout: u64 = 150;
    match get_plugin_config(&app_config, PLUGIN_NAME, "fetch_timeout"){
        Some(param_val_str) => {
            match param_val_str.trim().parse(){
                Result::Ok(param_int) => fetch_timeout = param_int,
                Err(e) => error!("When parsing parameter 'fetch_timeout' as integer value: {}", e)
            }
        }, None => error!("Could not get parameter 'fetch_timeout', using default value of: {}", fetch_timeout)
    };

    // get contexts from config file:
    let (summary_part_context, insights_part_context, summary_exec_context, system_context) = get_contexts_from_config(&app_config);

    // set a low connect timeout:
    let connect_timeout: u64 = 15;
    // prepare the http client for the REST service
    let ollama_client = build_llm_api_client(connect_timeout, fetch_timeout);

    // process each document received and return back to next handler:
    for doc in rx {
        info!("Saving processed document titled - {}", doc.title);

        let updated_doc:document::Document = update_doc(
            &ollama_client,
            doc,
            model_name.as_str(),
            ollama_svc_base_url.as_str(),
            overwrite,
            summary_part_context.as_str(),
            insights_part_context.as_str(),
            summary_exec_context.as_str(),
            system_context.as_str()
        );
        //for each document received in channel queue
        match tx.send(updated_doc) {
            Result::Ok(_) => {},
            Err(e) => error!("{}: When sending processed doc via tx: {}", PLUGIN_NAME, e)
        }
    }

    info!("{}: Completed processing all data.", PLUGIN_NAME);
}

pub fn prepare_gemma_prompt(system_context: &str, user_context: &str, input_text: &str) -> String{
    return format!("<start_of_turn>user\
        {}\
        \
        {}<end_of_turn><start_of_turn>model", user_context, input_text).to_string();
}

pub fn prepare_llama_prompt(system_context: &str, user_context: &str, input_text: &str) -> String {
    return format!("<|begin_of_text|><|start_header_id|>system<|end_header_id|>{}\
        <|eot_id|><|start_header_id|>user<|end_header_id|>{}\
        \n\n{}<|eot_id|> <|start_header_id|>assistant<|end_header_id|>", system_context, user_context, input_text).to_string();
}


pub fn update_doc(ollama_client: &Client, mut raw_doc: document::Document, model_name: &str, ollama_svc_base_url: &str, overwrite: bool, summary_part_context: &str, insights_part_context: &str, summary_exec_context: &str, system_context: &str) -> document::Document{

    let loopiters = raw_doc.text_parts.len() as i32;
    info!("{}: Starting to process {} parts of document - '{}'", PLUGIN_NAME, loopiters, raw_doc.title);

    // pop out each part, process it and push to new vector, replace this updated vector in document
    let mut updated_text_parts:  Vec<HashMap<String, String>> = Vec::new();
    let mut to_generate_summary: bool = true;
    let mut to_generate_insights: bool = true;
    let mut all_summaries: String = String::new();
    let mut all_actions: String = String::new();

    for i in 0..loopiters {
        match &raw_doc.text_parts.pop(){
            None => {break;}
            Some(text_part_map) => {
                // store results of llm into a copy of this text_part
                let mut text_part_map_clone = text_part_map.clone();
                to_generate_summary = true;
                to_generate_insights = true;
                let key = text_part_map.get("id").expect("Each text part in the document should contain key 'id'");
                let text_part = text_part_map.get("text").expect("Each text part in the document should contain key 'text'");
                info!("{}: Processing text part #{}", PLUGIN_NAME, key);

                // check if there is a key "summary", if so:
                if let Some(existing_summary) = text_part_map.get("summary") {
                    if (overwrite == false) & (existing_summary.len()>3) {
                        info!("Not overwriting existing summary for part #{}", key);
                        to_generate_summary = false;
                    }
                }
                if to_generate_summary == true{
                    let summary_part_prompt = build_llm_prompt(model_name, system_context, summary_part_context, text_part);
                    // call service with payload to generate summary of part:
                    debug!("Calling ollama service with prompt: \n{}", summary_part_prompt);
                    let json_payload = prepare_payload(summary_part_prompt, model_name, 8192, 8192, 0);
                    debug!("{:?}", json_payload);
                    let llm_output = http_post_json_ollama(ollama_svc_base_url, &ollama_client, json_payload);
                    debug!("Model response:\n{}", llm_output);
                    all_summaries.push_str("\n");
                    all_summaries.push_str(llm_output.as_str());
                    text_part_map_clone.insert("summary".to_string(), llm_output);
                }

                if let Some(existing_insights) = text_part_map.get("insights") {
                    if (overwrite == false) & (existing_insights.len()>3) {
                        info!("Not overwriting existing insights for part #{}", key);
                        to_generate_insights = false;
                    }
                }
                if to_generate_insights == true {
                    // call service with payload to generate insights:
                    let insights_part_prompt = build_llm_prompt(model_name, system_context, insights_part_context, text_part);
                    // call service with payload to generate summary of part:
                    debug!("Calling ollama service with prompt: \n{}", insights_part_prompt);
                    let json_payload = prepare_payload(insights_part_prompt, model_name, 8192, 8192, 0);
                    debug!("{:?}", json_payload);
                    let llm_output = http_post_json_ollama(ollama_svc_base_url, &ollama_client, json_payload);
                    debug!("Model response:\n{}", llm_output);
                    all_actions.push_str(llm_output.as_str());
                    text_part_map_clone.insert("insights".to_string(), llm_output);
                }

                // put the updated text part into a new vector
                updated_text_parts.push(text_part_map_clone);
            }
        }
    }
    // reverse the updated text parts vector:
    updated_text_parts.reverse();
    // store it in the document, replacing the previous contents
    raw_doc.text_parts = updated_text_parts;

    // generate the exec summary:
    let exec_summary_prompt= build_llm_prompt(model_name, system_context, summary_exec_context, all_summaries.as_str());
    // call service with payload to generate summary of part:
    debug!("Calling ollama service with prompt: \n{}", exec_summary_prompt);
    let json_payload = prepare_payload(exec_summary_prompt, model_name, 8192, 8192, 0);
    debug!("{:?}", json_payload);
    let exec_summary = http_post_json_ollama(ollama_svc_base_url, &ollama_client, json_payload);
    debug!("Model response:\n{}", exec_summary);
    // add to generated_content
    raw_doc.generated_content.insert("exec_summary".to_string(), exec_summary);

    // generate the actions summary:
    let actions_summary_prompt= build_llm_prompt(model_name, system_context, summary_exec_context, all_actions.as_str());
    // call service with payload to generate summary of part:
    debug!("Calling ollama service with prompt: \n{}", actions_summary_prompt);
    let json_payload = prepare_payload(actions_summary_prompt, model_name, 8192, 8192, 0);
    debug!("{:?}", json_payload);
    let actions_summary = http_post_json_ollama(ollama_svc_base_url, &ollama_client, json_payload);
    debug!("Model response:\n{}", actions_summary);
    // add to generated_content
    raw_doc.generated_content.insert("actions_summary".to_string(), actions_summary);

    info!("{}: Processed document titled: '{}' with model {}", PLUGIN_NAME, raw_doc.title, model_name);
    return raw_doc;
}

fn build_llm_prompt(model_name: &str, system_context: &str, user_context: &str, input_text: &str) -> String {
    if model_name.contains("llama") {
        return prepare_llama_prompt(system_context, user_context, input_text);
    } else if model_name.contains("gemma") {
        return prepare_gemma_prompt(system_context, user_context, input_text);
    }
    else {
        return format!("{}\n{}\n{}", system_context, user_context, input_text).to_string();
    }
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

pub fn prepare_payload(prompt: String, model: &str, num_context: usize, max_tok_gen: usize, temperature: usize) -> OllamaPayload {
    // put the parameters into the structure
    let json_payload = OllamaPayload {
        model: model.to_string(),
        taskID: 42, // what else!
        keep_alive: String::from("10m"),
        options: HashMap::from([("temperature".to_string(), temperature), ("num_predict".to_string(), max_tok_gen), ("num_ctx".to_string(), num_context)]),
        prompt: prompt,
        stream: false,
    };
    return json_payload;
}


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

    #[test]
    fn test_build_llm_prompt(){
        // TODO: implement this
        assert_eq!(1, 1);
    }
}
