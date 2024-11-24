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

// use faiss::{Index, index_factory, MetricType};
// pub fn faiss_demo(){
//     let my_data = [
//         [0.0, 1.0, 10.0, 0.0, 2.0, -1.0, 1.0, 12.0],
//         [0.0, 0.0, -1.0, 1.0, 6.0, 0.0, 0.0, 0.2],
//         [1.0, 1.0, 3.0, 1.0, 6.0, 0.0, 1.0, 0.3],
//         [1.0, 0.0, -3.0, 0.0, 2.0, -1.0, 0.0, 12.0],
//     ];
//     let my_query = [5.0, 5.0, 10.0, 0.0, 2.0, -0.3, 0.0, 7.0];
//
//     let mut index = index_factory(8, "Flat", MetricType::L2).unwrap();
//     index.add(&my_data[0]).unwrap();
//     index.add(&my_data[1]).unwrap();
//     index.add(&my_data[2]).unwrap();
//     index.add(&my_data[3]).unwrap();
//
//     let result = index.search(&my_query, 2).unwrap();
//     println!("{:?}", result);
// }
