use std::sync::mpsc::Sender;

const PLUGIN_NAME: &str = "mod_generic_retriever";

pub(crate) fn run_worker_thread(tx: Sender<document::Document>, app_config: Config) {

    let mut urls: Vec<String> = vec![];

    retrieve_docs_from_url(tx, urls, &app_config);

}

fn retrieve_docs_from_url(tx: Sender<document::Document>, urls: Vec<String>, app_config: &Config){
    // TODO: implement this

}

