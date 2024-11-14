use std::collections::HashMap;
use std::error::Error;
use std::sync::mpsc::{Receiver, Sender};
use config::Config;
use log::{debug, error, info};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, InvalidHeaderName};
use serde::{Deserialize, Serialize};
use crate::document;
use crate::llm::update_doc;
use crate::network::build_llm_api_client;
use crate::utils::get_plugin_config;

pub const PLUGIN_NAME: &str = "mod_chatgpt";
pub const PUBLISHER_NAME: &str = "LLM Processing via ChatGPT API Service";

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
pub fn process_data(tx: Sender<document::Document>, rx: Receiver<document::Document>, app_config: &Config){

    info!("{}: Getting configuration specific to the module.", PLUGIN_NAME);

    // get fetch timeout config parameter
    let mut fetch_timeout: u64 = 150;
    match get_plugin_config(&app_config, crate::plugins::mod_ollama::PLUGIN_NAME, "fetch_timeout"){
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
    // TODO: add proxy server url from config
    let mut custom_headers = prepare_http_headers(app_config);
    let api_client = build_llm_api_client(connect_timeout, fetch_timeout, None, Some(custom_headers));

    // process each document received and return back to next handler:
    for doc in rx {
        info!("{}: Started processing document titled - '{}',  with #{} parts.",
            PLUGIN_NAME, doc.title, doc.text_parts.len()
        );

        let updated_doc:document::Document = update_doc(
            &api_client,
            doc,
            &app_config,
            generate_using_llm
        );

        //for each document received in channel queue, send it to next queue:
        match tx.send(updated_doc) {
            Result::Ok(_) => {},
            Err(e) => error!("{}: When sending processed doc via tx: {}", PLUGIN_NAME, e)
        }
    }
    info!("{}: Completed processing all data.", PLUGIN_NAME);
}

pub fn generate_using_llm(svc_base_url: &str, http_api_client: &reqwest::blocking::Client, model_name: &str, prompt_text: &str, app_config: &Config) -> String {
    debug!("Calling chatgpt service with prompt: \n{}", prompt_text);
    let system_context = "You are an expert. Keep the tone professional + straightforward.";
    let json_payload = prepare_payload(prompt_text, model_name, system_context,8192, 8192, 0);

    debug!("{:?}", json_payload);
    let llm_output = http_post_json_chatgpt(svc_base_url, &http_api_client, json_payload);

    debug!("Chatgpt Model response:\n{}", llm_output);
    return llm_output;
}

pub fn prepare_http_headers(app_config: &Config) -> HeaderMap {
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

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct RequestPayload {
    pub model: String,
    pub messages: Vec<HashMap<String, String>>,
    pub temperature: f64,
    max_completion_tokens: usize
}

pub fn prepare_payload(prompt: &str, model: &str, system_context: &str, num_context: usize, max_tok_gen: usize, temperature: usize) -> RequestPayload {
    // put the parameters into the structure
    let json_payload = RequestPayload {
        model: model.to_string(),
        messages: vec![
            HashMap::from([
                ("role".to_string(), "system".to_string()),
                ("content".to_string(), system_context.to_string())
            ]),
            HashMap::from([
                ("role".to_string(), "user".to_string()),
                ("content".to_string(), prompt.to_string())
            ]),
        ],
        temperature: temperature as f64,
        max_completion_tokens: max_tok_gen,
    };
    // {
    //      "model": "gpt-4o-mini",
    //      "messages": [{"role": "user", "content": "Say this is a test!"}],
    //      "temperature": 0.7
    //    }
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
pub fn http_post_json_chatgpt(service_url: &str, client: &reqwest::blocking::Client, json_payload: RequestPayload) -> String{
    // add json payload to body
    match client.post(service_url)
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


#[cfg(test)]
mod tests {
    use config::Config;
    use log::debug;
    use crate::plugins::{mod_chatgpt};

    #[test]
    fn test_run_worker_thread() {
        // TODO: implement this
        assert_eq!(1, 1);
    }

    #[test]
    fn test_generate_using_llm(){
        let empty_config = Config::builder().build().unwrap();
        let api_client = mod_chatgpt::build_llm_api_client(
            15,
            300,
            None,
            Some(mod_chatgpt::prepare_http_headers(&empty_config))
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

}
