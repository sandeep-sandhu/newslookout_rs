// file: mod_classify.rs

use log::{error, warn, info, debug};

use crate::{document, network};
use crate::document::Document;
use crate::utils::{clean_text, get_text_from_element, to_local_datetime};

pub(crate) const PLUGIN_NAME: &str = "mod_classify";
const PUBLISHER_NAME: &str = "Data Classification";

pub(crate) fn process_data(doc: &document::Document, config: &config::Config){
    info!("{}: Getting configuration.", PLUGIN_NAME);

    // Print the configuration options:
    debug!("Loading models from models_dir = {:?}", config.get_string("models_dir"));
    // TODO: implement this

    info!("{}: Processed document: {}", PLUGIN_NAME, doc.title);
}

#[cfg(test)]
mod tests {
    use crate::plugins::mod_classify;

    #[test]
    fn test_run_worker_thread() {
        // TODO: implement this
        assert_eq!(1, 1);
    }
}