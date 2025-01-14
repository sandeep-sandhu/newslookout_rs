// file: mod_dedupe.rs

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use config::Config;
use log::{debug, error, info};
use crate::document::Document;
use crate::utils::{clean_text, get_text_from_element, to_local_datetime};

pub(crate) const PLUGIN_NAME: &str = "mod_dedupe";
const PUBLISHER_NAME: &str = "Data De-duplication";


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
pub(crate) fn process_data(tx: Sender<Document>, rx: Receiver<Document>, config: &Config, api_mutexes: &mut HashMap<String, Arc<Mutex<isize>>>){

    info!("{}: Getting configuration.", PLUGIN_NAME);

    for doc in rx {
        info!("{}: Started processing document titled - {}", PLUGIN_NAME, doc.title);
        let updated_doc:Document = update_doc(doc);
        match tx.send(updated_doc) {
            Result::Ok(_) => {},
            Err(e) => error!("{}: When sending processed doc via tx: {}", PLUGIN_NAME, e)
        }
    }

    info!("{}: Completed processing.", PLUGIN_NAME);
}

fn update_doc(raw_doc: Document) -> Document{
    info!("{}: processing document titled - '{}'", PLUGIN_NAME, raw_doc.title);

    // TODO: implement this

    return raw_doc;
}