use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender};
use config::Config;
use log::{debug, error, info};
use crate::document;
use crate::utils::{make_unique_filename};
use crate::get_plugin_cfg;

pub const PLUGIN_NAME: &str = "mod_persist_data";


pub(crate) fn process_data(tx: Sender<document::Document>, rx: Receiver<document::Document>, app_config: &Config){

    info!("{}: Getting configuration specific to the module.", PLUGIN_NAME);
    let mut counter: usize = 0;

    // write file to the folder specified in config file, e.g.: data_folder_name=/var/cache
    let mut data_folder_name = String::from("");
    match app_config.get_string("data_dir") {
        Ok(dirname) => data_folder_name = dirname,
        Err(e) => error!("When getting name of data folder to save, error: {}, using default value.", e)
    }

    // read parameter: "file_format"
    let mut file_format: String = String::from("json");
    match get_plugin_cfg!(PLUGIN_NAME, "file_format", &app_config) {
        Some(param_val_str) => file_format = param_val_str,
        None => error!("Could not get parameter 'file_format', using default value of: {}", file_format)
    };

    // read parameter: "destination"="file"/ "database"
    let mut destination: String = String::from("file");
    match get_plugin_cfg!(PLUGIN_NAME, "destination", &app_config) {
        Some(param_val_str) => destination = param_val_str,
        None => error!("Could not get parameter 'destination', using default value of: {}", destination)
    };

    // process each document received and return back to next handler:
    for doc in rx {
        info!("{}: Started persisting document titled - '{}'.", PLUGIN_NAME, doc.title);
        match destination.as_str() {
            "file" => {
                let updated_doc: document::Document = save_to_file(
                    doc,
                    data_folder_name.as_str(),
                    file_format.as_str()
                );
                //transmit each updated document to next processor via the channel
                match tx.send(updated_doc) {
                    Result::Ok(_) => {counter += 1;},
                    Err(e) => error!("{}: When sending processed doc via tx: {}", PLUGIN_NAME, e)
                }
            },
            "database" => {
                debug!("Writing document to database.");
                let updated_doc: document::Document = write_to_database(
                    doc,
                    &app_config
                );
                //for each document received via rx channel, to next processor via the channel
                match tx.send(updated_doc) {
                    Result::Ok(_) => {counter += 1;},
                    Err(e) => error!("{}: When sending processed doc via tx: {}", PLUGIN_NAME, e)
                }
            }
            _ => {
                error!("Unknown destination '{}' specified in config for this plugin, DATA WAS NOT PERSISTED!", destination);
                //for each document received via rx channel, transmit it to next processor via the channel
                match tx.send(doc) {
                    Result::Ok(_) => {},
                    Err(e) => error!("{}: When sending processed doc via tx: {}", PLUGIN_NAME, e)
                }
            }
        }
        //for each document received via rx channel, transmit it to next processor via the channel
    }
    info!("{}: Completed persisting {} documents to {}.", PLUGIN_NAME, counter, destination);
}

fn save_to_file(
    mut received: document::Document,
    data_folder_name : &str,
    file_format: &str
) -> document::Document{
    // save the file:
    match file_format{
        "json" => {
            // create filename: json_file_path
            let json_file_path = Path::new(data_folder_name).join(make_unique_filename(&received, "json"));
            received.filename = String::from(json_file_path.as_path().to_str().expect("Not able to convert path to string"));

            info!("Writing document to file: {} url: {:?}", received.filename, received.url);
            // serialize json to string
            match serde_json::to_string_pretty(&received){
                Ok(json_data) => {
                    // persist to json
                    match File::create(&json_file_path){
                        Ok(mut file) => {
                            match file.write_all(json_data.as_bytes()) {
                                Ok(_write_res) => {
                                    debug!("Wrote document from {}, titled '{}' to file: {:?}", received.url, received.title, json_file_path);
                                },
                                Err(write_err) => error!("When writing file to disk: {}", write_err)
                            }
                        },
                        Err(file_err)=> {
                            error!("When writing document to json file: {}", file_err);
                        }
                    }
                },
                Err(serderr) => error!("When serialising document to JSON text: {}", serderr)
            }
            return received;
        },
        _ => {
            error!("Not saving the document! Unknown file format specified in config.");
            return received;
        }
    }
}

fn write_to_database(doc: document::Document, app_config : &Config) -> document::Document{
    // TODO: implement this
    return doc;
}
