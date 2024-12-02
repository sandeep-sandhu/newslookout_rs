// file: mod_solrsubmit.rs

use std::sync::mpsc::{Receiver, Sender};
use config::Config;
use log::{error, info};
use crate::{document, network};
use crate::utils::{clean_text, get_text_from_element, to_local_datetime};

pub(crate) const PLUGIN_NAME: &str = "mod_solrsubmit";
const PUBLISHER_NAME: &str = "Index via SOLR Service";

/// Process documents received on channel rx and,
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
pub(crate) fn process_data(tx: Sender<document::Document>, rx: Receiver<document::Document>, config: &Config){

    info!("{}: Getting configuration for {}", PLUGIN_NAME, PUBLISHER_NAME);

    for doc in rx {
        info!("Saving processed document titled - {}", doc.title);
        let updated_doc:document::Document = update_doc(doc);
        match tx.send(updated_doc) {
            Result::Ok(_) => {},
            Err(e) => error!("{}: When sending processed doc via tx: {}", PLUGIN_NAME, e)
        }
    }

    info!("{}: Completed processing.", PLUGIN_NAME);
}

fn update_doc(raw_doc: document::Document) -> document::Document{
    info!("{}: updating document titled - '{}'", PLUGIN_NAME, raw_doc.title);

    // TODO: implement this

    return raw_doc;
}

#[cfg(test)]
mod tests {
    use crate::plugins::mod_solrsubmit;

    #[test]
    fn test_run_worker_thread() {
        // TODO: implement this
        assert_eq!(1, 1);
    }
}
