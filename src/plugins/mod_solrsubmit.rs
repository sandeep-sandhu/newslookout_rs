// file: mod_solrsubmit.rs

use log::info;
use crate::{document, network};
use crate::document::Document;
use crate::utils::{clean_text, get_text_from_element, to_local_datetime};

pub(crate) const PLUGIN_NAME: &str = "mod_solrsubmit";
const PUBLISHER_NAME: &str = "Index via SOLR Service";

pub(crate) fn process_data(doc: &document::Document, config: &config::Config){
    info!("Starting processing data using plugin: {}", PLUGIN_NAME);

    // Print the configuration options:
    info!("models_dir = {:?}", config.get_string("models_dir"));

    info!("Processed document: {}", doc.title);

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
