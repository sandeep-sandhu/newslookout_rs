use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use config::Config;
use log::{debug, error, info};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;
use crate::document;
use crate::get_plugin_cfg;

pub const PLUGIN_NAME: &str = "mod_persist_data";


pub(crate) fn process_data(tx: Sender<document::Document>, rx: Receiver<document::Document>, app_config: &Config, api_mutexes: &mut HashMap<String, Arc<Mutex<isize>>>){

    info!("{}: Getting configuration specific to the module.", PLUGIN_NAME);
    let mut counter: usize = 0;

    // write file to the folder specified in config file, e.g.: data_folder_name=/var/cache
    let mut data_folder_name = String::from("");
    match app_config.get_string("data_dir") {
        Ok(dirname) => data_folder_name = dirname,
        Err(e) => error!("When getting name of data folder to save, error: {}, using default value.", e)
    }

    // read parameter: "destination"="file"/ "database" / "blob"
    let mut destination: String = String::from("file");
    match get_plugin_cfg!(PLUGIN_NAME, "destination", &app_config) {
        Some(param_val_str) => destination = param_val_str,
        None => error!("Could not get parameter 'destination', using default value of: {}", destination)
    };

    for doc in rx {
        info!("{}: Started persisting document titled - '{}'.", PLUGIN_NAME, doc.title);
        match destination.as_str() {
            "file" => {
                let updated_doc = save_to_zip(doc, data_folder_name.as_str());
                match tx.send(updated_doc) {
                    Result::Ok(_) => { counter += 1; },
                    Err(e) => error!("{}: When sending processed doc via tx: {}", PLUGIN_NAME, e)
                }
            },
            "database" => {
                debug!("Writing document to database.");
                let updated_doc = write_to_database(doc, &app_config);
                match tx.send(updated_doc) {
                    Result::Ok(_) => { counter += 1; },
                    Err(e) => error!("{}: When sending processed doc via tx: {}", PLUGIN_NAME, e)
                }
            },
            _ => {
                error!("Unknown destination '{}' specified in config for this plugin, DATA WAS NOT PERSISTED!", destination);
                match tx.send(doc) {
                    Result::Ok(_) => {},
                    Err(e) => error!("{}: When sending processed doc via tx: {}", PLUGIN_NAME, e)
                }
            }
        }
    }
    info!("{}: Completed persisting {} documents to {}.", PLUGIN_NAME, counter, destination);
}

/// Generates the JSON entry filename: {module}_{unique_id}_{url_hash}.json
/// Always includes a URL hash suffix so entries are globally unique even when
/// two articles share the same unique_id (e.g. same URL slug from different pages).
fn make_json_entry_name(doc: &document::Document) -> String {
    let mut hasher = std::hash::DefaultHasher::new();
    doc.url.hash(&mut hasher);
    let url_hash = hasher.finish();
    if !doc.unique_id.is_empty() {
        format!("{}_{}_{:x}.json", doc.module, doc.unique_id, url_hash)
    } else {
        format!("{}_{:x}.json", doc.module, url_hash)
    }
}

/// Saves the document as a JSON entry inside a dated zip archive.
/// Archive: {data_folder}/{publish_date}.zip  (e.g. 2024-05-09.zip)
/// Entry  : {module}_{unique_id}.json          (flat, no subdirectory)
fn save_to_zip(mut received: document::Document, data_folder_name: &str) -> document::Document {
    let entry_name = make_json_entry_name(&received);
    let zip_name = format!("{}.zip", received.publish_date);
    let zip_path = Path::new(data_folder_name).join(&zip_name);

    // Record path as "YYYY-MM-DD.zip/entry.json" so the DB knows where to look.
    received.filename = format!("{}/{}", zip_name, entry_name);
    info!("Writing document '{}' to zip entry: {}", received.title, received.filename);

    let json_data = match serde_json::to_string_pretty(&received.to_output_json()) {
        Ok(data) => data,
        Err(e) => {
            error!("When serialising document to JSON: {}", e);
            return received;
        }
    };

    let zip_exists = zip_path.exists();
    let file = if zip_exists {
        match OpenOptions::new().read(true).write(true).open(&zip_path) {
            Ok(f) => f,
            Err(e) => {
                error!("When opening zip file {} for append: {}", zip_name, e);
                return received;
            }
        }
    } else {
        match OpenOptions::new().read(true).write(true).create(true).open(&zip_path) {
            Ok(f) => f,
            Err(e) => {
                error!("When creating zip file {}: {}", zip_name, e);
                return received;
            }
        }
    };

    let mut zip: ZipWriter<File> = if zip_exists {
        match ZipWriter::new_append(file) {
            Ok(z) => z,
            Err(e) => {
                error!("When opening zip {} for append: {}", zip_name, e);
                return received;
            }
        }
    } else {
        ZipWriter::new(file)
    };

    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    match zip.start_file(&entry_name, options) {
        Ok(_) => {},
        Err(e) => {
            error!("When starting zip entry {}: {}", entry_name, e);
            return received;
        }
    }

    match zip.write_all(json_data.as_bytes()) {
        Ok(_) => {
            debug!("Wrote '{}' to {}", entry_name, zip_name);
        },
        Err(e) => error!("When writing to zip entry {}: {}", entry_name, e)
    }

    match zip.finish() {
        Ok(_) => {},
        Err(e) => error!("When finalising zip {}: {}", zip_name, e)
    }

    received
}

fn write_to_database(doc: document::Document, _app_config: &Config) -> document::Document {
    // TODO: implement this
    doc
}
