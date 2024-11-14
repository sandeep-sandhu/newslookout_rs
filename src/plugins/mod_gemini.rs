use std::collections::HashMap;
use std::error::Error;
use std::sync::mpsc::{Receiver, Sender};
use config::Config;
use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use crate::document;
use crate::llm::update_doc;
use crate::network::build_llm_api_client;
use crate::utils::get_plugin_config;

pub const PLUGIN_NAME: &str = "mod_gemini";
pub const PUBLISHER_NAME: &str = "LLM Processing via Gemini API Service";


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
    let mut doc_counter: u32 = 0;

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

    let connect_timeout: u64 = 15;
    // set a low connect timeout:
    // prepare the http client for the REST service
    // TODO: add proxy server url from config
    let api_client = build_llm_api_client(connect_timeout, fetch_timeout, None, None);

    // process each document received and return back to next handler:
    for doc in rx {

        info!("{}: Started processing document titled - {}", PLUGIN_NAME, doc.title);
        let updated_doc:document::Document = update_doc(
            &api_client,
            doc,
            PLUGIN_NAME,
            &app_config,
            generate_using_llm
        );

        //for each document received in channel queue, send via transmit queue:
        match tx.send(updated_doc) {
            Result::Ok(_) => {doc_counter += 1;},
            Err(e) => error!("{}: When transmitting processed doc via tx: {}", PLUGIN_NAME, e)
        }
    }

    info!("{}: Completed processing {} documents.", PLUGIN_NAME, doc_counter);
}


/// Posts the prompt to generate text.
/// Converts the url, model and api key to full ul for the api service for non-stream content generation:
/// e.g. https://generativelanguage.googleapis.com/v1beta/models/gemini-1.5-flash-latest:generateContent?key=$GOOGLE_API_KEY
/// Prepares the json payload.
/// Posts the json payload and retrieves the response payload.
/// Converts the response from json to struct and returns the model generated text.
///
/// # Arguments
///
/// * `svc_base_url`:
/// * `http_api_client`:
/// * `model_name`:
/// * `prompt_text`:
/// * `app_config`:
///
/// returns: String
pub fn generate_using_llm(svc_base_url: &str, http_api_client: &reqwest::blocking::Client, model_name: &str, prompt_text: &str, app_config: &Config) -> String {

    let system_instruct= "You are an expert. Keep the tone professional + straightforward.";
    debug!("Calling gemini service with prompt: \n{}", prompt_text);

    // get key from env variable: GOOGLE_API_KEY
    let api_key = std::env::var("GOOGLE_API_KEY").unwrap_or(String::from(""));
    // prepare url for the api
    let api_url = format!("{}{}:generateContent?key={}", svc_base_url, model_name, api_key);

    let json_payload = prepare_payload(prompt_text, 8192, 8192, 0);
    debug!("{}: JSON PAYload:\n {:?}\n", PLUGIN_NAME, json_payload);

    let llm_output = http_post_json_gemini(api_url.as_str(), &http_api_client, json_payload);
    return llm_output;
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct RequestPayload {
    pub contents: Vec<HashMap<String, Vec<HashMap<String, String>>>>,
    pub safetySettings: Vec<HashMap<String, String>>,
    pub generationConfig: HashMap<String, usize>,
}

// Response JSON format:
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


/// Prepare the JSON payload for sending to the LLM API service.
///
/// # Arguments
///
/// * `prompt`:
/// * `system_instruct`:
/// * `num_context`:
/// * `max_tok_gen`:
/// * `temperature`:
///
/// returns: RequestPayload
pub fn prepare_payload(prompt: &str, num_context: usize, max_tok_gen: usize, temperature: usize) -> RequestPayload {
    // put the parameters into the structure
    let json_payload = RequestPayload {
        contents: vec![
            HashMap::from([
            ("parts".to_string(), vec![HashMap::from([("text".to_string(), prompt.to_string())])])
             ])],
        safetySettings: vec![HashMap::from([
            ("category".to_string(), "HARM_CATEGORY_DANGEROUS_CONTENT".to_string()),
            ("threshold".to_string(), "BLOCK_ONLY_HIGH".to_string())
        ])],
        generationConfig: HashMap::from([
            ("temperature".to_string(), temperature),
            ("maxOutputTokens".to_string(), max_tok_gen),
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
pub fn http_post_json_gemini<'post>(service_url: &str, client: &reqwest::blocking::Client, json_payload: RequestPayload) -> String {
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


#[cfg(test)]
mod tests {
    use config::Config;
    use log::debug;
    use crate::plugins::mod_gemini;
    use crate::plugins::mod_gemini::RequestPayload;

    #[test]
    fn test_run_worker_thread() {
        // TODO: implement this
        assert_eq!(1, 1);
    }

    #[test]
    fn test_generate_using_llm(){
        let api_client = mod_gemini::build_llm_api_client(100, 3000, None, None);
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

    #[test]
    fn test_prepare_payload(){
        let json_struct = mod_gemini::prepare_payload("Why is the sky blue?", 8192, 8192, 0);
        if let Ok(json_text) = serde_json::to_string(&json_struct){
            debug!("{}", json_text);
            let deserialized_json:RequestPayload = serde_json::from_str(json_text.as_str()).unwrap();
            assert_eq!(deserialized_json.contents.len(), 1);
        }
    }

}
