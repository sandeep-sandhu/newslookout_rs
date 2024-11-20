use std::collections::HashMap;
use std::error::Error;
use std::sync::mpsc::{Receiver, Sender};
use config::Config;
use log::{debug, error, info};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, InvalidHeaderName};
use serde::{Deserialize, Serialize};
use crate::document;
use crate::llm::{generate_using_chatgpt_svc, http_post_json_chatgpt, prepare_chatgpt_headers, prepare_chatgpt_payload, update_doc};
use crate::network::build_llm_api_client;
use crate::utils::get_plugin_config;

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
    // set a low connect timeout:
    let connect_timeout: u64 = 15;

    // prepare the http client for the REST service
    let mut custom_headers = prepare_chatgpt_headers(app_config);
    let api_client = build_llm_api_client(connect_timeout, fetch_timeout, None, Some(custom_headers));

    // process each document received and return back to next handler:
    for doc in rx {
        info!("{}: Started processing document titled - '{}',  with #{} parts.",
            PLUGIN_NAME, doc.title, doc.text_parts.len()
        );

        let updated_doc:document::Document = update_doc(
            &api_client,
            doc,
            PLUGIN_NAME,
            &app_config,
            generate_using_chatgpt_svc
        );

        //for each document received in channel queue, send it to next queue:
        match tx.send(updated_doc) {
            Result::Ok(_) => {doc_counter += 1;},
            Err(e) => error!("{}: When sending processed doc via tx: {}", PLUGIN_NAME, e)
        }
    }
    info!("{}: Completed processing {} documents.", PLUGIN_NAME, doc_counter);
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
