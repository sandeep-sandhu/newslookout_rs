// file: llm.rs

use std::cmp::{max, min};
use crate::{get_cfg, get_cfg_bool, get_cfg_int, get_plugin_cfg};
use std::collections::HashMap;
use std::error::Error;
use std::ops::Deref;
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use config::Config;
use log::{debug, error, info};
use regex::Regex;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use crate::{cfg, document, llm};
use crate::document::Document;
use crate::network::build_llm_api_client;
use crate::plugins::mod_summarize::PLUGIN_NAME;
use crate::cfg::{get_data_folder};
use crate::utils::{make_unique_filename, save_to_disk_as_json, split_by_word_count};

pub const MIN_ACCEPTABLE_INPUT_CHARS: usize = 50;
pub const MIN_ACCEPTABLE_SUMMARY_CHARS: usize = 25;
pub const MAX_TOKENS: f64 = 8000.0;
pub const TOKENS_PER_WORD: f64 = 1.33;
pub const MIN_GAP_BTWN_RQST_SECS: u64 = 6;

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


pub fn invoke_llm_func_with_lock(api_access_mutex: &Arc<Mutex<isize>>, llm_input_text: &str, llm_params: &LLMParameters, llm_func: fn(&str, &LLMParameters) -> String) -> String{

    // attempt lock, retrieve value of mutux
    let mut shared_val = api_access_mutex.lock().unwrap();
    // then execute llm service call,
    let result = llm_func(llm_input_text, llm_params);
    let duration_previous = Duration::from_secs(max(*shared_val,0) as u64);
    // get current timestamp:
    let mut duration_now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let seconds_elapsed = (duration_now-duration_previous).as_secs();
    info!("Shared API access elapsed seconds: {}", seconds_elapsed);
    // check if current tiemstamp is more than given duration from mutex value,
    if seconds_elapsed < MIN_GAP_BTWN_RQST_SECS {
        // add delay in seconds to make up for the remaining time:
        info!("Additional {} seconds delay to limit API requests/sec", MIN_GAP_BTWN_RQST_SECS-seconds_elapsed);
        thread::sleep(Duration::from_secs(MIN_GAP_BTWN_RQST_SECS -seconds_elapsed));
    }
    // set current timestamp and then return
    duration_now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    *shared_val = duration_now.as_secs() as isize;
    return result;
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct GeminiRequestPayload {
    pub contents: Vec<HashMap<String, Vec<HashMap<String, String>>>>,
    #[serde(rename = "safetySettings")]
    pub safety_settings: Vec<HashMap<String, String>>,
    #[serde(rename = "generationConfig")]
    pub generation_config: HashMap<String, usize>,
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

#[derive(Serialize, Deserialize, Debug)]
pub struct GenerationConfig {
    pub temperature: usize,
    #[serde(rename = "maxOutputTokens")]
    pub max_output_tokens: usize,
    pub response_modalities: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Parts {
    pub text: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Contents {
    pub role: String,
    pub parts: Parts,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GoogleGenAIRequestPayload {
    pub contents: Contents,
    #[serde(rename = "safety_settings")]
    pub safety_settings: Vec<HashMap<String, String>>,
    pub generation_config: GenerationConfig,
}


pub fn prepare_google_genai_api_payload(prompt: String, llm_params: &LLMParameters) -> GoogleGenAIRequestPayload {
    // put the parameters into the structure
    let json_payload = GoogleGenAIRequestPayload {
        contents: crate::llm::Contents{
            role: "USER".to_string(),
            parts: Parts { text: prompt },
        },
        safety_settings: vec![
            HashMap::from([
                ("category".to_string(), "HARM_CATEGORY_DANGEROUS_CONTENT".to_string()),
                ("threshold".to_string(), "BLOCK_ONLY_HIGH".to_string()),
            ])
        ],
        generation_config: crate::llm::GenerationConfig{
            temperature: llm_params.model_temperature as usize,
            max_output_tokens: llm_params.max_tok_gen,
            response_modalities: "TEXT".to_string(),
        },
    };
    json_payload
}


/*
Gemini Generative AI API:

curl -X POST -H "x-goog-api-key: PUT-YOUR-API-KEY-HERE" -H "Content-Type: application/json"  --data "{\"contents\":{\"role\": \"USER\",\"parts\": { \"text\": \"Explain the derivation of maxwells equations.\" },},\"generation_config\": {\"temperature\": \"0\", \"maxOutputTokens\": \"16300\",  \"response_modalities\": \"TEXT\",}, }" https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash-exp:generateContent

HTTP Response format:
{
  "candidates": [
    {
      "content": {
        "parts": [
          {
            "text": "Okay, let's break down the derivation of Maxwell's equations. It's not a single, linear derivation, but rather a process of combining experimental observations and theoretical insights. We'll go through the key steps and the underlying principles.\n\n**The Foundation: Experimental Laws**\n\nMaxwell's equations are built upon four fundamental experimental laws of electromagnetism:\n\n1. **Gauss's Law for Electricity:** This law relates the electric field to the distribution of electric charges. It states that the flux of the electric field through any closed surface is proportional to the enclosed electric charge. Mathematically:\n\n   * **Integral Form:**  ∮ **E** ⋅ d**A** = Q_enc / ε₀\n   * **Differential Form:** ∇ ⋅ **E** = ρ / ε₀\n\n   Where:\n     * **E** is the electric field\n     * d**A** is an infinitesimal area vector\n     * Q_enc is the enclosed charge\n     * ε₀ is the permittivity of free space\n     * ρ is the charge density\n     * ∇ ⋅ is the divergence operator\n\n2. **Gauss's Law for Magnetism:** This law states that there are no magnetic monopoles (isolated north or south poles). The magnetic flux through any closed surface is always zero. Mathematically:\n\n   * **Integral Form:** ∮ **B** ⋅ d**A** = 0\n   * **Differential Form:** ∇ ⋅ **B** = 0\n\n   Where:\n     * **B** is the magnetic field\n\n3. **Faraday's Law of Induction:** This law describes how a changing magnetic field induces an electric field. It states that the electromotive force (EMF) around a closed loop is equal to the negative rate of change of magnetic flux through the loop. Mathematically:\n\n   * **Integral Form:** ∮ **E** ⋅ d**l** = - dΦ_B / dt\n   * **Differential Form:** ∇ × **E** = - ∂**B** / ∂t\n\n   Where:\n     * d**l** is an infinitesimal length vector along the loop\n     * Φ_B is the magnetic flux\n     * ∂**B** / ∂t is the partial derivative of the magnetic field with respect to time\n     * ∇ × is the curl operator\n\n4. **Ampère's Law (with Maxwell's Correction):** This law, in its original form, related the magnetic field to the electric current. However, Maxwell realized it was incomplete and added a crucial term. The corrected law states that the magnetic field is generated by both electric currents and changing electric fields. Mathematically:\n\n   * **Integral Form:** ∮ **B** ⋅ d**l** = μ₀ (I_enc + ε₀ dΦ_E / dt)\n   * **Differential Form:** ∇ × **B** = μ₀ (**J** + ε₀ ∂**E** / ∂t)\n\n   Where:\n     * μ₀ is the permeability of free space\n     * I_enc is the enclosed current\n     * Φ_E is the electric flux\n     * **J** is the current density\n     * ∂**E** / ∂t is the partial derivative of the electric field with respect to time\n\n**Maxwell's Contribution: The Displacement Current**\n\nThe key innovation by Maxwell was the addition of the **displacement current** term (ε₀ ∂**E** / ∂t) to Ampère's Law. Here's why it was necessary:\n\n* **Inconsistency in Ampère's Law:** The original Ampère's Law, without the displacement current, was inconsistent when dealing with time-varying fields. For example, consider a capacitor being charged. Current flows into the capacitor, but there's no current flowing *between* the plates. Ampère's Law, without the correction, would predict no magnetic field between the plates, which is incorrect.\n* **Completing the Picture:** Maxwell realized that a changing electric field also creates a magnetic field, just like a current does. This \"displacement current\" term accounts for this effect and makes Ampère's Law consistent with the other laws of electromagnetism.\n* **Predicting Electromagnetic Waves:** The inclusion of the displacement current was crucial for Maxwell to predict the existence of electromagnetic waves. He showed that the interplay between changing electric and magnetic fields could propagate through space as waves, traveling at the speed of light.\n\n**The Four Maxwell's Equations**\n\nPutting it all together, Maxwell's equations in their differential form are:\n\n1. **Gauss's Law for Electricity:** ∇ ⋅ **E** = ρ / ε₀\n2. **Gauss's Law for Magnetism:** ∇ ⋅ **B** = 0\n3. **Faraday's Law of Induction:** ∇ × **E** = - ∂**B** / ∂t\n4. **Ampère-Maxwell Law:** ∇ × **B** = μ₀ (**J** + ε₀ ∂**E** / ∂t)\n\n**Key Takeaways**\n\n* **Experimental Basis:** Maxwell's equations are rooted in experimental observations of electric and magnetic phenomena.\n* **Unification:** They unify electricity and magnetism into a single framework, electromagnetism.\n* **Prediction of Electromagnetic Waves:** They predict the existence of electromagnetic waves, including light.\n* **Fundamental Laws:** They are fundamental laws of physics, applicable across a wide range of scales.\n* **Differential Form:** The differential form is particularly useful for theoretical analysis and understanding the local behavior of fields.\n\n**In Summary**\n\nThe derivation of Maxwell's equations is not a single, straightforward process. It's a culmination of experimental findings, theoretical insights, and a crucial correction by Maxwell himself. These equations are not just a set of formulas; they represent a profound understanding of the fundamental nature of electromagnetism and have revolutionized our understanding of the universe. They are the cornerstone of modern physics and technology.\n"
          }
        ],
        "role": "model"
      },
      "finishReason": "STOP",
      "safetyRatings": [
        {
          "category": "HARM_CATEGORY_HATE_SPEECH",
          "probability": "NEGLIGIBLE"
        },
        {
          "category": "HARM_CATEGORY_DANGEROUS_CONTENT",
          "probability": "NEGLIGIBLE"
        },
        {
          "category": "HARM_CATEGORY_HARASSMENT",
          "probability": "NEGLIGIBLE"
        },
        {
          "category": "HARM_CATEGORY_SEXUALLY_EXPLICIT",
          "probability": "NEGLIGIBLE"
        }
      ],
      "citationMetadata": {
        "citationSources": [
          {
            "startIndex": 425,
            "endIndex": 546,
            "uri": "https://www.physicsforums.com/threads/using-gauss-law-on-a-solid-annular-sphere.472001/"
          },
          {
            "startIndex": 455,
            "endIndex": 613
          },
          {
            "startIndex": 1343,
            "endIndex": 1556
          }
        ]
      },
      "avgLogprobs": "Infinity"
    }
  ],
  "usageMetadata": {
    "promptTokenCount": 9,
    "candidatesTokenCount": 1267,
    "totalTokenCount": 1276
  },
  "modelVersion": "gemini-2.0-flash-exp"
}

generation_config options for specifying a json response in specified schema:
"response_mime_type": "application/json",
"response_schema": {"type": "object", "properties": {"recipe_name": {"type": "string"}}}
*/

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
        safety_settings: vec![HashMap::from([
            ("category".to_string(), "HARM_CATEGORY_DANGEROUS_CONTENT".to_string()),
            ("threshold".to_string(), "BLOCK_ONLY_HIGH".to_string())
        ])],
        generation_config: HashMap::from([
            ("temperature".to_string(), llm_params.model_temperature as usize),
            ("maxOutputTokens".to_string(), llm_params.max_tok_gen),
        ]),
    };
    return json_payload;
}

pub fn http_post_json_google_genai<'post>(service_url: &str, client: &reqwest::blocking::Client, json_payload: GoogleGenAIRequestPayload) -> Option<String> {

    let mut prompt_token_count: u64 = 0;
    let mut candidates_token_count: u64 = 0;
    let mut total_token_count: u64 = 0;
    let mut api_result = None;

    // add json payload to body
    match client.post(service_url)
        .json(&json_payload)
        .send() {
        Result::Ok(resp) => {
            match resp.status() {
                StatusCode::OK => {
                    match resp.json::<serde_json::value::Value>(){
                        Result::Ok( json ) => {
                            debug!("Google GenAI API response:\n{:?}", json);
                            if let Some(resp_error) = json.get("error"){
                                if let Some(error_message) = resp_error.get("message"){
                                    if let Some(err_message) = error_message.as_str(){
                                        error!("API Error message: {}", err_message);
                                        return None
                                    }
                                }
                            }
                            if let Some(resp_candidates) = json.get("candidates"){

                                if let Some(first_candidate) = resp_candidates.get(0) {

                                    if let Some(resp_content) = first_candidate.get("content") {
                                        if let Some(parts) = resp_content.get("parts") {

                                            if let Some(first_part) = parts.get(0) {

                                                if let Some(text_part) = first_part.get("text") {
                                                    api_result = Some(text_part.to_string())
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            if let Some(resp_usage_metadata) = json.get("usageMetadata"){
                                if let Some(prompt_token_count_str) = resp_usage_metadata.get("promptTokenCount"){
                                    prompt_token_count = prompt_token_count_str.as_u64().unwrap_or_default();
                                }
                                if let Some(candidates_token_count_str) = resp_usage_metadata.get("candidatesTokenCount"){
                                    candidates_token_count = candidates_token_count_str.as_u64().unwrap_or_default();
                                }
                                if let Some(total_token_count_str) = resp_usage_metadata.get("totalTokenCount"){
                                    total_token_count = total_token_count_str.as_u64().unwrap_or_default();
                                }
                            }
                        },
                        Err(e) => {
                            error!("When retrieving json from Google GenAI API response: {}", e);
                            if let Some(err_source) = e.source(){
                                error!("Caused by: {}", err_source);
                            }
                        },
                    }
                },
                StatusCode::NOT_FOUND => {
                    error!("Google GenAI API: Service not found!");
                },
                StatusCode::PAYLOAD_TOO_LARGE => {
                    error!("Google GenAI API: Request payload is too large!");
                },
                StatusCode::TOO_MANY_REQUESTS => {
                    error!("Google GenAI API: Too many requests. Exceeded the Provisioned Throughput.");
                },
                s => error!("Google GenAI API response status: {s:?}"),
            };
        }
        Err(e) => {
            error!("When posting json payload to Google GenAI API service: {}", e);
            if let Some(err_source) = e.source(){
                error!("Caused by: {}", err_source);
            }
        }
    }
    info!("Google GenAI prompt token count: {},  candidatesTokenCount: {}, totalTokenCount: {}",
        prompt_token_count, candidates_token_count, total_token_count);
    return api_result;
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
            match resp.status() {
                StatusCode::OK => {
                    match resp.json::<serde_json::value::Value>() {
                        Result::Ok(json) => {
                            debug!("Google Gemini API response:\n{:?}", json);
                            // Object {"error": Object {"code": Number(404), "message": String("models/gemini-flash-1.5 is not found for API version v1beta, or is not supported for generateContent. Call ListModels to see the list of available models and their supported methods."), "status": String("NOT_FOUND")}}
                            if let Some(resp_candidates) = json.get("candidates") {
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
                            if let Some(err_source) = e.source() {
                                error!("Caused by: {}", err_source);
                            }
                        },
                    }
                },
                StatusCode::NOT_FOUND => {
                    error!("Gemini API: Service not found!");
                },
                StatusCode::PAYLOAD_TOO_LARGE => {
                    error!("Gemini API: Request payload is too large!");
                },
                StatusCode::TOO_MANY_REQUESTS => {
                    error!("Gemini API: Too many requests. Exceeded the Provisioned Throughput.");
                }
                s => error!("Gemini API: Received response status: {s:?}")
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
            match resp.status() {
                StatusCode::OK => {
                    match resp.json::<serde_json::value::Value>() {
                        Ok(json) => {
                            debug!("ChatGPT API: model response:\n{:?}", json);
                            if let Some(choices) = json.get("choices") {
                                if let Some(first_choice) = choices.get(0) {
                                    if let Some(message) = first_choice.get("message") {
                                        if let Some(content) = message.get("content") {
                                            return content.to_string();
                                        }
                                    }
                                    if let Some(logprobs) = first_choice.get("logprobs") {
                                        // get object: "logprobs" , get attrib: "content" array of Object -> {"bytes", "logprob"}
                                        if let Some(logprobs_content) = logprobs.get("content") {
                                            match logprobs_content.as_array() {
                                                None => {}
                                                Some(logprobs_vec) => {
                                                    // let mut text_confidence = "";
                                                    // for log_prob_pair in logprobs_vec{
                                                    //     Extract from pair: {"bytes", "logprob"}
                                                    //     let linear_prob = round(exp(logprob) * 100, 2);
                                                    // }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            // get and print string:  "model"
                            // get and print object/dict: "usage" -> get integer attributes: "prompt_tokens", "total_tokens"
                        },
                        Err(e) => {
                            error!("ChatGPT API: When retrieving json from response: {}", e);
                            if let Some(err_source) = e.source() {
                                error!("Caused by: {}", err_source);
                            }
                            info!("ChatGPT Payload: {:?}", json_payload);
                        },
                    }
                },
                StatusCode::NOT_FOUND => {
                    error!("ChatGPT API: Service not found!");
                },
                StatusCode::PAYLOAD_TOO_LARGE => {
                    error!("ChatGPT API: Request payload is too large!");
                },
                StatusCode::TOO_MANY_REQUESTS => {
                    error!("ChatGPT API: Too many requests. Exceeded the Provisioned Throughput.");
                }
                s => error!("ChatGPT API: Received response status: {s:?}")
            }
        }
        Err(e) => {
            error!("ChatGPT API: When posting json payload to service: {}", e);
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
pub fn generate_using_ollama_api(
    ollama_svc_base_url: &str,
    ollama_client: &reqwest::blocking::Client,
    model_name: &str,
    prompt_and_context: &str,
    app_config: &Config
) -> String {
    debug!("Calling ollama service with prompt: \n{}", prompt_and_context);

    let json_payload = prepare_ollama_payload(prompt_and_context, model_name, 8192, 8192, 0);
    debug!("{:?}", json_payload);

    let llm_output = http_post_json_ollama(ollama_svc_base_url, &ollama_client, json_payload);
    debug!("Model response:\n{}", llm_output);
    return llm_output;
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
    match get_plugin_cfg!(PLUGIN_NAME, "overwrite", &app_config) {
        Some(param_val_str) => {
            match param_val_str.trim().parse(){
                Ok(param_val) => overwrite = param_val,
                Err(e) => error!("When parsing parameter 'overwrite' as integer value: {}", e)
            }
        }, None => error!("{}: Could not get parameter 'overwrite', using default value of: {}", PLUGIN_NAME, overwrite)
    };

    let mut max_word_count: usize = 850;
    match get_plugin_cfg!(PLUGIN_NAME, "max_word_count", &app_config) {
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
    let connect_timeout: u64 = fetch_timeout;

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

    // this is configured separately for each llm service, see below:
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
            svc_base_url = get_cfg!("gemini_svc_url", app_config, "https://generativelanguage.googleapis.com/v1beta/models");
            model_name = get_cfg!("gemini_model_name", app_config, "gemini-1.5-flash");
        },
        "google_genai" => {
            summarize_function = llm::generate_using_google_genai;
            // prepare the http client for the REST service
            let custom_headers = prepare_googlegenai_headers(app_config);
            http_api_client = build_llm_api_client(
                connect_timeout,
                fetch_timeout,
                None,
                Some(custom_headers)
            );
            svc_base_url = get_cfg!("google_genai_svc_url", app_config, "https://generativelanguage.googleapis.com/v1beta/models");
            model_name = get_cfg!("google_genai_model_name", app_config, "gemini-2.0-flash-exp");
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
    // prepare url:
    let api_url = format!("{}/{}:generateContent?key={}", llm_params.svc_base_url, llm_params.model_name, api_key);

    let prompt = format!("{}\n{}", llm_params.prompt, input_text);

    let json_payload = prepare_gemini_api_payload(prompt, llm_params);

    let llm_output = http_post_json_gemini(api_url.as_str(), &llm_params.api_client, json_payload);
    info!("Gemini Generated content: {}", llm_output);

    return llm_output;
}


/// Add headers for google gen ai api:
/// "x-goog-api-key: PUT-YOUR-API-KEY-HERE"
/// "Content-Type: application/json"
///
/// # Arguments
///
/// * `app_config`: The application configuration
///
/// returns: HeaderMap<HeaderValue>
pub fn prepare_googlegenai_headers(_app_config: &Config) -> HeaderMap {
    let mut custom_headers = HeaderMap::new();
    const GOOG_API_HEADER: reqwest::header::HeaderName = reqwest::header::HeaderName::from_static("x-goog-api-key");
    let api_key = std::env::var("GOOGLE_API_KEY").unwrap_or(String::from(""));
    if let Ok(header_apikey_val) = HeaderValue::from_str(api_key.as_str()) {
        custom_headers.insert(GOOG_API_HEADER, header_apikey_val);
    }
    custom_headers.insert(reqwest::header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    custom_headers
}

/// Expects API base url as https://generativelanguage.googleapis.com/v1beta/models
///
/// # Arguments
///
/// * `prompt`:
/// * `llm_params`:
///
/// returns: String
///
/// # Examples
///
/// ```
///
/// ```
pub fn generate_using_google_genai(prompt: &str, llm_params: &LLMParameters) -> String {

    // prepare url for the api:
    let api_url = format!("{}/{}:generateContent", llm_params.svc_base_url, llm_params.model_name);

    let json_payload: GoogleGenAIRequestPayload = prepare_google_genai_api_payload(prompt.to_string(), llm_params);

    if let Some(llm_output) = http_post_json_google_genai(api_url.as_str(), &llm_params.api_client, json_payload) {
        info!("Google GenAI API generated content: {}", llm_output);
        return llm_output;
    } else{
        error!("No content generated by Google GenAI API");
        return String::from("");
    }
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
    #[serde(rename = "taskID")]
    pub task_id: usize,
    pub keep_alive: String,
    /// For options such as: "temperature": 0, "num_predict": 8192, "num_ctx": 8192,
    pub options: HashMap<String, usize>,
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
        task_id: 42, // what else!
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
    use log::{debug, info, error};
    use crate::llm;
    use crate::llm::{GeminiRequestPayload, prepare_chatgpt_headers, prepare_gemini_api_payload, LLMParameters, prepare_googlegenai_headers};
    use crate::network::build_llm_api_client;

    #[test]
    fn test_generate_using_chatgpt_llm(){
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
        let genfunc = llm_params.sumarize_fn;
        // let resp = genfunc("Why is the sky blue? Reply very concisely.", &llm_params);
        // println!("Response from model = {:?}", resp);
        assert_eq!(1,1);
    }

    #[test]
    fn test_generate_using_gemini_llm(){
        let api_client = build_llm_api_client(100, 3000, None, None);
        let svc_url = "https://generativelanguage.googleapis.com/v1beta/models/";
        let model_name = "gemini-1.5-flash-latest";
        let empty_config = Config::builder().build().unwrap();
        let llm_params = LLMParameters {
            llm_service: "gemini".to_string(),
            api_client,
            sumarize_fn: llm::generate_using_gemini,
            fetch_timeout: 300,
            overwrite_existing_value: true,
            save_intermediate: true,
            max_summary_wc: 8192,
            model_temperature: 0.0,
            prompt: "Answer this question concisely. ".to_string(),
            max_tok_gen: 8192,
            model_name: model_name.to_string(),
            num_context: 0,
            svc_base_url: svc_url.to_string(),
            system_context: "You are an expert".to_string(),
        };
        let genfunc = llm_params.sumarize_fn;
        // let resp = genfunc("Why is the sky blue? Reply very concisely.", &llm_params);
        // println!("Response from model = {:?}", resp);
        assert_eq!(1,1);
    }


    #[test]
    fn test_generate_using_google_genai_llm(){
        let svc_url = "https://generativelanguage.googleapis.com/v1beta/models";
        let model_name = "gemini-2.0-flash-exp";
        let empty_config = Config::builder().build().unwrap();
        let api_client = build_llm_api_client(
            15,
            300,
            None,
            Some(prepare_googlegenai_headers(&empty_config))
        );
        let llm_params = LLMParameters {
            llm_service: "google_genai".to_string(),
            api_client,
            sumarize_fn: llm::generate_using_google_genai,
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
        let genfunc = llm_params.sumarize_fn;
        // let resp = genfunc("Why is the sky blue? Reply very concisely.", &llm_params);
        // println!("Response from model = {:?}", resp);
        assert_eq!(1,1);
    }

}
