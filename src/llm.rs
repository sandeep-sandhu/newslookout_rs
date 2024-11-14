// file: llm.rs

use std::collections::HashMap;
use std::path::Path;
use config::Config;
use log::{error, info};
use reqwest::blocking::Client;
use crate::document;
use crate::plugins::mod_ollama::PLUGIN_NAME;
use crate::utils::{build_llm_prompt, get_contexts_from_config, get_data_folder, get_plugin_config, make_unique_filename, save_to_disk_as_json};

pub fn update_doc(ollama_client: &Client, mut input_doc: document::Document, app_config: &Config, llm_fn: fn(&str, &Client, &str, &str, &Config) -> String) -> document::Document{

    let loopiters = input_doc.text_parts.len() as i32;
    info!("{}: Starting to process {} parts of document - '{}'", PLUGIN_NAME, loopiters, input_doc.title);

    let mut model_name: String = String::from("llama3.1");
    match get_plugin_config(&app_config, PLUGIN_NAME, "model_name"){
        Some(param_val_str) => {
            model_name =param_val_str;
        }, None => {}
    };

    let mut svc_url: String = String::from("http://127.0.0.1/");
    match get_plugin_config(&app_config, PLUGIN_NAME, "svc_url"){
        Some(param_val_str) => {
            svc_url = format!("{}api/generate", param_val_str);
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

    let mut save_intermediate: bool = false;
    match get_plugin_config(&app_config, PLUGIN_NAME, "save_intermediate"){
        Some(param_val_str) => {
            match param_val_str.trim().parse(){
                Result::Ok(param_bool) => save_intermediate = param_bool,
                Err(e) => error!("When parsing parameter 'save_intermediate' as boolean value: {}", e)
            }
        }, None => error!("Could not get parameter 'save_intermediate', using default as false")
    };

    let binding = get_data_folder(&app_config);
    let data_folder_name = binding.to_str().unwrap_or_default();

    let mut temperature: f64 = 0.0;
    match get_plugin_config(&app_config, PLUGIN_NAME, "temperature"){
        Some(param_val_str) => {
            match param_val_str.trim().parse(){
                Result::Ok(param_float) => temperature = param_float,
                Err(e) => error!("When parsing parameter 'temperature' as float value: {}", e)
            }
        }, None => error!("Could not get parameter 'temperature', using default value of: {}", temperature)
    };

    // get contexts from config file:
    let (summary_part_context, insights_part_context, summary_exec_context, system_context) = get_contexts_from_config(&app_config);

    // make full path by joining folder to unique filename
    let json_file_path = Path::new(data_folder_name).join(make_unique_filename(&input_doc, "json"));
    input_doc.filename = String::from(json_file_path.as_path().to_str().expect("Not able to convert path to string"));

    // pop out each part, process it and push to new vector, replace this updated vector in document
    let mut updated_text_parts:  Vec<HashMap<String, String>> = Vec::new();
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
                info!("{}: Processing text part #{}", PLUGIN_NAME, key);

                // check if there is a key "summary", if so:
                if let Some(existing_summary) = text_part_map.get("summary") {
                    if (overwrite == false) & (existing_summary.len()>3) {
                        info!("Not overwriting existing summary for part #{}", key);
                        to_generate_summary = false;
                    }
                }
                if to_generate_summary == true{
                    let summary_part_prompt = build_llm_prompt(model_name.as_str(), system_context.as_str(), summary_part_context.as_str(), text_part.as_str());
                    // call service with payload to generate summary of part:
                    let summary_part = llm_fn(svc_url.as_str(), ollama_client, model_name.as_str(), summary_part_prompt.as_str(), app_config);
                    all_summaries.push_str("\n");
                    all_summaries.push_str(summary_part.as_str());
                    text_part_map_clone.insert("summary".to_string(), summary_part);
                }

                if let Some(existing_insights) = text_part_map.get("insights") {
                    if (overwrite == false) & (existing_insights.len()>3) {
                        info!("Not overwriting existing insights for part #{}", key);
                        to_generate_insights = false;
                    }
                }
                if to_generate_insights == true {
                    // call service with payload to generate insights:
                    let insights_part_prompt = build_llm_prompt(model_name.as_str(), system_context.as_str(), insights_part_context.as_str(), text_part.as_str());
                    // call service with payload to generate insights of part:s
                    let insights_part = llm_fn(svc_url.as_str(), ollama_client, model_name.as_str(), insights_part_prompt.as_str(), app_config);
                    all_actions.push_str(insights_part.as_str());
                    text_part_map_clone.insert("insights".to_string(), insights_part);
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
    let exec_summary_prompt= build_llm_prompt(model_name.as_str(), system_context.as_str(), summary_exec_context.as_str(), all_summaries.as_str());
    // call service with payload to generate summary:
    let exec_summary= llm_fn(svc_url.as_str(), ollama_client, model_name.as_str(), exec_summary_prompt.as_str(), app_config);
    // add to generated_content
    input_doc.generated_content.insert("exec_summary".to_string(), exec_summary);

    // generate the actions summary:
    let actions_summary_prompt= build_llm_prompt(model_name.as_str(), system_context.as_str(), summary_exec_context.as_str(), all_actions.as_str());
    // call service with payload to generate actions summary:
    let actions_summary= llm_fn(svc_url.as_str(), ollama_client, model_name.as_str(), actions_summary_prompt.as_str(), app_config);
    input_doc.generated_content.insert("actions_summary".to_string(), actions_summary);

    save_to_disk_as_json(&input_doc, json_file_path.to_str().unwrap_or_default());

    info!("{}: Model {} completed processing document titled: '{}' ", PLUGIN_NAME, input_doc.title, model_name);
    return input_doc;
}

