// mod_cmdline

const PLUGIN_NAME: &str = "mod_cmdline";
pub const PUBLISHER_NAME: &str = "Command Line";

use std::process::Command;
use std::sync::mpsc::{Receiver, Sender};
use config::Config;
use log::{error, info};
use rusqlite::Params;
use crate::{document, get_plugin_cfg};

pub(crate) fn process_data(tx: Sender<document::Document>, rx: Receiver<document::Document>, app_config: &Config){


    let mut command_name = String::from("cmd");
    match get_plugin_cfg!(PLUGIN_NAME, "command_name", app_config) {
        Some(param_val_str) => command_name = param_val_str,
        None => error!(
            "Error getting command_name from config of plugin {}, using default value: {}",
            PLUGIN_NAME,
            command_name
        )
    }
    info!("{}: Getting configuration, command to execute: {}", PLUGIN_NAME, command_name);

    for doc in rx {

        info!("{}: Started processing document titled - {}", PLUGIN_NAME, doc.title);

        // pass doc.filename as argument
        match Command::new(command_name.as_str())
            .arg(doc.filename.as_str())
            .output() {
            Ok(output) => {
                info!("Command execution output: {}", String::from_utf8_lossy(&output.stdout));
            },
            Err(e) => {
                error!("When executing process {} with arg: {}: {}", command_name, doc.filename, e);
            }
        }

        match tx.send(doc) {
            Result::Ok(_) => {},
            Err(e) => error!("{}: When transmitting doc: {}", PLUGIN_NAME, e)
        }

    }

    info!("{}: Completed processing all data.", PLUGIN_NAME);
}
