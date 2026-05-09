// file: mod_metadata.rs
// Purpose: Data processing plugin - extract metadata from documents using LLM.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use config::Config;
use log::{debug, error, info};
use crate::{document, get_cfg};
use crate::llm::{invoke_llm_func_with_lock, prepare_llm_parameters, LLMParameters, MIN_ACCEPTABLE_SUMMARY_CHARS};

pub const PLUGIN_NAME: &str = "mod_metadata";

/// Executes this function of the module in the separate thread launched by the pipeline to
/// process documents received on channel rx, tag them with metadata using an LLM and,
/// transmit the updated documents to tx.
///
/// # Arguments
///
/// * `tx`: Queue transmitter for the next thread
/// * `rx`: Queue receiver for this thread
/// * `app_config`: The application's configuration object
/// * `api_mutexes`: Map of mutexes for rate-limiting API access
///
/// returns: ()
///
pub fn process_data(tx: Sender<document::Document>, rx: Receiver<document::Document>, app_config: &Config, api_mutexes: &mut HashMap<String, Arc<Mutex<isize>>>) {
    info!("{}: Starting module - Document metadata tagging.", PLUGIN_NAME);
    let mut doc_counter: u32 = 0;
    let prompt_template = get_cfg!("prompt_metadata", app_config, "Identify industry categories from this text. Return as String array in json format.\nTEXT:\n");
    let mut llm_params = prepare_llm_parameters(app_config, prompt_template.clone(), PLUGIN_NAME);
    let option_mutex = api_mutexes.get(llm_params.llm_service.as_str());

    for mut doc in rx {

        update_doc_with_metadata(&mut llm_params, &mut doc, prompt_template.as_str(), option_mutex);

        match tx.send(doc) {
            Result::Ok(_) => { doc_counter += 1; },
            Err(e) => error!("{}: When sending processed doc via tx: {}", PLUGIN_NAME, e)
        }
    }
    info!("{}: Completed processing a total of {} documents.", PLUGIN_NAME, doc_counter);
}

fn update_doc_with_metadata(llm_params: &mut LLMParameters, raw_doc: &mut document::Document, prompt_template: &str, llm_api_mutex_result: Option<&Arc<Mutex<isize>>>) {
    if raw_doc.text.len() < MIN_ACCEPTABLE_SUMMARY_CHARS {
        error!("{}: No content available to identify metadata for doc - '{}'",
            PLUGIN_NAME,
            raw_doc.title,
        );
        return;
    }
    let summarize_fn = llm_params.sumarize_fn;
    let metadata_prompt = format!(
        "{}\n{}\n",
        prompt_template,
        raw_doc.text
    );
    let max_extent_permitted = std::cmp::min(metadata_prompt.len() - 1, 62000);
    let metadata_prompt: String = metadata_prompt.chars().take(max_extent_permitted).collect();
    if let Some(llm_api_mutex) = llm_api_mutex_result {
        match raw_doc.generated_content.get("metadata") {
            Some(existing_exec_summary) => {
                if existing_exec_summary.to_string().len() < MIN_ACCEPTABLE_SUMMARY_CHARS {
                    debug!("{}: Extracting metadata using prompt:\n{}", PLUGIN_NAME, metadata_prompt);
                    let metadata_text = invoke_llm_func_with_lock(
                        llm_api_mutex,
                        metadata_prompt.as_str(),
                        llm_params,
                        summarize_fn
                    );
                    raw_doc.generated_content.insert("metadata".to_string(), metadata_text);
                }
            },
            None => {
                debug!("{}: Extracting metadata using prompt:\n{}", PLUGIN_NAME, metadata_prompt);
                let metadata_text = invoke_llm_func_with_lock(
                    llm_api_mutex,
                    metadata_prompt.as_str(),
                    llm_params,
                    summarize_fn
                );
                raw_doc.generated_content.insert("metadata".to_string(), metadata_text);
            }
        }
        info!("{}: Completed processing document - '{}'.", PLUGIN_NAME, raw_doc.title);
    }
}


#[cfg(test)]
mod tests {
    use config::Config;
    use crate::plugins::mod_metadata::process_data;

    #[test]
    fn test_metadata_plugin_compiles() {
        // Verify the plugin can be imported and the function signature is correct.
        // This test validates compilation only - full LLM test requires a live service.
        assert!(true);
    }
}
