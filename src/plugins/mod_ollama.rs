// file: mod_ollama.rs

use std::collections::HashMap;
use std::error::Error;
use std::sync::mpsc::{Receiver, Sender};
use config::Config;
use log::{error, warn, info, debug};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use crate::{document, network};
use crate::network::{http_post_json_ollama, make_ollama_http_client};
use crate::utils::{clean_text, get_network_params, get_plugin_config, get_text_from_element, to_local_datetime};

pub(crate) const PLUGIN_NAME: &str = "mod_ollama";
const PUBLISHER_NAME: &str = "LLM Processing via Ollama Service";


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

    info!("{}: Getting configuration.", PLUGIN_NAME);

    let mut model_name: String = String::from("llama3_1_8b");
    match get_plugin_config(&app_config, PLUGIN_NAME, "model_name"){
        Some(param_val_str) => {
            model_name =param_val_str;
        }, None => {}
    };

    let mut ollama_svc_base_url: String = String::from("http://127.0.0.1/");
    match get_plugin_config(&app_config, PLUGIN_NAME, "ollama_svc_base_url"){
        Some(param_val_str) => {
            // prepare full url of the form - http://127.0.0.1/api/generate
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
        }, None => error!("Could not get parameter 'overwrite'")
    };

    let connect_timeout: u64 = 60;
    let fetch_timeout: u64 = 600;
    let ollama_client = make_ollama_http_client(connect_timeout, fetch_timeout);

    for doc in rx {
        info!("Saving processed document titled - {}", doc.title);
        let updated_doc:document::Document = update_doc(&ollama_client, doc, model_name.as_str(), ollama_svc_base_url.as_str(), overwrite);
        //for each document received in channel queue
        match tx.send(updated_doc) {
            Result::Ok(_) => {},
            Err(e) => error!("{}: When sending processed doc via tx: {}", PLUGIN_NAME, e)
        }
    }

    info!("{}: Completed processing all data.", PLUGIN_NAME);
}

fn update_doc(ollama_client: &Client, mut raw_doc: document::Document, model_name: &str, ollama_svc_base_url: &str, overwrite: bool) -> document::Document{

    let loopiters = raw_doc.text_parts.len() as i32;
    info!("{}: Starting to process {} parts of document - '{}'", PLUGIN_NAME, loopiters, raw_doc.title);

    let max_word_limit: usize = 200;
    let summary_system_context = "You are an expert in generating accurate summaries of financial news.";
    let summary_user_context = format!("Read the following text titled '{}' published by {} and summarize it in less than {} words.
        Return in bullet format with one sentence heading giving an overview of the topic in bullets.
        Walk through the text in manageable parts step by step, analyzing, grouping similar topics and summarizing as you go.
        You MUST use the same terms and abbreviations from the text while preparing the summary.
        NO other text MUST be included.\n", raw_doc.title, raw_doc.plugin_name, max_word_limit);

    let insights_system_context = "You are an expert in understanding and analysing financial services news and events.";
    let insights_user_context = format!("Based on the following text from a regulatory notification titled '{}', describe each action that a bank or lender or financial institution should take to manage risks, increase revenues, reduce costs or improve customer service.
            Walk me through this text in manageable parts step by step, analyzing and extracting and merging actions as we go.
            Answer back the action in full and complete sentences along with the original text from which it was extracted.
            Merge similar action items into one.
            Do not return actions that the regulator {} is planning to take.
            NO other text MUST be included.
            You MUST not return implied actions such as - REs shall take necessary steps to ensure compliance with these instructions.
            Return all actions extracted as a JSON object in this format:
            ```[{{\"original_text\": \"REs must do...\", \"insight\": \"Regulated Entities must do...\",}}]```
            TEXT:\n", raw_doc.title, raw_doc.plugin_name);

    // pop out each part, process it and push to new vector, replace this updated vector in document
    let mut updated_text_parts:  Vec<HashMap<String, String>> = Vec::new();
    let mut to_generate_summary: bool = true;
    let mut to_generate_insights: bool = true;

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
                    // call service with payload to generate summary:
                    let model_summary = generate_llm_response(ollama_client, model_name, text_part, summary_system_context, summary_user_context.as_str(), ollama_svc_base_url);
                    text_part_map_clone.insert("summary".to_string(), model_summary);
                }

                if let Some(existing_insights) = text_part_map.get("insights") {
                    if (overwrite == false) & (existing_insights.len()>3) {
                        info!("Not overwriting existing insights for part #{}", key);
                        to_generate_insights = false;
                    }
                }
                if to_generate_insights == true {
                    // call service with payload to generate insights:
                    let model_insights = generate_llm_response(ollama_client, model_name, text_part, insights_system_context, insights_user_context.as_str(), ollama_svc_base_url);
                    text_part_map_clone.insert("insights".to_string(), model_insights);
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

    info!("{}: Processed document titled: '{}' with model {}", PLUGIN_NAME, raw_doc.title, model_name);
    return raw_doc;
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct OllamaPayload {
    model: String,
    taskID: usize,
    keep_alive: String,
    options: HashMap<String, usize>, //"temperature": 0, "num_predict": 8192, "num_ctx": 8192,
    prompt: String,
    stream: bool,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct OllamaResponse{
    pub(crate) model: String,
    pub(crate) created_at: String,
    pub(crate) response: String,
    pub(crate) done: bool,
    pub(crate) context: Vec<usize>,
    pub(crate) total_duration: usize,
    pub(crate) load_duration: usize,
    pub(crate) prompt_eval_count: usize,
    pub(crate) prompt_eval_duration: usize,
    pub(crate) eval_count: usize,
    pub(crate) eval_duration: usize,
}

fn prepare_payload(prompt: String, model: &str, num_context: usize, max_tok_gen: usize, temperature: usize) -> OllamaPayload {
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


fn prepare_prompt(input_text: &str, system_context: &str, user_context: &str) -> String {
    let mut prompt = format!("<|begin_of_text|><|start_header_id|>system<|end_header_id|>{}<|eot_id|><|start_header_id|>user<|end_header_id|>{}\n\n{}<|eot_id|> <|start_header_id|>assistant<|end_header_id|>", system_context, user_context, input_text);
    return prompt;
}

fn generate_llm_response(ollama_client: &Client, model_name: &str, input_text: &str, system_context: &str, user_context: &str, service_host_port: &str) -> String {
    // prepare prompt
    let prompt = prepare_prompt(input_text, system_context, user_context);
    debug!("Calling ollama service with prompt: \n{}", prompt);
    let json_payload = prepare_payload(prompt, model_name, 8192, 8192, 0);
    debug!("{:?}", json_payload);
    let llm_output = http_post_json_ollama(service_host_port, &ollama_client, json_payload);
    debug!("Model response:\n{}", llm_output.response);
    return llm_output.response;
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
