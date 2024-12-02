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
use crate::{document, get_cfg, llm};
use crate::llm::{prepare_llm_parameters, LLMParameters, MAX_TOKENS, MIN_ACCEPTABLE_SUMMARY_CHARS, TOKENS_PER_WORD};
use crate::network::build_llm_api_client;
use crate::utils::{word_count};
use crate::cfg::{get_plugin_config};

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

    let mut prompt_summary_part = get_cfg!("prompt_summary_part", app_config, "Summarize this text:\n");

    let mut prompt_summary_exec = get_cfg!("prompt_summary_exec", app_config, "Summarize this text:\n");

    let mut llm_params = prepare_llm_parameters(app_config, prompt_summary_part.clone(), PLUGIN_NAME);

    // process each document received and return back to next handler:
    for mut doc in rx {

        update_doc(&mut llm_params, &mut doc, prompt_summary_part.as_str(), prompt_summary_exec.as_str());

        //for each document received in channel queue, send it to next queue:
        match tx.send(doc) {
            Result::Ok(_) => {doc_counter += 1;},
            Err(e) => error!("{}: When sending processed doc via tx: {}", PLUGIN_NAME, e)
        }
    }
    info!("{}: Completed processing {} documents.", PLUGIN_NAME, doc_counter);
}


/// Generate the summary of this document's text.
/// If the document size (measured in tokens of the LLM's tokenizer) is less than the maximum
/// permissible, then the entire summary is generated in one go.
/// Or else, it is broken into parts and summarised by parts first before generating the executive
/// summary from these summaries of parts.
///
/// # Arguments
///
/// * `llm_params`:
/// * `raw_doc`:
/// * `prompt_part`:
/// * `prompt_exec_summary`:
///
/// returns: ()
fn update_doc(llm_params: &mut LLMParameters, raw_doc: &mut document::Document, prompt_part: &str, prompt_exec_summary: &str) {

    let summarize_fn = llm_params.sumarize_fn;

    let mut all_summaries: String = String::new();

    // prepare for executive summary:
    let exec_summ_prompt = format!(
        "{}\n{}\nPublish Date: {}",
        prompt_exec_summary,
        raw_doc.title,
        raw_doc.publish_date
    );

    // check how long is the text + prompt in word counts and tokens,
    let full_text_tokens = TOKENS_PER_WORD * ((word_count(exec_summ_prompt.as_str()) + word_count(raw_doc.text.as_str()) ) as f64) ;

    // compare with = llm_params.num_context,
    // break up and process only if longer than max context:
    if full_text_tokens > (llm_params.num_context as f64) {

        info!("{}: Summarizing in {} parts, document - '{}'",
            PLUGIN_NAME, raw_doc.text_parts.len(), raw_doc.title
        );
        llm_params.prompt = prompt_part.to_string();
        let mut part_no = raw_doc.text_parts.len();

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
                                    info!("Not re-generating summary for part #{}", part_no);
                                    // do not proceed further to generate summary:
                                    continue;
                                } else {
                                    info!("Although overwrite={}, existing summary is inadequate, so re-generating it for part #{}",
                                        llm_params.overwrite_existing_value,
                                        part_no);
                                }
                            }
                            _ => {}
                        }
                    }

                    // else, compute the summary for this part:
                    match text_part.get("text"){
                        Some(text_string) => {
                            let this_parts_text_length = text_string.to_string().len();
                            if this_parts_text_length > MIN_ACCEPTABLE_SUMMARY_CHARS {
                                info!("Summarizing part #{}", part_no);
                                let mut text_part_map_clone = text_part.clone();
                                let summary_text = summarize_fn(
                                    text_string.to_string().as_str(),
                                    llm_params
                                );
                                _ = text_part_map_clone.insert("summary".to_string(), Value::String(summary_text.clone()));
                                updated_text_parts.push(text_part_map_clone);
                                all_summaries.push_str("\n");
                                all_summaries.push_str(summary_text.as_str());
                            }
                            else {
                                error!("Inadequate quantity of text to summarize (length = {}) for part #{}", this_parts_text_length, part_no);
                            }
                        },
                        None => {}
                    }
                }
            }
            part_no = part_no - 1;
        }
        // put updated text_parts into document:
        updated_text_parts.reverse();
        raw_doc.text_parts = updated_text_parts;
    }
    else {
        // use original doc for context:
        info!("{}: Summarizing entire document at once - '{}'",
            PLUGIN_NAME, raw_doc.title
        );
        all_summaries = raw_doc.text.clone();
    }

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
                info!("Generating executive summary (in {} parts)",
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
