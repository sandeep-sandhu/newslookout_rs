// file: llm.rs

use crate::{get_cfg, get_cfg_bool, get_cfg_int, get_plugin_cfg};
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
use crate::{cfg, document, llm};
use crate::document::Document;
use crate::network::build_llm_api_client;
use crate::plugins::mod_summarize::PLUGIN_NAME;
use crate::cfg::{get_plugin_config, get_data_folder};
use crate::utils::{make_unique_filename, save_to_disk_as_json, split_by_word_count};

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
        Ok(resp) => {
            match resp.json::<serde_json::value::Value>(){
                Ok( json ) => {
                    info!("chatgpt model response:\n{:?}", json);
                    if let Some(choices) = json.get("choices"){
                        if let Some(first_choice) = choices.get(0) {
                            if let Some(message) = first_choice.get("message") {
                                if let Some(content) = message.get("content") {
                                    return content.to_string();
                                }
                            }
                            // get object: "logprobs" , get attrib: "content" array of Object -> {"bytes", "logprob"}
                        }
                    }
                    // get and print string:  "model"
                    // get and print object/dict: "usage" -> get integer attributes: "prompt_tokens", "total_tokens"
                },
                Err(e) => {
                    error!("When retrieving json from response: {}", e);
                    if let Some(err_source) = e.source(){
                        error!("Caused by: {}", err_source);
                    }
                    info!("ChatGPT Payload: {:?}", json_payload);
                },
            }
        }
        Err(e) => {
            error!("When posting json payload to service: {}", e);
            if let Some(err_source) = e.source(){
                error!("Caused by: {}", err_source);
            }
            info!("ChatGPT Payload: {:?}", json_payload);
        }
    }
    return String::from("");
}

/// Generate text using Ollama API service
///
/// # Arguments
///
/// * `ollama_svc_base_url`: The base url for the API, e.g. http://127.0.0.1:11434/api/generate
/// * `ollama_client`: The reqwest client to be used for HTTP POSTing the JSON payload to the service
/// * `model_name`: The name of the model registered with ollama
/// * `summary_part_prompt`: The complete prompt + context for the LLM
/// * `app_config`: The Application config object
///
/// returns: String
pub fn generate_using_ollama_api(ollama_svc_base_url: &str, ollama_client: &reqwest::blocking::Client, model_name: &str, prompt_and_context: &str, app_config: &Config) -> String {
    debug!("Calling ollama service with prompt: \n{}", prompt_and_context);

    let json_payload = prepare_ollama_payload(prompt_and_context, model_name, 8192, 8192, 0);
    debug!("{:?}", json_payload);

    let llm_output = http_post_json_ollama(ollama_svc_base_url, &ollama_client, json_payload);
    debug!("Model response:\n{}", llm_output);
    return llm_output;
}


/// Get user context (prompts) from application configuration.
///
/// # Arguments
///
/// * `app_config`:
///
/// returns: (String, String, String, String)
pub fn get_contexts_from_config(app_config: &Config) -> (String, String, String, String){

    let summary_part_context: String = get_cfg!(
        "summary_part_context", app_config, "Summarise the following text concisely.\n\nTEXT:\n"
    );

    let insights_part_context: String = get_cfg!(
        "insights_part_context", app_config, "Read the following text and extract actions from it.\n\nTEXT:\n"
    );

    let summary_exec_context: String = get_cfg!(
        "summary_exec_context", app_config, "Summarise the following text concisely.\n\nTEXT:\n"
    );

    let system_context: String = get_cfg!(
        "system_context", app_config, "You are an expert in analysing news and documents."
    );

    (summary_part_context, insights_part_context, summary_exec_context, system_context)
}


pub fn prepare_llm_parameters(app_config: &config::Config, task_prompt: String, plugin_name: &str) -> LLMParameters {

    // get llm sevice name:
    let mut llm_svc_name = String::from("ollama");
    match get_plugin_cfg!(plugin_name, "llm_service", app_config) {
        Some(param_val_str) => llm_svc_name = param_val_str,
        None => error!(
            "Error getting LLM service from config of plugin {}, using default value: {}",
            plugin_name,
            llm_svc_name
        )
    }

    // get overwrite config parameter
    let mut overwrite: bool = false;
    match get_plugin_config(&app_config, PLUGIN_NAME, "overwrite"){
        Some(param_val_str) => {
            match param_val_str.trim().parse(){
                Ok(param_val) => overwrite = param_val,
                Err(e) => error!("When parsing parameter 'overwrite' as integer value: {}", e)
            }
        }, None => error!("{}: Could not get parameter 'overwrite', using default value of: {}", PLUGIN_NAME, overwrite)
    };

    let mut max_word_count: usize = 850;
    match get_plugin_config(&app_config, PLUGIN_NAME, "max_word_count"){
        Some(param_val_str) => {
            match param_val_str.trim().parse(){
                Ok(param_val) => max_word_count = param_val,
                Err(e) => error!("When parsing parameter 'max_word_count': {}", e)
            }
        }, None => error!("{}: Could not get parameter 'max_word_count', using default: {}", PLUGIN_NAME, max_word_count)
    };

    // get fetch timeout config parameter
    let fetch_timeout: u64 = get_cfg_int!("model_api_timeout", app_config, 30) as u64;

    // set the model service connect timeout:
    let connect_timeout: u64 = fetch_timeout as u64;

    let max_llm_context_tokens: isize = get_cfg_int!("max_llm_context_tokens", app_config, 8192);

    let max_gen_tokens: isize = get_cfg_int!("max_gen_tokens", app_config, 8192);

    // build default client to be used by service endpoints
    let mut http_api_client = build_llm_api_client(
        connect_timeout,
        fetch_timeout,
        None,
        None
    );

    let mut summarize_function: fn(&str, &LLMParameters)-> String =
        llm::generate_using_ollama;

    let mut svc_base_url = String::from("http://127.0.0.1:11434/api/generate");

    let system_context: String = get_cfg!("system_context", app_config, "Act as an expert");

    // this is configured based on llm service:
    let mut model_name = String::from("gemma2_27b");

    match llm_svc_name.as_str() {
        "chatgpt" => {
            summarize_function = llm::generate_using_chatgpt;
            // prepare the http client for the REST service
            let custom_headers = prepare_chatgpt_headers(app_config);
            http_api_client = build_llm_api_client(
                connect_timeout,
                fetch_timeout,
                None,
                Some(custom_headers)
            );
            svc_base_url = get_cfg!("chatgpt_svc_url", app_config, "https://api.openai.com/v1/chat/completions");
            model_name = get_cfg!("chatgpt_model_name", app_config, "gpt-4o-mini");
        },
        "gemini" => {
            summarize_function = llm::generate_using_gemini;
            svc_base_url = get_cfg!("gemini_svc_url", app_config, "https://generativelanguage.googleapis.com/v1beta/models/");
            model_name = get_cfg!("gemini_model_name", app_config, "gemini-1.0-pro");
        },
        "ollama" => {
            summarize_function = llm::generate_using_ollama;
            svc_base_url = get_cfg!("ollama_svc_url", app_config, "http://127.0.0.1:11434/api/generate");
            model_name = get_cfg!("ollama_model_name", app_config, "llama3.1");
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
    use crate::llm::{GeminiRequestPayload, prepare_chatgpt_headers, prepare_gemini_api_payload, LLMParameters};
    use crate::network::build_llm_api_client;

    #[test]
    fn test_generate_using_llm(){
        let svc_url = "https://api.openai.com/v1/chat/completions";
        let model_name = "gpt-4o-mini";
        let empty_config = Config::builder().build().unwrap();
        let api_client = build_llm_api_client(
            15,
            300,
            None,
            Some(prepare_chatgpt_headers(&empty_config))
        );
        let llm_params = LLMParameters {
            llm_service: "chatgpt".to_string(),
            api_client,
            sumarize_fn: llm::generate_using_chatgpt,
            fetch_timeout: 300,
            overwrite_existing_value: true,
            save_intermediate: true,
            max_summary_wc: 8192,
            model_temperature: 0.0,
            prompt: "".to_string(),
            max_tok_gen: 8192,
            model_name: model_name.to_string(),
            num_context: 0,
            svc_base_url: svc_url.to_string(),
            system_context: "You are an expert".to_string(),
        };
        // let resp = llm::generate_using_chatgpt(
        //     "Why is the sky blue? Reply very concisely.",
        //     &llm_params
        // );
        // println!("Response from model = {:?}", resp);
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
