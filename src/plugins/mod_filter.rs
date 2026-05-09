// file: mod_filter.rs
// Purpose: Data processing plugin - filter documents by doc_type classification.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use config::Config;
use log::{error, info};
use crate::document;

pub const PLUGIN_NAME: &str = "mod_filter";

/// Executes this function of the module in the separate thread launched by the pipeline to
/// process documents received on channel rx and,
/// transmit only the documents that pass the filter to tx.
///
/// Only documents where doc_type contains "speech" or "regulatory-notification" are forwarded.
/// All other documents are dropped.
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
pub fn process_data(tx: Sender<document::Document>, rx: Receiver<document::Document>, _app_config: &Config, _api_mutexes: &mut HashMap<String, Arc<Mutex<isize>>>) {
    info!("{}: Starting module 'filter'", PLUGIN_NAME);
    let mut doc_counter: u32 = 0;

    for doc in rx {
        // filtering based on doc_type
        let mut doc_type = "speech".to_string();
        match doc.classification.get("doc_type") {
            Some(mapped_value) => { doc_type = mapped_value.clone() },
            None => {}
        }

        if doc_type.contains("speech") || doc_type.contains("regulatory-notification") {
            match tx.send(doc) {
                Result::Ok(_) => { doc_counter += 1; },
                Err(e) => error!("{}: When sending processed doc via tx: {}", PLUGIN_NAME, e)
            }
        } else {
            info!("{}: Ignoring document type {} for - '{}'", PLUGIN_NAME, doc_type, doc.title);
        }
    }
    info!("{}: Processed {} documents in total", PLUGIN_NAME, doc_counter);
}


#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::mpsc;
    use config::Config;
    use crate::document::Document;
    use crate::plugins::mod_filter::process_data;

    #[test]
    fn test_filter_passes_speech() {
        let (tx_in, rx_in) = mpsc::channel::<Document>();
        let (tx_out, rx_out) = mpsc::channel::<Document>();

        let mut doc = Document::default();
        doc.title = "Test speech".to_string();
        doc.classification.insert("doc_type".to_string(), "speech".to_string());
        tx_in.send(doc).unwrap();
        drop(tx_in);

        let mut api_mutexes = HashMap::new();
        let cfg = Config::builder().build().unwrap();
        process_data(tx_out, rx_in, &cfg, &mut api_mutexes);

        let received = rx_out.try_recv();
        assert!(received.is_ok(), "Speech document should pass the filter");
    }

    #[test]
    fn test_filter_drops_market_action() {
        let (tx_in, rx_in) = mpsc::channel::<Document>();
        let (tx_out, rx_out) = mpsc::channel::<Document>();

        let mut doc = Document::default();
        doc.title = "Test market action".to_string();
        doc.classification.insert("doc_type".to_string(), "market_action".to_string());
        tx_in.send(doc).unwrap();
        drop(tx_in);

        let mut api_mutexes = HashMap::new();
        let cfg = Config::builder().build().unwrap();
        process_data(tx_out, rx_in, &cfg, &mut api_mutexes);

        let received = rx_out.try_recv();
        assert!(received.is_err(), "market_action document should be dropped by the filter");
    }

    #[test]
    fn test_filter_passes_regulatory_notification() {
        let (tx_in, rx_in) = mpsc::channel::<Document>();
        let (tx_out, rx_out) = mpsc::channel::<Document>();

        let mut doc = Document::default();
        doc.title = "Test regulatory notification".to_string();
        doc.classification.insert("doc_type".to_string(), "regulatory-notification".to_string());
        tx_in.send(doc).unwrap();
        drop(tx_in);

        let mut api_mutexes = HashMap::new();
        let cfg = Config::builder().build().unwrap();
        process_data(tx_out, rx_in, &cfg, &mut api_mutexes);

        let received = rx_out.try_recv();
        assert!(received.is_ok(), "regulatory-notification document should pass the filter");
    }
}
