// file: llm.rs

use std::collections::HashMap;
use std::error::Error;
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender};
use config::Config;
use log::{debug, error, info};
use regex::Regex;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use crate::{document, llm};
use crate::document::Document;
use crate::network::build_llm_api_client;
use crate::plugins::mod_summarize::PLUGIN_NAME;
use crate::utils::{get_contexts_from_config, get_data_folder, get_plugin_config, make_unique_filename, save_to_disk_as_json, split_by_word_count};

pub const MIN_ACCEPTABLE_SUMMARY_CHARS: usize = 25;
pub const MAX_TOKENS: f64 = 8000.0;
pub const TOKENS_PER_WORD: f64 = 1.33;

pub struct LLMParameters{
    pub llm_service: String,
    pub api_client: reqwest::blocking::Client,
    pub sumarize_fn: fn(&str, &LLMParameters)-> String,
    pub fetch_timeout: u64,
    pub overwrite_existing_value: bool,
    pub save_intermediate: bool,
    pub max_summary_wc: usize,
    pub model_temperature: f32,
    pub prompt: String,
    pub max_tok_gen: usize,
    pub model_name: String,
    pub num_context: usize,
    pub svc_base_url: String,
    pub system_context: String,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct GeminiRequestPayload {
    pub contents: Vec<HashMap<String, Vec<HashMap<String, String>>>>,
    pub safetySettings: Vec<HashMap<String, String>>,
    pub generationConfig: HashMap<String, usize>,
}

// Gemini API Response JSON format:
// {
//   "candidates": [
//     {
//       "content": {
//         "parts": [
//           {
//             "text": "Meow!  *Stretches and yawns, showing off a pink tongue* Morning! I'm feeling very fluffy and ready for some head scratches...and maybe a tasty treat.  What about you? \n"
//           }
//         ],
//         "role": "model"
//       },
//       "finishReason": "STOP",
//       "index": 0,
//       "safetyRatings": [
//         {
//           "category": "HARM_CATEGORY_SEXUALLY_EXPLICIT",
//           "probability": "LOW"
//         },
//         {
//           "category": "HARM_CATEGORY_HATE_SPEECH",
//           "probability": "NEGLIGIBLE"
//         },
//         {
//           "category": "HARM_CATEGORY_HARASSMENT",
//           "probability": "NEGLIGIBLE"
//         },
//         {
//           "category": "HARM_CATEGORY_DANGEROUS_CONTENT",
//           "probability": "NEGLIGIBLE"
//         }
//       ]
//     }
//   ],
//   "usageMetadata": {
//     "promptTokenCount": 16,
//     "candidatesTokenCount": 44,
//     "totalTokenCount": 60
//   },
//   "modelVersion": "gemini-1.5-flash-001"
// }


/// Prepare the JSON payload for sending to the Gemini LLM API service.
///
/// # Arguments
///
/// * `prompt`: The prompt to the model.
/// * `llm_params`: the LLMParameters struct with various params, e.g. temperature, num_ctx, max_gen
///
/// returns: RequestPayload
pub fn prepare_gemini_api_payload(prompt: String, llm_params: &LLMParameters) -> GeminiRequestPayload {
    // put the parameters into the structure
    let json_payload = GeminiRequestPayload {
        contents: vec![
            HashMap::from([
                ("parts".to_string(),
                 vec![HashMap::from([
                    ("text".to_string(), prompt)
                ])]
                )
            ])],
        safetySettings: vec![HashMap::from([
            ("category".to_string(), "HARM_CATEGORY_DANGEROUS_CONTENT".to_string()),
            ("threshold".to_string(), "BLOCK_ONLY_HIGH".to_string())
        ])],
        generationConfig: HashMap::from([
            ("temperature".to_string(), llm_params.model_temperature as usize),
            ("maxOutputTokens".to_string(), llm_params.max_tok_gen),
        ]),
    };
    return json_payload;
}


/// Posts the json payload to REST service and retrieves back the result.
///
/// # Arguments
///
/// * `service_url`:
/// * `client`:
/// * `json_payload`:
///
/// returns: String
pub fn http_post_json_gemini<'post>(service_url: &str, client: &reqwest::blocking::Client, json_payload: GeminiRequestPayload) -> String {
    // add json payload to body
    match client.post(service_url)
        .json(&json_payload)
        .send() {
        Result::Ok(resp) => {

            match resp.json::<serde_json::value::Value>(){
                Result::Ok( json ) => {
                    info!("Gemini model response:\n{:?}", json);
                    if let Some(resp_candidates) = json.get("candidates"){

                        if let Some(first_candidate) = resp_candidates.get(0) {

                            if let Some(resp_content) = first_candidate.get("content") {
                                if let Some(parts) = resp_content.get("parts") {

                                    if let Some(first_part) = parts.get(0) {

                                        if let Some(text_part) = first_part.get("text") {
                                            if let Some(response_str) = text_part.as_str() {
                                                return response_str.to_string();
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
                Err(e) => {
                    error!("When retrieving json from response: {}", e);
                    if let Some(err_source) = e.source(){
                        error!("Caused by: {}", err_source);
                    }
                },
            }
        }
        Err(e) => {
            error!("When posting json payload to service: {}", e);
            if let Some(err_source) = e.source(){
                error!("Caused by: {}", err_source);
            }
        }
    }
    return String::from("");
}

pub fn build_llm_prompt(model_name: &str, system_context: &str, user_context: &str, input_text: &str) -> String {
    if model_name.contains("llama") {
        return prepare_llama_prompt(system_context, user_context, input_text);
    } else if model_name.contains("gemma") {
        return prepare_gemma_prompt(system_context, user_context, input_text);
    }
    else {
        return format!("{}\n{}\n{}", system_context, user_context, input_text).to_string();
    }
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


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ChatGPTRequestPayload {
    pub model: String,
    pub messages: Vec<HashMap<String, String>>,
    pub temperature: f64,
    max_completion_tokens: usize,
    logprobs: bool,
}


pub fn prepare_chatgpt_headers(app_config: &Config) -> HeaderMap {
    let mut custom_headers = HeaderMap::new();
    //   -H "Authorization: Bearer $OPENAI_API_KEY" \
    let api_key = format!("Bearer {}", std::env::var("OPENAI_API_KEY").unwrap_or(String::from("")));
    if let Ok(header_val) = HeaderValue::from_str(api_key.as_str()) {
        custom_headers.insert(reqwest::header::AUTHORIZATION, header_val);
    }
    // // TODO: get from config file:  -H "OpenAI-Organization: YOUR_ORG_ID" \
    // let org_id = std::env::var("OPENAI_ORG").unwrap_or(String::from(""));
    // match HeaderValue::from_str(org_id.as_str()) {
    //     Ok(header_val) => {
    //         match HeaderName::from_lowercase(b"OpenAI-Organization") {
    //             Ok(org_name) => custom_headers.insert(org_name, header_val),
    //             Err(e) => error!("when setting header: {}", e)
    //         }
    //     },
    //     Err(e) => error!("when setting header: {}", e);
    // }
    // // TODO: get from config file:  -H "OpenAI-Project: $PROJECT_ID"
    // let project_id = std::env::var("PROJECT_ID").unwrap_or(String::from(""));
    // if let Ok(header_val) = HeaderValue::from_str(project_id.as_str()){
    //     let proj_id = HeaderName::from_lowercase(b"OpenAI-Project").unwrap();
    //     custom_headers.insert(proj_id, header_val);
    // }
    return custom_headers;
}

/// Generate payload of the format:
///     {
//           "model": "gpt-4o-mini",
//           "messages": [{"role": "user", "content": "Say this is a test!"}],
//           "temperature": 0.7
//         }
/// # Arguments
///
/// * `prompt`: The prompt to the model.
/// * `llm_params`: The LLMParameters object with relevant parameters to be used.
///
/// returns: ChatGPTRequestPayload
pub fn prepare_chatgpt_payload(prompt: String, llm_params: &LLMParameters) -> ChatGPTRequestPayload {

    // put the parameters into the structure
    let json_payload = ChatGPTRequestPayload {
        model: llm_params.model_name.clone(),
        messages: vec![
            HashMap::from([
                ("role".to_string(), "system".to_string()),
                ("content".to_string(), llm_params.system_context.clone())
            ]),
            HashMap::from([
                ("role".to_string(), "user".to_string()),
                ("content".to_string(), prompt)
            ]),
        ],
        temperature: llm_params.model_temperature as f64,
        max_completion_tokens: llm_params.max_tok_gen,
        logprobs: true,
    };
    return json_payload;
}


/// Posts the json payload to REST service and retrieves back the result.
///
/// # Arguments
///
/// * `service_url`:
/// * `client`:
/// * `json_payload`:
///
/// returns: String
pub fn http_post_json_chatgpt(llm_params: &LLMParameters, json_payload: ChatGPTRequestPayload) -> String{
    // add json payload to body
    match llm_params.api_client.post(llm_params.svc_base_url.clone())
        .json(&json_payload)
        .send() {
        Result::Ok(resp) => {
            match resp.json::<serde_json::value::Value>(){
                Result::Ok( json ) => {
                    info!("chatgpt model response:\n{:?}", json);
                    if let Some(choices) = json.get("choices"){
                        if let Some(first_choice) = choices.get(0) {
                            if let Some(message) = first_choice.get("message") {
                                if let Some(content) = message.get("content") {
                                    return content.to_string();
                                }
                            }
                        }
                    }
                },
                Err(e) => {
                    error!("When retrieving json from response: {}", e);
                    if let Some(err_source) = e.source(){
                        error!("Caused by: {}", err_source);
                    }
                },
            }
        }
        Err(e) => {
            error!("When posting json payload to service: {}", e);
            if let Some(err_source) = e.source(){
                error!("Caused by: {}", err_source);
            }
        }
    }
    return String::from("");
}


pub fn update_doc(http_api_client: &Client, mut input_doc: document::Document, model_name: &str, app_config: &Config, llm_fn: fn(&str, &Client, &str, &str, &Config) -> String) -> document::Document{
    const MIN_ACCEPTABLE_SUMMARY_LEN: usize = 20;
    const MIN_ACCEPTABLE_INSIGHTS_LEN: usize = 3;

    let loopiters = input_doc.text_parts.len() as i32;
    info!("Starting to process {} parts of document - '{}'", loopiters, input_doc.title);

    let mut svc_url: String = String::from("http://127.0.0.1/api/generate");
    let mut overwrite: bool = false;
    let mut save_intermediate: bool = true;

    let binding = get_data_folder(&app_config);
    let data_folder_name = binding.to_str().unwrap_or_default();
    let mut temperature: f64 = 0.0;

    // get contexts from config file:
    let (summary_part_context, insights_part_context, summary_exec_context, system_context) = get_contexts_from_config(&app_config);

    // make full path by joining folder to unique filename
    let json_file_path = Path::new(data_folder_name).join(make_unique_filename(&input_doc, "json"));
    input_doc.filename = String::from(json_file_path.as_path().to_str().expect("Not able to convert path to string"));

    // pop out each part, process it and push to new vector, replace this updated vector in document
    let mut updated_text_parts:  Vec<HashMap<String, Value>> = Vec::new();
    let mut to_generate_summary: bool = true;
    let mut to_generate_insights: bool = true;
    let mut all_summaries: String = String::new();
    let mut all_actions: String = String::new();

    for i in 0..loopiters {
        match &input_doc.text_parts.pop(){
            None => {break;}
            Some(text_part_map) => {
                // store results of llm into a copy of this text_part
                let mut text_part_map_clone = text_part_map.clone();
                to_generate_summary = true;
                to_generate_insights = true;
                let key = text_part_map.get("id").expect("Each text part in the document should contain key 'id'");
                let text_part = text_part_map.get("text").expect("Each text part in the document should contain key 'text'");
                info!("Processing text part #{}", key);

                // check if there is a key "summary", if so:
                if let Some(existing_summary) = text_part_map.get("summary") {
                    if (overwrite == false) & (existing_summary.to_string().len() > MIN_ACCEPTABLE_SUMMARY_LEN) {
                        info!("Not overwriting existing summary for part #{}", key);
                        to_generate_summary = false;
                    }
                }
                if to_generate_summary == true{
                    let summary_part_prompt = build_llm_prompt(model_name, system_context.as_str(), summary_part_context.as_str(), text_part.to_string().as_str());
                    // call service with payload to generate summary of part:
                    let summary_part = llm_fn(svc_url.as_str(), http_api_client, model_name, summary_part_prompt.as_str(), app_config);
                    all_summaries.push_str("\n");
                    all_summaries.push_str(summary_part.as_str());
                    text_part_map_clone.insert("summary".to_string(), Value::String(summary_part));
                }

                if let Some(existing_insights) = text_part_map.get("insights") {
                    if (overwrite == false) & (existing_insights.to_string().len() > MIN_ACCEPTABLE_INSIGHTS_LEN) {
                        info!("Not overwriting existing insights for part #{}", key);
                        to_generate_insights = false;
                    }
                }
                if to_generate_insights == true {
                    // call service with payload to generate insights:
                    let insights_part_prompt = build_llm_prompt(model_name, system_context.as_str(), insights_part_context.as_str(), text_part.to_string().as_str());
                    // call service with payload to generate insights of part:s
                    let insights_part = llm_fn(svc_url.as_str(), http_api_client, model_name, insights_part_prompt.as_str(), app_config);
                    all_actions.push_str(insights_part.as_str());
                    _ = text_part_map_clone.insert("insights".to_string(), Value::String(insights_part));
                }

                // put the updated text part into a new vector
                updated_text_parts.push(text_part_map_clone);
                // save to file raw_doc.filename
                if save_intermediate == true{
                    save_to_disk_as_json(&input_doc, json_file_path.to_str().unwrap_or_default());
                }

            }
        }
    }
    // reverse the updated text parts vector:
    updated_text_parts.reverse();
    // store it in the document, replacing the previous contents
    input_doc.text_parts = updated_text_parts;

    // generate the exec summary:
    let exec_summary_prompt= build_llm_prompt(model_name, system_context.as_str(), summary_exec_context.as_str(), all_summaries.as_str());
    // call service with payload to generate summary:
    let exec_summary= llm_fn(svc_url.as_str(), http_api_client, model_name, exec_summary_prompt.as_str(), app_config);
    // add to generated_content
    input_doc.generated_content.insert("exec_summary".to_string(), exec_summary);

    // generate the actions summary:
    let actions_summary_prompt= build_llm_prompt(model_name, system_context.as_str(), summary_exec_context.as_str(), all_actions.as_str());
    // call service with payload to generate actions summary:
    let actions_summary= llm_fn(svc_url.as_str(), http_api_client, model_name, actions_summary_prompt.as_str(), app_config);
    input_doc.generated_content.insert("actions_summary".to_string(), actions_summary);

    save_to_disk_as_json(&input_doc, json_file_path.to_str().unwrap_or_default());

    info!("Model {} completed processing document titled: '{}' ", model_name, input_doc.title);
    return input_doc;
}


pub fn generate_using_ollama_api(ollama_svc_base_url: &str, ollama_client: &reqwest::blocking::Client, model_name: &str, summary_part_prompt: &str, app_config: &Config) -> String {
    debug!("Calling ollama service with prompt: \n{}", summary_part_prompt);

    let json_payload = prepare_ollama_payload(summary_part_prompt, model_name, 8192, 8192, 0);
    debug!("{:?}", json_payload);

    let llm_output = http_post_json_ollama(ollama_svc_base_url, &ollama_client, json_payload);
    debug!("Model response:\n{}", llm_output);
    return llm_output;
}

pub fn prepare_llm_parameters(app_config: &config::Config, task_prompt: String) -> LLMParameters {

    // get llm sevice name:
    let mut llm_svc_name = String::from("ollama");
    match get_plugin_config(&app_config, PLUGIN_NAME, "llm_service"){
        Some(param_val_str) => llm_svc_name = param_val_str,
        None => error!("Error getting LLM service from config, using default value: {}",
            llm_svc_name)
    }

    let mut model_name = String::from("gemma2_27b");

    let mut system_context: String = String::from("");
    match app_config.get_string("system_context") {
        Ok(param_val_str) => system_context = param_val_str,
        Err(e) => error!("Could not load parameter 'system_context' from config: {}", e)
    }

    // get overwrite config parameter
    let mut overwrite: bool = false;
    match get_plugin_config(&app_config, PLUGIN_NAME, "overwrite"){
        Some(param_val_str) => {
            match param_val_str.trim().parse(){
                Ok(param_val) => overwrite = param_val,
                Err(e) => error!("When parsing parameter 'fetch_timeout' as integer value: {}", e)
            }
        }, None => error!("Could not get parameter 'overwrite', using default value of: {}", overwrite)
    };

    let mut max_word_count: usize = 850;
    match get_plugin_config(&app_config, PLUGIN_NAME, "max_word_count"){
        Some(param_val_str) => {
            match param_val_str.trim().parse(){
                Ok(param_val) => max_word_count = param_val,
                Err(e) => error!("When parsing parameter 'max_word_count': {}", e)
            }
        }, None => error!("Could not get parameter 'max_word_count', using default: {}", overwrite)
    };

    // get fetch timeout config parameter
    let mut fetch_timeout: u64 = 3700;
    match app_config.get_int("model_api_timeout"){
        Ok(param_int) => {
            fetch_timeout = param_int as u64;
        }, Err(e) => error!("Could not get parameter 'fetch_timeout': {}", e)
    };

    let mut max_llm_context_tokens: u64 = 8192;
    match app_config.get_int("max_llm_context_tokens"){
        Ok(param_int) => {
            max_llm_context_tokens = param_int as u64;
        }, Err(e) => error!("Could not get parameter 'max_llm_context_tokens': {}", e)
    };

    let mut max_gen_tokens: u64 = 8192;
    match app_config.get_int("max_gen_tokens"){
        Ok(param_int) => {
            max_gen_tokens = param_int as u64;
        }, Err(e) => error!("Could not get parameter 'max_gen_tokens': {}", e)
    };

    // set a low connect timeout:
    let connect_timeout: u64 = 15;
    let mut http_api_client = build_llm_api_client(connect_timeout, fetch_timeout, None, None);

    let mut summarize_function: fn(&str, &LLMParameters)-> String =
        llm::generate_using_ollama;

    let mut svc_base_url = String::from("http://127.0.0.1:11434/api/generate");

    match llm_svc_name.as_str() {
        "chatgpt" => {
            summarize_function = llm::generate_using_chatgpt;
            // prepare the http client for the REST service
            let mut custom_headers = prepare_chatgpt_headers(app_config);
            http_api_client = build_llm_api_client(
                connect_timeout,
                fetch_timeout,
                None,
                Some(custom_headers)
            );
            match app_config.get_string("chatgpt_svc_url") {
                Ok(param_val_str) => svc_base_url = param_val_str,
                Err(e) => error!("Could not load 'chatgpt_svc_url' from config: {}", e)
            }
            match app_config.get_string("chatgpt_model_name") {
                Ok(param_val_str) => model_name = param_val_str,
                Err(e) => error!("Could not load 'chatgpt_model_name' from config: {}", e)
            }
        },
        "gemini" => {
            summarize_function = llm::generate_using_gemini;
            match app_config.get_string("gemini_svc_url") {
                Ok(param_val_str) => svc_base_url = param_val_str,
                Err(e) => error!("Could not load 'gemini_svc_url' from config: {}", e)
            }
            match app_config.get_string("gemini_model_name") {
                Ok(param_val_str) => model_name = param_val_str,
                Err(e) => error!("Could not load 'gemini_model_name' from config: {}", e)
            }
        },
        "ollama" => {
            summarize_function = llm::generate_using_ollama;
            match app_config.get_string("ollama_svc_url") {
                Ok(param_val_str) => svc_base_url = param_val_str,
                Err(e) => error!("Could not load 'ollama_svc_url' from config: {}", e)
            }
            match app_config.get_string("ollama_model_name") {
                Ok(param_val_str) => model_name = param_val_str,
                Err(e) => error!("Could not load 'ollama_model_name' from config: {}", e)
            }
        },
        _ => error!("Unknown llm service specified in config: {}", llm_svc_name)
    }

    let llm_params = LLMParameters{
        llm_service: llm_svc_name,
        api_client: http_api_client,
        sumarize_fn: summarize_function,
        fetch_timeout,
        overwrite_existing_value: overwrite,
        save_intermediate: true,
        max_summary_wc: max_word_count,
        model_temperature: 0.0,
        prompt: task_prompt,
        max_tok_gen: max_gen_tokens as usize,
        model_name: model_name,
        num_context: max_llm_context_tokens as usize,
        svc_base_url: svc_base_url,
        system_context: system_context,
    };

    return llm_params;
}

pub fn generate_using_ollama(input_text: &str, llm_params: &LLMParameters) -> String {

    let prompt = build_llm_prompt(
        llm_params.model_name.as_str(),
        llm_params.system_context.as_str(),
        llm_params.prompt.as_str(),
        input_text,
    );
    debug!("Ollama Prepared prompt: {}", prompt);

    // prepare payload
    let payload = prepare_ollama_payload(
        prompt.as_str(),
        llm_params.model_name.as_str(),
        llm_params.num_context,
        llm_params.max_tok_gen,
        llm_params.model_temperature as usize );

    debug!("Payload:\n{:?}", payload);

    let llm_output = http_post_json_ollama(
        llm_params.svc_base_url.as_str(),
        &llm_params.api_client,
        payload
    );
    info!("Ollama Generated content: {}", llm_output);
    
    return llm_output;
}

/// Posts the prompt to generate text using the Gemini LLM API.
/// Converts the url, model and api key to full url for the api service for non-stream
/// content generation:
/// e.g. https://generativelanguage.googleapis.com/v1beta/models/gemini-1.5-flash-latest:generateContent?key=$GOOGLE_API_KEY
/// First, the payload is prepared in json format.
/// Then, it is HTTP POST(ed) to the URL and the response payload is retrieved and converted
/// from json to struct to extract and return the model generated output text.
///
/// # Arguments
///
/// * `input_text`: The prompt + context input to the model service
/// * `llm_params`: The API parameters to be used, e.g. temperature, max token count, model, etc.
///
/// returns: String
pub fn generate_using_gemini(input_text: &str, llm_params: &LLMParameters) -> String {

    // get key from env variable: GOOGLE_API_KEY
    let api_key = std::env::var("GOOGLE_API_KEY").unwrap_or(String::from(""));
    // prepare url for the api
    let api_url = format!("{}{}:generateContent?key={}", llm_params.svc_base_url, llm_params.model_name, api_key);

    let prompt = format!("{}\n{}", llm_params.prompt, input_text);

    let json_payload = prepare_gemini_api_payload(prompt, llm_params);

    let llm_output = http_post_json_gemini(api_url.as_str(), &llm_params.api_client, json_payload);
    info!("Gemini Generated content: {}", llm_output);

    return llm_output;
}

pub fn generate_using_chatgpt(input_text: &str, llm_params: &LLMParameters) -> String {

    let prompt = format!("{}\n{}", llm_params.prompt, input_text);

    let json_payload = prepare_chatgpt_payload(prompt, llm_params);

    let llm_output= http_post_json_chatgpt(llm_params, json_payload);
    info!("ChatGPT Generated content: {}", llm_output);

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

pub fn prepare_ollama_payload(prompt: &str, model: &str, num_context: usize, max_tok_gen: usize, temperature: usize) -> OllamaPayload {
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
pub fn http_post_json_ollama(service_url: &str, client: &reqwest::blocking::Client, json_payload: OllamaPayload) -> String{
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
    use config::Config;
    use log::debug;
    use crate::llm;
    use crate::llm::{GeminiRequestPayload, prepare_chatgpt_headers, prepare_gemini_api_payload};
    use crate::network::build_llm_api_client;

    #[test]
    fn test_generate_using_llm(){
        let empty_config = Config::builder().build().unwrap();
        let api_client = build_llm_api_client(
            15,
            300,
            None,
            Some(prepare_chatgpt_headers(&empty_config))
        );
        // let resp = mod_chatgpt::generate_using_llm(
        //     "https://api.openai.com/v1/chat/completions",
        //     &api_client,
        //     "gpt-4o-mini",
        //     "Why is the sky blue? Reply very concisely.",
        //     &empty_config
        // );
        // debug!("Response from model = {:?}", resp);
        assert_eq!(1,1);
    }

    #[test]
    fn test_generate_using_gemini_llm(){
        let api_client = build_llm_api_client(100, 3000, None, None);
        // let resp = mod_gemini::generate_using_llm(
        //     "https://generativelanguage.googleapis.com/v1beta/models/",
        //     &api_client,
        //     "gemini-1.5-flash",
        //     "Why is the sky blue?",
        //     &Config::builder().build().unwrap()
        // );
        // debug!("Response from model = {:?}", resp);
        assert_eq!(1,1);
    }

    // #[test]
    // fn test_prepare_gemini_api_payload(){
    //     let json_struct = prepare_gemini_api_payload("Why is the sky blue?", 8192, 8192, 0);
    //     if let Ok(json_text) = serde_json::to_string(&json_struct){
    //         debug!("{}", json_text);
    //         let deserialized_json:GeminiRequestPayload = serde_json::from_str(json_text.as_str()).unwrap();
    //         assert_eq!(deserialized_json.contents.len(), 1);
    //     }
    // }
}
