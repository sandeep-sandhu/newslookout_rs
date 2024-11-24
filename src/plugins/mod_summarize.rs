// file: mod_summarize

use std::collections::HashMap;
use std::error::Error;
use std::sync::mpsc::{Receiver, Sender};
use config::Config;
use log::{debug, error, info};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, InvalidHeaderName};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::{document, llm};
use crate::llm::{prepare_llm_parameters, LLMParameters, MAX_TOKENS, MIN_ACCEPTABLE_SUMMARY_CHARS, TOKENS_PER_WORD};
use crate::network::build_llm_api_client;
use crate::utils::{get_plugin_config, word_count};

pub const PLUGIN_NAME: &str = "mod_summarize";
pub const PUBLISHER_NAME: &str = "Text Summarization";

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

    let mut prompt_summary_part = String::from("prompt_summary_part");
    match app_config.get_string("prompt_summary_part") {
        Ok(param_val) => prompt_summary_part = param_val,
        Err(e) => error!("When getting prompt part summary: {}, using default value: {}",
            e, prompt_summary_part)
    }

    let mut prompt_summary_exec = String::from("prompt_summary_exec");
    match app_config.get_string("prompt_summary_exec") {
        Ok(param_val) => prompt_summary_exec = param_val,
        Err(e) => error!("When getting prompt exec summary: {}, using default value: {}",
            e, prompt_summary_exec)
    }

    let mut llm_params = prepare_llm_parameters(app_config, prompt_summary_part.clone());

    // process each document received and return back to next handler:
    for mut doc in rx {

        info!("{}: Started processing document titled - '{}',  with {} parts.",
            PLUGIN_NAME, doc.title, doc.text_parts.len()
        );

        update_doc(&mut llm_params, &mut doc, prompt_summary_part.as_str(), prompt_summary_exec.as_str());

        //for each document received in channel queue, send it to next queue:
        match tx.send(doc) {
            Result::Ok(_) => {doc_counter += 1;},
            Err(e) => error!("{}: When sending processed doc via tx: {}", PLUGIN_NAME, e)
        }
    }
    info!("{}: Completed processing {} documents.", PLUGIN_NAME, doc_counter);
}


fn update_doc(llm_params: &mut LLMParameters, raw_doc: &mut document::Document, prompt_part: &str, prompt_exec_summary: &str) {


    let summarize_fn = llm_params.sumarize_fn;

    let mut all_summaries: String = String::new();

    llm_params.prompt = prompt_part.to_string();

    // pop out each part, process it and push to new vector, replace this updated vector in document
    let mut updated_text_parts:  Vec<HashMap<String, Value>> = Vec::new();

    while raw_doc.text_parts.is_empty() == false {

        match &raw_doc.text_parts.pop(){
            None => {break;}
            Some(text_part) => {

                // if we should not overwrite, check whether a value exists
                if llm_params.overwrite_existing_value == false {
                    match text_part.get("summary"){
                        Some(summary) => {
                            if summary.to_string().len() > MIN_ACCEPTABLE_SUMMARY_CHARS {
                                // add to part summaries
                                all_summaries.push_str(&format!("{},",summary));
                                // add as-is to output vector
                                let mut text_part_map_clone = text_part.clone();
                                updated_text_parts.push(text_part_map_clone);
                                // do not proceed further to generate summary:
                                continue;
                            }
                        }
                        _ => {}
                    }
                }

                // else, compute the summary for this part:
                match text_part.get("text"){
                    Some(text_string) => {
                        info!("Started generating summary for text part");
                        let mut text_part_map_clone = text_part.clone();
                        let summary_text = summarize_fn(
                            text_string.to_string().as_str(),
                            llm_params
                        );
                        _ = text_part_map_clone.insert("summary".to_string(), Value::String(summary_text.clone()));
                        updated_text_parts.push(text_part_map_clone);
                        all_summaries.push_str("\n");
                        all_summaries.push_str(summary_text.as_str());
                    },
                    None => {}
                }
            }
        }
    }
    // put updated text_parts into document:
    updated_text_parts.reverse();
    raw_doc.text_parts = updated_text_parts;

    // prepare for executive summary:
    let exec_summ_prompt = format!(
        "{}\n{}\nPublish Date: {}",
        prompt_exec_summary,
        raw_doc.title,
        raw_doc.publish_date
    );
    llm_params.prompt = exec_summ_prompt.to_string();
    // before generating summary, calculate how long is the context:
    let full_context = word_count(
        &format!("{}{}", exec_summ_prompt, all_summaries)
    ) as f64 * TOKENS_PER_WORD;
    let count_exec_summ_sub_parts = full_context / MAX_TOKENS;

    // in the end check if existing exec_summary is longer than min_acceptable,
    // and length of current summaries is more than acceptable summary, then.
    // get exec summary prepared:
    match raw_doc.generated_content.get("exec_summary"){
        Some(existing_exec_summary) => {
            // exec summary exists, but it is not adequate, so re-generate:
            if existing_exec_summary.to_string().len() < MIN_ACCEPTABLE_SUMMARY_CHARS {
                info!("Executive summary would need to be generated in {} parts.",
                    count_exec_summ_sub_parts);
                let exec_summary_text = summarize_fn(
                    all_summaries.as_str(),
                    llm_params
                );
                raw_doc.generated_content.insert("exec_summary".to_string(), exec_summary_text);
            }
        },
        None => {
            let exec_summary_text = summarize_fn(
                all_summaries.as_str(),
                llm_params
            );
            raw_doc.generated_content.insert("exec_summary".to_string(), exec_summary_text);
        }
    }
    info!("{}: Completed processing document - '{}'.", PLUGIN_NAME, raw_doc.title);
}


#[cfg(test)]
mod tests {
    use config::Config;
    use log::debug;
    use crate::llm;
    use crate::plugins::{mod_summarize};

    #[test]
    fn test_run_worker_thread() {
        // TODO: implement this
        assert_eq!(1, 1);
    }


}
