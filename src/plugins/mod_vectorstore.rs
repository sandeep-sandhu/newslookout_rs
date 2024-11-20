// file: mod_vectorstore.rs
// purpose: add retrieved document to vectorstore
// chunk text, use sentence tokenizer, save to vectorstore.


use std::sync::mpsc::{Receiver, Sender};
use config::Config;
use log::{debug, error, info};
use crate::document::Document;

pub const PLUGIN_NAME: &str = "mod_vectorstore";

pub fn process_data(tx: Sender<Document>, rx: Receiver<Document>, config: &Config){

    info!("{}: Getting configuration.", PLUGIN_NAME);

    for doc in rx {
        info!("{}: Started processing document titled - {}", PLUGIN_NAME, doc.title);
        let updated_doc:Document = update_doc(doc);
        match tx.send(updated_doc) {
            Result::Ok(_) => {},
            Err(e) => error!("{}: When sending processed doc via tx: {}", PLUGIN_NAME, e)
        }
    }

    info!("{}: Completed processing all data.", PLUGIN_NAME);
}

fn update_doc(raw_doc: Document) -> Document{
    info!("{}: updating document titled - '{}'", PLUGIN_NAME, raw_doc.title);

    // TODO: implement this

    return raw_doc;
}

