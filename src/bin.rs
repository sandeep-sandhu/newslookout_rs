// Purpose: Application
// Description: Starts the app.
// libraries:
use std::env;
use rusty_tesseract;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender};
use newslookout::{cleanup_pid_file, init_logging, init_pid_file, get_cfg, get_plugin_cfg, document};
use log::{debug, error, info};
use newslookout::document::Document;
use newslookout::pipeline;
use newslookout::llm::{invoke_llm_func_with_lock, prepare_llm_parameters, LLMParameters, MAX_TOKENS, MIN_ACCEPTABLE_SUMMARY_CHARS, TOKENS_PER_WORD};
use std::sync::{Arc, Mutex};
use chrono::{NaiveDate, TimeZone, Utc};
use newslookout::pipeline::{create_api_mutexes, load_dataproc_plugins, load_retriever_plugins, DataProcPlugin, RetrieverPlugin};
use std::panic;
use std::panic::AssertUnwindSafe;
use newslookout::utils::{get_text_using_ocr, word_count, get_urls_from_database, make_unique_filename, load_pdf_content, check_and_fix_url, clean_text, to_local_datetime, get_text_from_element};
use config::Config;
use newslookout::cfg::read_config_from_file;
use newslookout::network::{http_get, make_http_client, read_network_parameters, NetworkParameters};
use newslookout::web_api::{create_status_tracker, start_web_api};
use rand::{Rng, RngExt};
use regex::Regex;
use reqwest::blocking::Client;
use scraper::ElementRef;
use serde_json::Value;

// ---

fn main() {

    // Install a panic hook so all panics print a clear message to stdout (visible even without log).
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() { s.to_string() }
                  else if let Some(s) = info.payload().downcast_ref::<String>() { s.clone() }
                  else { "unknown panic".to_string() };
        let location = info.location().map(|l| format!("{}:{}", l.file(), l.line()))
                           .unwrap_or_else(|| "unknown location".to_string());
        println!("FATAL ERROR: {} (at {})", msg, location);
        eprintln!("FATAL ERROR: {} (at {})", msg, location);
        default_hook(info);
    }));

    if env::args().len() < 2 {
        println!("Usage: newslookout_rs <config_file>");
        std::process::exit(1);
    }

    let now = match env::var("SOURCE_DATE_EPOCH") {
        Ok(val) => { Utc.timestamp_opt(val.parse::<i64>().unwrap(), 0).unwrap() }
        Err(_) => Utc::now(),
    };
    println!("NewsLookout, version: {}", now);

    run_pipeline();
}



fn run_pipeline(){

    let config_file: String = env::args().nth(1).unwrap();
    println!("Loading configuration from file: {}", config_file);

    let config = read_config_from_file(config_file);
    let configref = Arc::new(config);
    println!("Initializing PID file...");
    init_pid_file(configref.clone());
    println!("Initializing logging...");
    init_logging(configref.clone());

    let rl_model_path = configref.get_string("rl_model_path").ok();
    info!("Initializing content extractor (model path: {:?})...", rl_model_path);
    let extractor_init_result = panic::catch_unwind(AssertUnwindSafe(|| {
        newslookout::content_extraction::init_html_extractor(rl_model_path.as_deref());
    }));
    if let Err(e) = extractor_init_result {
        let msg = if let Some(s) = e.downcast_ref::<&str>() { s.to_string() }
                  else if let Some(s) = e.downcast_ref::<String>() { s.clone() }
                  else { "unknown error".to_string() };
        error!("Content extractor initialization failed (will use CSS fallback): {}", msg);
        println!("WARNING: Content extractor initialization failed: {}. Using CSS fallback.", msg);
    }
    let all_api_mutexes: HashMap<String, Arc<Mutex<isize>>> = create_api_mutexes();
    info!("Starting the data pipeline");

    let mut retriever_plugins = load_retriever_plugins(configref.clone());
    let mut data_proc_plugins = load_dataproc_plugins(configref.clone(), all_api_mutexes.clone());

    // rbi data retriever:
    let rbi_enabled = get_plugin_cfg!("rbi_new", "enabled", configref.clone()).unwrap_or_else(|| String::from("false")).parse::<bool>().unwrap();
    let rbi_plugin = RetrieverPlugin{
        name: "rbi_new".to_string(),
        priority: 1,
        enabled: rbi_enabled,
        method: run_rbi_scanner,
    };
    retriever_plugins.push(rbi_plugin);

    // sebi data retriever:
    let sebi_enabled = get_plugin_cfg!("sebi", "enabled", configref.clone()).unwrap_or_else(|| String::from("false")).parse::<bool>().unwrap();
    let sebi_plugin = RetrieverPlugin{
        name: "sebi".to_string(),
        priority: 1,
        enabled: sebi_enabled,
        method: run_sebi_scanner,
    };
    retriever_plugins.push(sebi_plugin);

    // irdai data retriever:
    let irdai_enabled = get_plugin_cfg!("irdai", "enabled", configref.clone()).unwrap_or_else(|| String::from("false")).parse::<bool>().unwrap();
    let irdai_plugin = RetrieverPlugin{
        name: "irdai".to_string(),
        priority: 1,
        enabled: irdai_enabled,
        method: run_irdai_scanner,
    };
    retriever_plugins.push(irdai_plugin);

    // doc_type classification plugin:
    let doc_type_priority = get_plugin_cfg!("doc_type", "priority", configref.clone()).unwrap_or_else(|| String::from("1")).parse::<isize>().unwrap();
    let doc_type_enabled = get_plugin_cfg!("doc_type", "enabled", configref.clone()).unwrap_or_else(|| String::from("false")).parse::<bool>().unwrap();
    let doc_type_processing = DataProcPlugin{
        name: "doc_type".to_string(),
        priority: doc_type_priority,
        enabled: doc_type_enabled,
        api_mutexes: all_api_mutexes.clone(),
        method: run_document_classifier,
    };
    data_proc_plugins.push(doc_type_processing);

    // filter plugin
    let filter_priority = get_plugin_cfg!("filter", "priority", configref.clone()).unwrap_or_else(|| String::from("2")).parse::<isize>().unwrap();
    let filter_enabled = get_plugin_cfg!("filter", "enabled", configref.clone()).unwrap_or_else(|| String::from("false")).parse::<bool>().unwrap();
    let filter_processing = DataProcPlugin{
        name: "filter".to_string(),
        priority: filter_priority,
        enabled: filter_enabled,
        api_mutexes: all_api_mutexes.clone(),
        method: run_filter,
    };
    data_proc_plugins.push(filter_processing);

    // metadata plugin: metadata
    let metadata_priority = get_plugin_cfg!("metadata", "priority", configref.clone()).unwrap_or_else(|| String::from("3")).parse::<isize>().unwrap();
    let metadata_enabled = get_plugin_cfg!("metadata", "enabled", configref.clone()).unwrap_or_else(|| String::from("false")).parse::<bool>().unwrap();
    let metadata_processing = DataProcPlugin{
        name: "metadata".to_string(),
        priority: metadata_priority,
        enabled: metadata_enabled,
        api_mutexes: all_api_mutexes.clone(),
        method: run_metadata_tagger,
    };
    data_proc_plugins.push(metadata_processing);


    // // summarize plugin
    let summarize_priority = get_plugin_cfg!("summarize", "priority", configref.clone()).unwrap_or_else(|| String::from("3")).parse::<isize>().unwrap();
    let summarize_enabled = get_plugin_cfg!("summarize", "enabled", configref.clone()).unwrap_or_else(||String::from("false")).parse::<bool>().unwrap();
    let summary_processing = DataProcPlugin{
        name: "summarize".to_string(),
        priority: summarize_priority,
        enabled: summarize_enabled,
        api_mutexes: all_api_mutexes.clone(),
        method: run_document_summarizer,
    };
    data_proc_plugins.push(summary_processing);

    // insights plugin
    let insights_enabled = get_plugin_cfg!("insights", "enabled", configref.clone()).unwrap_or_else(||String::from("false")).parse::<bool>().unwrap();
    let insights_priority = get_plugin_cfg!("insights", "priority", configref.clone()).unwrap_or_else(|| String::from("5")).parse::<isize>().unwrap();
    let insights_processing = DataProcPlugin{
        name: "insights".to_string(),
        priority: insights_priority,
        enabled: insights_enabled,
        api_mutexes: all_api_mutexes.clone(),
        method: run_insights_extraction,
    };
    data_proc_plugins.push(insights_processing);

    // change detection plugin:
    let changes_enabled = get_plugin_cfg!("changes_from_previous", "enabled", configref.clone()).unwrap_or_else(||String::from("false")).parse::<bool>().unwrap();
    let changes_priority = get_plugin_cfg!("changes_from_previous", "priority", configref.clone()).unwrap_or_else(|| String::from("6")).parse::<isize>().unwrap();
    let changes_from_previous = DataProcPlugin{
        name: "changes_from_previous".to_string(),
        priority: changes_priority,
        enabled: changes_enabled,
        api_mutexes: all_api_mutexes.clone(),
        method: run_changes_extraction,
    };
    data_proc_plugins.push(changes_from_previous);

    info!("Loaded {} retriever and {} processing plugins to run.", retriever_plugins.len(), data_proc_plugins.len());

    // Start web API status server if enabled in config
    let status_tracker = create_status_tracker();
    let web_api_enabled = configref.get_bool("web_api_enabled").unwrap_or(false);
    if web_api_enabled {
        let host = configref.get_string("web_api_host").unwrap_or_else(|_| "127.0.0.1".to_string());
        let port = configref.get_int("web_api_port").unwrap_or(8080) as u16;
        start_web_api(&host, port, status_tracker.clone());
    }

    let docs_retrieved = pipeline::start_data_pipeline(
        retriever_plugins,
        data_proc_plugins,
        configref.clone(),
        Some(status_tracker),
    );

    // use this collection of retrieved documents information for any further custom processing -> docs_retrieved

    info!("Data pipeline completed processing {} documents.", docs_retrieved.len());
    cleanup_pid_file(configref);
}


// document filtering
fn run_filter(tx: Sender<Document>, rx: Receiver<Document>, _app_config: &Config, api_mutexes: &mut HashMap<String, Arc<Mutex<isize>>>){
    info!("Starting module 'filter'");
    let mut doc_counter: u32 = 0;

    for doc in rx {
        // filtering based on doc_type

        let mut doc_type = "speech".to_string();
        match doc.classification.get("doc_type") {
            Some(mapped_value) => { doc_type = mapped_value.clone() },
            None => {}
        }

        if doc_type.contains("speech") || doc_type.contains("regulatory-notification") {
            match tx.send(doc) {
                Result::Ok(_) => { doc_counter += 1; },
                Err(e) => error!("filter: When sending processed doc via tx: {}", e)
            }
        }else {
            info!("filter: Ignoring document type {} for - '{}'", doc_type, doc.title);
        }
    }
    info!("filter: Processed {} documents in total", doc_counter);
}

// document metadata tagging:
fn run_metadata_tagger(tx: Sender<Document>, rx: Receiver<Document>, app_config: &Config, api_mutexes: &mut HashMap<String, Arc<Mutex<isize>>>) {
    info!("Starting module - Document metadata tagging.");
    let mut doc_counter: u32 = 0;
    let prompt_template = get_cfg!("prompt_metadata", app_config, "Identify industry categories from this text. Return as String array in json format.\nTEXT:\n");
    let mut llm_params = prepare_llm_parameters(app_config, prompt_template.clone(), "doc_type");
    let mut option_mutex = api_mutexes.get(llm_params.llm_service.as_str());

    for mut doc in rx {

        update_doc_with_metadata(&mut llm_params, &mut doc, prompt_template.as_str(), option_mutex);

        match tx.send(doc) {
            Result::Ok(_) => { doc_counter += 1; },
            Err(e) => error!("metadata_tagger: When sending processed doc via tx: {}", e)
        }

    }
    info!("metadata_tagger: Completed processing a total of {} documents.", doc_counter);
}

fn update_doc_with_metadata(llm_params: &mut LLMParameters, raw_doc: &mut document::Document, prompt_template: &str, llm_api_mutex_result: Option<&Arc<Mutex<isize>>>) {
    const PLUGIN_NAME: &str = "metadata_tagger";
    if raw_doc.text.len() < MIN_ACCEPTABLE_SUMMARY_CHARS {
        error!("{}: No content available to identify metadata for doc - '{}'",
            PLUGIN_NAME,
            raw_doc.title,
        );
        return;
    }
    let summarize_fn = llm_params.sumarize_fn;
    let metadata_prompt = format!(
        "{}\n{}\n",
        prompt_template,
        raw_doc.text
    );
    let max_extent_permitted = std::cmp::min(metadata_prompt.len()-1, 62000);
    let metadata_prompt: String = metadata_prompt.chars().take(max_extent_permitted).collect();
    if let Some(mut llm_api_mutex) = llm_api_mutex_result {
        match raw_doc.generated_content.get("metadata") {
            Some(existing_exec_summary) => {
                if existing_exec_summary.to_string().len() < MIN_ACCEPTABLE_SUMMARY_CHARS {
                    debug!("{}: Extracting metadata using prompt:\n{}", PLUGIN_NAME, metadata_prompt);
                    let metadata_text = invoke_llm_func_with_lock(
                        llm_api_mutex,
                        metadata_prompt.as_str(),
                        llm_params,
                        summarize_fn
                    );
                    raw_doc.generated_content.insert("metadata".to_string(), metadata_text);
                }
            },
            None => {
                debug!("{}: Extracting metadata using prompt:\n{}", PLUGIN_NAME, metadata_prompt);
                let metadata_text = invoke_llm_func_with_lock(
                    llm_api_mutex,
                    metadata_prompt.as_str(),
                    llm_params,
                    summarize_fn
                );
                raw_doc.generated_content.insert("metadata".to_string(), metadata_text);
            }
        }
        info!("{}: Completed processing document - '{}'.", PLUGIN_NAME, raw_doc.title);
    }
}

// document classification:
pub fn run_document_classifier(tx: Sender<Document>, rx: Receiver<Document>, app_config: &Config, _api_mutexes: &mut HashMap<String, Arc<Mutex<isize>>>) {
    info!("Starting module doc_type - Document classification.");
    let mut doc_counter: u32 = 0;
    let prompt_template = get_cfg!("prompt_metadata", app_config, "Identify industry categories from this text. Return as String array in json format.\nTEXT:\n");

    for mut doc in rx {

        if doc.module.eq_ignore_ascii_case("rbi_new") {
            let doc_type = classify_rbi_document_type(doc.title.as_str(), doc.section_name.as_str());
            doc.classification.insert("doc_type".to_string(), doc_type.to_string());
        } else if doc.module.eq_ignore_ascii_case("sebi") {
            let doc_type = classify_sebi_document_type(doc.title.as_str(), doc.url.as_str(), doc.section_name.as_str());
            doc.classification.insert("doc_type".to_string(), doc_type.to_string());
        } else if doc.module.eq_ignore_ascii_case("irdai") {
            let doc_type = classify_irdai_document_type(doc.title.as_str(), doc.url.as_str(), doc.section_name.as_str());
            doc.classification.insert("doc_type".to_string(), doc_type.to_string());
        }

        // for future use, add categorisation rules for other modules/websites:

        match tx.send(doc) {
            Result::Ok(_) => { doc_counter += 1; },
            Err(e) => error!("Document classification: When sending processed doc via tx: {}", e)
        }

    }
    info!("Document classification: Completed processing a total of {} documents.", doc_counter);
}



fn classify_rbi_document_type(title: &str, section_name: &str) -> &'static str {

    // process speeches first:
    match section_name {
        "Speeches" => return "speech",
        "speeches" => return "speech",
        _ => debug!("Unknown section type '{}'", section_name)
    }
    let auctions_pattn: Regex = Regex::new(r"(auction under |auction held|Auction Result|Underwriting Auction|Auction of State|Auction of Government|Auction Results|Treasury Bills auction|Auctions Conducted on)").unwrap();
    let repo_revrepo_pattn: Regex = Regex::new(r"(Variable Rate Reverse Repo|Variable Rate Repo)").unwrap();
    let mm_operation_pattn: Regex = Regex::new(r"(Money Market Operations|Reserve Money for)").unwrap();
    let bonds_pattn: Regex = Regex::new(r"(Sovereign Gold Bond \(SGB\) Scheme|Sovereign Gold Bonds,|Buyback of Government of India Dated Securities)").unwrap();
    let survey_pattn: Regex = Regex::new(r"(Launch[esing]* [of ]*[\w\s\d\(\)-]+ Survey|^Survey on )").unwrap();
    let int_rate_pattn: Regex = Regex::new(r"(Rate of interest on Government of India|Lending and Deposit Rates of Scheduled Commercial Banks)").unwrap();
    let mkt_data_pattn: Regex = Regex::new(r"Foreign Exchange Turnover Data:").unwrap();

    // mark doctype based on patterns:
    if auctions_pattn.is_match(title){
        return "market_action";
    }
    if repo_revrepo_pattn.is_match(title){
        return "market_action";
    }
    if mm_operation_pattn.is_match(title){
        return "market_action";
    }
    if bonds_pattn.is_match(title){
        return "market_action";
    }
    if survey_pattn.is_match(title){
        return "survey";
    }
    if int_rate_pattn.is_match(title){
        return "market_action";
    }
    if mkt_data_pattn.is_match(title){
        return "market_data";
    }
    // if nothing matches, then by default this document is of type regulatory-notification:
    return "regulatory-notification";
}

fn classify_irdai_document_type(title: &str, url: &str, section_name: &str) -> &'static str {
    // default type regulatory-notification:
    "regulatory-notification"
}

fn classify_sebi_document_type(title: &str, url: &str, section_name: &str) -> &'static str {

    // process speeches first:
    match section_name {
        "Speeches" => return "speech",
        "speeches" => return "speech",
        _ => debug!("Unknown section type '{}'", section_name)
    }

    let notice_pattn: Regex = Regex::new(r"(Notice of Demand under Recovery Certificate|Notice of Demand for Recovery Certificate|Notices of Attachment dated|Notice of Demand dated|Public Notice For E-Auction|Warning letter issued to)").unwrap();
    let indiv_order_pattn: Regex = Regex::new(r"(Remittance Order for Recovery Certificate|General Remittance Order dated|Completion Order for Recovery Certificate|Completion of Recovery Certificate|Release Order for Recovery Certificate|SEBI Order for Compliance|Settlement Order in respect of|Recovery Proceedings under|Adjudication Order)").unwrap();
    let appeal1_pattn: Regex = Regex::new(r"(Appeal No)").unwrap();
    let appeal2_pattn: Regex = Regex::new(r"(filed by)").unwrap();
    let prospectus_pattn: Regex = Regex::new(r"(Prospectus|Addendum to DRHP)").unwrap();
    let filings_url_pattn: Regex = Regex::new(r"(/filings/public-issues/|/filings/takeovers/|/filings/rights-issues/|/filings/invit-public-issues/)").unwrap();

    if notice_pattn.is_match(title){
        return "individual_notice";
    }
    else if indiv_order_pattn.is_match(title){
        return "individual_order";
    }
    else if appeal1_pattn.is_match(title) && appeal2_pattn.is_match(title) {
        return "individual_appeal";
    }
    else if prospectus_pattn.is_match(title){
        return "prospectus";
    }
    else if filings_url_pattn.is_match(url){
        return "filings";
    }
    // by default this document is of type regulatory-notification:
    "regulatory-notification"
}


// --- rbi document processing
fn run_rbi_scanner(tx: Sender<Document>, cfg: Arc<config::Config>){

    let enabled = get_plugin_cfg!("rbi_new", "enabled", &cfg).unwrap().parse::<bool>().unwrap();

    if enabled == true {

        info!("Starting custom plugin - rbi_new.");
        let database_filename = get_cfg!("completed_urls_datafile", &cfg, "hzn_scan_urls.db");
        let data_folder = get_cfg!("data_dir", &cfg, "data");
        let pdf_folder = get_cfg!("pdf_data_dir", &cfg, "data/master_data");

        let mut counter = 0;
        let mut netw_params = read_network_parameters(&cfg);
        netw_params.referrer_url = Some("https://website.rbi.org.in/".to_string());
        let client = make_http_client(&netw_params);

        let mut already_retrieved_urls = get_urls_from_database(database_filename.as_str(), "rbi_new");
        info!("For Plugin {}: Got {} previously retrieved urls from table.", "rbi_new", already_retrieved_urls.len());

        let mut rng = rand::rng();

        let max_pages = get_plugin_cfg!("rbi_new", "max_pages", &cfg).unwrap().parse::<u64>().unwrap();
        let maxitemsinpage = get_plugin_cfg!("rbi_new", "items_per_page", &cfg).unwrap().parse::<u64>().unwrap();

        let listing_urls = vec![
            ("https://website.rbi.org.in/web/rbi/notifications/rbi-circulars", "Circular"),
            ("https://website.rbi.org.in/web/rbi/press-releases", "Press Release"),
            ("https://website.rbi.org.in/web/rbi/notifications/draft-notifications", "Draft Notifications"),
            ("https://website.rbi.org.in/web/rbi/notifications/master-directions", "Master Directions"),
            ("https://website.rbi.org.in/en/web/rbi/notifications/master-circulars", "Master Circulars"),
            ("https://website.rbi.org.in/web/rbi/notifications", "Notifications"),
            ("https://website.rbi.org.in/web/rbi/about-us/legal-framework/act", "Acts"),
            ("https://website.rbi.org.in/web/rbi/about-us/legal-framework/rules", "Rules"),
            ("https://website.rbi.org.in/web/rbi/about-us/legal-framework/regulations", "Regulations"),
            ("https://website.rbi.org.in/web/rbi/about-us/legal-framework/schemes", "Schemes"),
            ("https://website.rbi.org.in/web/rbi/speeches", "Speeches"),
            ("https://website.rbi.org.in/web/rbi/interviews", "Interviews and Media Interactions"),
            ("https://website.rbi.org.in/web/rbi/publications/reports/reports_list", "Reports"),
            ("https://website.rbi.org.in/web/rbi/publications/rbi-bulletin", "Bulletin"),
            ("https://website.rbi.org.in/web/rbi/publications/reports/financial_stability_reports", "Reports"),
            ("https://website.rbi.org.in/web/rbi/publications/chapters?category=24927745", "Report on Currency and Finance"),
            ("https://website.rbi.org.in/web/rbi/publications/articles?category=24927873", "Monetary Policy Report"),
        ];

        for (starter_url, section_name) in listing_urls {
            for pageno in 1..(max_pages + 1) {
                let urlargs = format!("?delta={}&start={}", maxitemsinpage, pageno);
                let mut listing_url_with_args = String::from(starter_url);
                listing_url_with_args.push_str(&urlargs);

                // retrieve content from this url and extract vector of documents, mainly individual urls to retrieve.
                let content = http_get(&listing_url_with_args, &client, (&netw_params).retry_times, rng.random_range((&netw_params).wait_time_min..=((&netw_params).wait_time_min * 3)));

                let count_of_docs = get_docs_from_listing_page(
                    content,
                    &tx,
                    &listing_url_with_args,
                    section_name,
                    &mut already_retrieved_urls,
                    &client,
                    &netw_params,
                    data_folder.as_str(),
                    pdf_folder.as_str(),
                    "rbi_new",
                    "https://website.rbi.org.in/"
                );
                counter += count_of_docs;
            }
        }
        info!("Completed retrieving {} document for plugin - rbi_new.", counter);
    }
}

fn extract_docinfo_from_row(module_name: &str, row_each: ElementRef, source_url: &str) -> Document{
    // TODO: generalize this - rbi_new, irdai, etc.
    let mut this_new_doc = Document::default();
    // init document with default "others" categories in classification field.
    this_new_doc.classification = HashMap::from( [
        ("channel".to_string(),"other".to_string()),
        ("customer_type".to_string(), "other".to_string()),
        ("function".to_string(),"other".to_string()),
        ("market_type".to_string(),"other".to_string()),
        ("occupation".to_string(),"other".to_string()),
        ("product_type".to_string(),"other".to_string()),
        ("risk_type".to_string(),"other".to_string()),
        // document type:
        ("doc_type".to_string(),"regulatory-notification".to_string()),
    ]);

    let mut date_str = String::from("");
    let mut pdf_link_selector = scraper::Selector::parse("a.matomo_download").unwrap();
    let mut doctitle_selector = scraper::Selector::parse("span.mtm_list_item_heading").unwrap();
    let mut alink_selector = scraper::Selector::parse("a.mtm_list_item_heading").unwrap();

    if module_name.contains("rbi_new"){
        alink_selector = scraper::Selector::parse("a.mtm_list_item_heading").unwrap();
        let date_selector = scraper::Selector::parse("div.notification-date>span").unwrap();
        for date_div_elem in row_each.select(&date_selector) {
            date_str = clean_text(date_div_elem.inner_html());
            match NaiveDate::parse_from_str(date_str.as_str(), "%b %d, %Y"){
                Ok(naive_date) => {
                    this_new_doc.publish_date_ms = to_local_datetime(naive_date).timestamp();
                    this_new_doc.publish_date = naive_date.format("%Y-%m-%d").to_string();
                },
                Err(date_err) => {
                    error!("{}: Could not parse date '{}', error: {}", module_name, date_str.as_str(), date_err)
                }
            }
        }
        pdf_link_selector = scraper::Selector::parse("a.matomo_download").unwrap();
        doctitle_selector = scraper::Selector::parse("span.mtm_list_item_heading").unwrap();
    }
    else if module_name.contains("irdai"){
        alink_selector = scraper::Selector::parse("td.table-col-subTitle>a").unwrap();
        let date_selector = scraper::Selector::parse("td.table-col-lastUpdated").unwrap();
        for date_div_elem in row_each.select(&date_selector) {
            date_str = clean_text(date_div_elem.inner_html());
            match NaiveDate::parse_from_str(date_str.as_str(), "%d-%m-%Y"){
                Ok(naive_date) => {
                    this_new_doc.publish_date_ms = to_local_datetime(naive_date).timestamp();
                    this_new_doc.publish_date = naive_date.format("%Y-%m-%d").to_string();
                },
                Err(date_err) => {
                    error!("{}: Could not parse date '{}', error: {}", module_name, date_str.as_str(), date_err)
                }
            }
        }
        pdf_link_selector = scraper::Selector::parse("div.doc-download>a").unwrap();
        doctitle_selector = scraper::Selector::parse("td.table-col-shortDesc").unwrap();
        let unique_id_selector = scraper::Selector::parse("td.table-col-referenceNumber").unwrap();
        for uniqueid_elem in row_each.select(&unique_id_selector) {
            this_new_doc.unique_id = clean_text(get_text_from_element(uniqueid_elem));
        }

    }
    // get url:
    for alink_elem in row_each.select(&alink_selector) {
        if let Some(href) = alink_elem.value().attr("href") {
            this_new_doc.url = href.parse().unwrap();
        }
    }
    for pdf_url_elem in row_each.select(&pdf_link_selector) {
        if let Some(href) = pdf_url_elem.value().attr("href") {
            this_new_doc.pdf_url = href.parse().unwrap();
        }
    }
    this_new_doc.links_inward = vec![source_url.to_string()];


    for title_span_elem in row_each.select(&doctitle_selector) {
        this_new_doc.title = clean_text(get_text_from_element(title_span_elem));
    }

    let mut snippet_text = String::from(" ");
    let description_snippet_selector = scraper::Selector::parse("div.notifications-description p").unwrap();
    let snippet_regex: Regex = Regex::new(
        r"(RBI[/A-Z]+\d{4}-\d{2,4}/\d*)(.+\d{4}-\d{2,4}[ ]*)((January|February|March|April|May|June|July|August|September|October|November|December)[\d ]+,[\d ]+)(.+)(Madam|Madam[ ]*/[ ]*Dear Sir|Dear Sir/|Dear Sir /|Madam / Dear Sir|Madam / Sir|$)"
    ).unwrap();
    for snippet_elem in row_each.select(&description_snippet_selector) {
        let description_snippet = clean_text(
            get_text_from_element(snippet_elem)
        )
            .replace("\r\n", " ")
            .replace("\n", " ");
        snippet_text.push_str(" ");
        snippet_text.push_str(description_snippet.as_str());
        if let Some(caps) = snippet_regex.captures(snippet_text.as_str()) {
            // let id_prefix = caps.get(1).unwrap().as_str();
            this_new_doc.unique_id = clean_text(caps.get(2).unwrap().as_str().to_string());
            // let pubdate_longformat_str = caps.get(3).unwrap().as_str();
            this_new_doc.recipients = caps.get(5).unwrap().as_str().to_string();
        }
    }


    return this_new_doc;
}

fn get_docs_from_listing_page(
    content: String,
    tx: &Sender<document::Document>,
    url_listing_page: &String,
    section_name: &str,
    already_retrieved_urls: &mut HashSet<String>,
    client: &reqwest::blocking::Client,
    netw_params: &NetworkParameters,
    data_folder: &str,
    pdf_folder: &str,
    module_name: &str,
    base_url: &str
) -> usize
{
    let mut counter: usize=0;
    info!("{}: Retrieving url listing from: {}", module_name, url_listing_page);
    let html_document = scraper::Html::parse_document(&content.as_str());
    //
    let mut rows_selector = scraper::Selector::parse("div.notifications-row-wrapper>div>div").unwrap();
    let mut plugin_full_name = "Reserve Bank of India".to_string();
    if module_name.contains("rbi_new"){
        rows_selector = scraper::Selector::parse("div.notifications-row-wrapper>div>div").unwrap();
    } else if module_name.contains("irdai") {
        plugin_full_name = "Insurance Regulatory Development Agency of India".to_string();
        rows_selector = scraper::Selector::parse("tbody.table-data>tr").unwrap();
    }

    'rows_loop: for row_each in html_document.select(&rows_selector){
        let mut this_new_doc = extract_docinfo_from_row(module_name, row_each, url_listing_page);
        this_new_doc.module = module_name.to_string();
        this_new_doc.plugin_name = plugin_full_name.clone();
        this_new_doc.section_name = section_name.to_string();
        this_new_doc.source_author = plugin_full_name.clone();
        this_new_doc.data_proc_flags = document::DATA_PROC_CLASSIFY_INDUSTRY |
            document::DATA_PROC_CLASSIFY_MARKET | document::DATA_PROC_CLASSIFY_PRODUCT |
            document::DATA_PROC_EXTRACT_NAME_ENTITY | document::DATA_PROC_SUMMARIZE |
            document::DATA_PROC_EXTRACT_ACTIONS;
        if already_retrieved_urls.contains(&this_new_doc.url){
            info!("{}: Ignoring already retrieved url: {}", module_name, this_new_doc.url);
            continue 'rows_loop;
        }
        if let Some(proper_url) = check_and_fix_url(this_new_doc.url.as_str(), base_url){
            this_new_doc.url = proper_url;
        }else{
            info!("{}: Ignoring invalid url: {}", module_name, this_new_doc.url);
            continue 'rows_loop;
        }
        populate_content_in_doc(&mut this_new_doc, client, netw_params);

        _ = already_retrieved_urls.insert(this_new_doc.url.clone());
        let filename = make_unique_filename(&this_new_doc, "json");
        let json_file_path = Path::new(data_folder).join(filename);
        this_new_doc.filename = String::from(
            json_file_path.as_path().to_str().expect("Not able to convert path to string")
        );

        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            load_pdf_content(&mut this_new_doc, &client, pdf_folder);
        }));
        if result.is_err() {
            if let Err(errvar) = result {
                error!("{}: When reading PDF of document '{}' the error was: {:?}", module_name, this_new_doc.title, errvar);
            }
        }

        if this_new_doc.text.len() < MIN_ACCEPTABLE_SUMMARY_CHARS {
            error!("{}: Unable to extract text from pdf for document - '{}'", module_name, this_new_doc.title);
        }

        if this_new_doc.recipients.len() > 2 {
            this_new_doc.recipients = clean_recepients(this_new_doc.recipients.as_str());
        }
        match tx.send(this_new_doc) {
            Result::Ok(_res) => {
                counter += 1;
            },
            Err(e) => error!("{}: When sending document via channel: {}", module_name, e)
        }
    }
    return counter;
}

fn clean_recepients(recepients: &str) -> String{
    let letter_greeting_regex: Regex = Regex::new(
        r"([Dear ]*Madam[ ]*/[Dear ]*Sir|Dear Sir/|Dear Sir /|Madam / Dear Sir|Madam / Sir|Madam|Sir)"
    ).unwrap();
    for substr in letter_greeting_regex.split(recepients){
        return substr.trim().to_string();
    }
    return recepients.to_string();
}

fn populate_content_in_doc(this_new_doc: &mut Document, client: &reqwest::blocking::Client, netw_params: &NetworkParameters) {
    let mut rng = rand::rng();
    let whole_page_content_selector = scraper::Selector::parse("div.Notification-content-wrap").unwrap();
    let html_content = http_get(
        &this_new_doc.url,
        &client,
        netw_params.retry_times,
        rng.random_range(netw_params.wait_time_min..=(netw_params.wait_time_max*3))
    );
    let page_content = scraper::Html::parse_document(html_content.as_str());
    for page_div in page_content.select(&whole_page_content_selector){
        this_new_doc.html_content = page_div.html();
    }
}


fn run_insights_extraction(tx: Sender<Document>, rx: Receiver<Document>, app_config: &Config, api_mutexes: &mut HashMap<String, Arc<Mutex<isize>>>){
    info!("insights: Starting module - Actionables Extraction.");
    let mut doc_counter: u32 = 0;
    let mut prompt_insights_part = String::from("prompt_insights_part");
    let mut prompt_insights_exec = String::from("prompt_insights_exec");

    match app_config.get_string("prompt_insights_part") {
        Ok(param_val) => prompt_insights_part = param_val,
        Err(e) => error!("When getting prompt part summary: {}, using default value: {}",
            e, prompt_insights_part)
    }
    match app_config.get_string("prompt_insights_summary") {
        Ok(param_val) => prompt_insights_exec = param_val,
        Err(e) => error!("When getting prompt exec summary: {}, using default value: {}",
            e, prompt_insights_exec)
    }

    let mut llm_params = prepare_llm_parameters(app_config, prompt_insights_part.clone(), "insights");
    match get_plugin_cfg!("insights", "overwrite", &app_config) {
        Some(param_val_str) => {
            match param_val_str.trim().parse(){
                Ok(param_val) => llm_params.overwrite_existing_value = param_val,
                Err(e) => error!("When parsing parameter 'overwrite' as bool value: {}", e)
            }
        }, None => error!("insights: Could not get parameter 'overwrite', using default value of: {}", llm_params.overwrite_existing_value)
    };
    let mut option_mutex = api_mutexes.get(llm_params.llm_service.as_str());

    for mut doc in rx {
        // only process these modules for actionables:
        if doc.module.eq_ignore_ascii_case("rbi_new")
            || doc.module.eq_ignore_ascii_case("PDF")
            || doc.module.eq_ignore_ascii_case("sebi")
            || doc.module.eq_ignore_ascii_case("irdai")
            || doc.module.eq_ignore_ascii_case("npci") // add more module names for future use
        {
            update_doc_with_actions(&mut llm_params, &mut doc, prompt_insights_part.as_str(), prompt_insights_exec.as_str(), option_mutex);
            let mut doc_type = "speech".to_string();
            match doc.classification.get("doc_type"){
                Some(mapped_value) => {doc_type = mapped_value.clone()},
                None => {}
            }
            if doc_type.contains("speech") || doc_type.contains("regulatory-notification") {
                match tx.send(doc) {
                    Result::Ok(_) => { doc_counter += 1; },
                    Err(e) => error!("insights: When sending processed doc to next plugin: {}", e)
                }
            }
        }
    }
    info!("insights: Completed extracting Actionables for a total of {} documents.", doc_counter);
}


fn update_doc_with_actions(llm_params: &mut LLMParameters, doc: &mut Document, prompt_part: &str, prompt_exec_summary: &str, llm_api_mutex_result: Option<&Arc<Mutex<isize>>>) {
    const PLUGIN_NAME: &str = "insights";
    if doc.text.len() < MIN_ACCEPTABLE_SUMMARY_CHARS {
        error!("{}: No text content available for actionables for document - '{}'",
            PLUGIN_NAME,
            doc.title,
        );
        return;
    }
    let genai_fn = llm_params.sumarize_fn;
    let mut all_insights: String = String::new();
    llm_params.prompt = prompt_part.to_string();
    let mut updated_text_parts:  Vec<HashMap<String, Value>> = Vec::new();
    let mut part_counter = doc.text_parts.len();
    let insights_summ_prompt = format!(
        "{}\n{}\nPublish Date: {}",
        prompt_exec_summary,
        doc.title,
        doc.publish_date
    );
    let full_text_tokens = TOKENS_PER_WORD * ((word_count(insights_summ_prompt.as_str()) + word_count(doc.text.as_str()) ) as f64) ;
    if let Some(mut llm_api_mutex) = llm_api_mutex_result {
        info!("{}: Extracting actionables from each of {} parts of document - '{}'", PLUGIN_NAME, doc.text_parts.len(), doc.title);
        while doc.text_parts.is_empty() == false {
            match &doc.text_parts.pop() {
                None => { break; }
                Some(text_part) => {
                    if llm_params.overwrite_existing_value == false {
                        match text_part.get("insights") {
                            Some(summary) => {
                                if summary.to_string().len() > MIN_ACCEPTABLE_SUMMARY_CHARS {
                                    all_insights.push_str(&format!("{},", summary));
                                    let text_part_map_clone = text_part.clone();
                                    updated_text_parts.push(text_part_map_clone);
                                    continue;
                                }
                            }
                            _ => {}
                        }
                    }
                    match text_part.get("text") {
                        Some(text_string) => {
                            info!("{}: Started generating Actionables for text part #{} of document - '{}'", PLUGIN_NAME, part_counter, doc.title);
                            let mut text_part_map_clone = text_part.clone();
                            let summary_text = invoke_llm_func_with_lock(
                                llm_api_mutex,
                                text_string.to_string().as_str(),
                                llm_params,
                                genai_fn
                            );
                            let insights_json_str = summary_text.replace("```json", "```");
                            let start_bytes = insights_json_str.find("```").unwrap_or(0);
                            let insights_json_str = &insights_json_str[start_bytes..];
                            let end_bytes = insights_json_str.find("```").unwrap_or(insights_json_str.len());
                            let insights_json_str = &insights_json_str[start_bytes..end_bytes];
                            let insights_struct: serde_json::Value = serde_json::from_str(insights_json_str)
                                .unwrap_or(Value::String(summary_text.clone()));
                            _ = text_part_map_clone.insert("insights".to_string(), insights_struct);
                            updated_text_parts.push(text_part_map_clone);
                            all_insights.push_str("\n");
                            all_insights.push_str(summary_text.as_str());
                        },
                        None => {}
                    }
                }
            }
            part_counter = part_counter - 1;
        }
        updated_text_parts.reverse();
        doc.text_parts = updated_text_parts;
        if full_text_tokens <= (llm_params.num_context as f64) {
            all_insights = doc.text.clone();
        }
        llm_params.prompt = insights_summ_prompt.to_string();
        let full_context = word_count(
            &format!("{}{}", insights_summ_prompt, all_insights)
        ) as f64 * TOKENS_PER_WORD;
        let count_exec_summ_sub_parts = full_context / MAX_TOKENS;
        match doc.generated_content.get("actions_summary") {
            Some(existing_exec_summary) => {
                if existing_exec_summary.to_string().len() < MIN_ACCEPTABLE_SUMMARY_CHARS {
                    info!("insights: Actions summary would need to be generated in {} parts.",
                        count_exec_summ_sub_parts);
                    let exec_summary_text = invoke_llm_func_with_lock(
                        llm_api_mutex,
                        all_insights.as_str(),
                        llm_params,
                        genai_fn
                    );
                    doc.generated_content.insert("actions_summary".to_string(), exec_summary_text);
                }
            },
            None => {
                let exec_summary_text = invoke_llm_func_with_lock(
                    llm_api_mutex,
                    all_insights.as_str(),
                    llm_params,
                    genai_fn
                );
                doc.generated_content.insert("actions_summary".to_string(), exec_summary_text);
            }
        }

        info!("insights Actionables Extraction: Completed processing document - '{}'.", doc.title);
    }
}

// summarise
fn run_document_summarizer(tx: Sender<document::Document>, rx: Receiver<document::Document>, app_config: &Config, api_mutexes: &mut HashMap<String, Arc<Mutex<isize>>>){
    const PLUGIN_NAME: &str = "summarize";
    info!("{}: Getting configuration for module.", PLUGIN_NAME);
    let mut doc_counter: u32 = 0;
    let prompt_summary_part = get_cfg!("prompt_summary_part", app_config, "Summarize this text:\n");
    let prompt_summary_exec = get_cfg!("prompt_summary_exec", app_config, "Summarize this text:\n");
    let mut llm_params = prepare_llm_parameters(app_config, prompt_summary_part.clone(), PLUGIN_NAME);
    let mut option_mutex = api_mutexes.get(llm_params.llm_service.as_str());

    for mut doc in rx {
        update_doc_with_summary(&mut llm_params, &mut doc, prompt_summary_part.as_str(), prompt_summary_exec.as_str(), option_mutex);
        match tx.send(doc) {
            Result::Ok(_) => {doc_counter += 1;},
            Err(e) => error!("{}: When sending processed doc via tx: {}", PLUGIN_NAME, e)
        }
    }
    info!("{}: Completed processing {} documents.", PLUGIN_NAME, doc_counter);
}

fn update_doc_with_summary(llm_params: &mut LLMParameters, raw_doc: &mut document::Document, prompt_part: &str, prompt_exec_summary: &str, llm_api_mutex_result: Option<&Arc<Mutex<isize>>>) {
    const PLUGIN_NAME: &str = "summarize";
    if raw_doc.text.len() < MIN_ACCEPTABLE_SUMMARY_CHARS {
        error!("{}: No text content available for summarization of document - '{}'",
            PLUGIN_NAME,
            raw_doc.title,
        );
        return;
    }
    let summarize_fn = llm_params.sumarize_fn;
    if let Some(mut llm_api_mutex) = llm_api_mutex_result {
        let mut all_summaries: String = String::new();
        let exec_summ_prompt = format!(
            "{}\n{}\nPublish Date: {}",
            prompt_exec_summary,
            raw_doc.title,
            raw_doc.publish_date
        );
        let full_text_tokens = TOKENS_PER_WORD * ((word_count(exec_summ_prompt.as_str()) + word_count(raw_doc.text.as_str())) as f64);
        if full_text_tokens > (llm_params.num_context as f64) {
            info!("{}: Summarizing in {} parts, document - '{}'",
                PLUGIN_NAME, raw_doc.text_parts.len(), raw_doc.title
            );
            llm_params.prompt = prompt_part.to_string();
            let total_num_of_parts = raw_doc.text_parts.len();
            let mut part_no = raw_doc.text_parts.len();
            let mut updated_text_parts: Vec<HashMap<String, Value>> = Vec::new();

            while raw_doc.text_parts.is_empty() == false {
                match &raw_doc.text_parts.pop() {
                    None => { break; }
                    Some(text_part) => {
                        if llm_params.overwrite_existing_value == false {
                            match text_part.get("summary") {
                                Some(summary) => {
                                    if summary.to_string().len() > MIN_ACCEPTABLE_SUMMARY_CHARS {
                                        all_summaries.push_str(&format!("{},", summary));
                                        let text_part_map_clone = text_part.clone();
                                        updated_text_parts.push(text_part_map_clone);
                                        info!("Not re-generating summary for part #{}", part_no);
                                        part_no = part_no - 1;
                                        continue;
                                    } else {
                                        info!("Although overwrite={}, existing summary is inadequate, so re-generating it for part #{}",
                                            llm_params.overwrite_existing_value,
                                            part_no);
                                    }
                                }
                                _ => {}
                            }
                        }
                        match text_part.get("text") {
                            Some(text_string) => {
                                let this_parts_text_length = text_string.to_string().len();
                                if this_parts_text_length > MIN_ACCEPTABLE_SUMMARY_CHARS {
                                    info!("Summarizing part #{} of document - '{}'", part_no, raw_doc.title);
                                    let mut text_part_map_clone = text_part.clone();
                                    let summary_text = invoke_llm_func_with_lock(
                                        llm_api_mutex,
                                        format!("(Text excerpt number {} from total of {} excerpts of document)\n{}", part_no, total_num_of_parts, text_string.to_string()).as_str(),
                                        llm_params,
                                        summarize_fn
                                    );
                                    _ = text_part_map_clone.insert("summary".to_string(), Value::String(summary_text.clone()));
                                    updated_text_parts.push(text_part_map_clone);
                                    all_summaries.push_str("\n");
                                    all_summaries.push_str(summary_text.as_str());
                                } else {
                                    error!("Inadequate quantity of text to summarize (length = {}) for part #{}", this_parts_text_length, part_no);
                                }
                            },
                            None => {}
                        }
                    }
                }
                part_no = part_no - 1;
            }
            updated_text_parts.reverse();
            raw_doc.text_parts = updated_text_parts;
        } else {
            info!("{}: Summarizing entire document at once - '{}'",
                PLUGIN_NAME, raw_doc.title
            );
            all_summaries = raw_doc.text.clone();
        }

        llm_params.prompt = exec_summ_prompt.to_string();
        let full_context = word_count(
            &format!("{}{}", exec_summ_prompt, all_summaries)
        ) as f64 * TOKENS_PER_WORD;
        let count_exec_summ_sub_parts = full_context / MAX_TOKENS;

        match raw_doc.generated_content.get("exec_summary") {
            Some(existing_exec_summary) => {
                if existing_exec_summary.to_string().len() < MIN_ACCEPTABLE_SUMMARY_CHARS {
                    info!("Generating executive summary (in {} parts)",
                            count_exec_summ_sub_parts);
                    let exec_summary_text = invoke_llm_func_with_lock(
                        llm_api_mutex,
                        all_summaries.as_str(),
                        llm_params,
                        summarize_fn
                    );
                    raw_doc.generated_content.insert("exec_summary".to_string(), exec_summary_text);
                }
            },
            None => {
                let exec_summary_text = invoke_llm_func_with_lock(
                    llm_api_mutex,
                    all_summaries.as_str(),
                    llm_params,
                    summarize_fn
                );
                raw_doc.generated_content.insert("exec_summary".to_string(), exec_summary_text);
            }
        }
        info!("{}: Completed processing document - '{}'.", PLUGIN_NAME, raw_doc.title);
    }
}

// --- change detection:

fn run_changes_extraction(tx: Sender<Document>, rx: Receiver<Document>, app_config: &Config, api_mutexes: &mut HashMap<String, Arc<Mutex<isize>>>){
    info!("changes_from_previous: Starting module to extract and summarize differences from previous notifications.");
    let mut doc_counter: u32 = 0;
    let prompt_insights_change_summary = get_cfg!("prompt_insights_change_summary", app_config, "Summarize the differences:");
    let prompt_insights_changes = get_cfg!("prompt_insights_changes", app_config, "Tabulate the differences in following documents.");

    let mut llm_params = prepare_llm_parameters(app_config, prompt_insights_changes.clone(), "changes_from_previous");
    match get_plugin_cfg!("changes_from_previous", "overwrite", &app_config) {
        Some(param_val_str) => {
            match param_val_str.trim().parse(){
                Ok(param_val) => llm_params.overwrite_existing_value = param_val,
                Err(e) => error!("changes_from_previous: When parsing parameter 'overwrite' as bool value: {}", e)
            }
        }, None => error!("changes_from_previous: Could not get parameter 'overwrite', using default value of: {}", llm_params.overwrite_existing_value)
    };
    let mut option_mutex = api_mutexes.get(llm_params.llm_service.as_str());

    for mut doc in rx {
        update_doc_with_changes(&mut llm_params, &mut doc, prompt_insights_changes.as_str(), prompt_insights_change_summary.as_str());

        match tx.send(doc) {
            Result::Ok(_) => { doc_counter += 1; },
            Err(e) => error!("changes_from_previous: When sending processed doc via tx: {}", e)
        }
    }
    info!("changes_from_previous: Completed processing a total of {} documents.", doc_counter);
}

fn get_previous_doc(curr_doc: &Document) -> Document {
    let curr_doc_title = curr_doc.title.as_str();
    let curr_doc_timestamp = curr_doc.publish_date_ms;
    let curr_doc_unique_id = curr_doc.unique_id.as_str();

    // first, load old circular document from field: curr_doc.generated.previous_circular_file
    match curr_doc.generated_content.get("previous_circular_file") {
        Some(previous_circular_file) => {
            match std::fs::File::open(previous_circular_file.clone()){
                Ok(open_file) => {
                    match serde_json::from_reader::<std::fs::File, document::Document>(open_file) {
                        Result::Ok(doc_loaded_from_file) => {
                            return doc_loaded_from_file;
                        }
                        Err(e) => error!("{}: When trying to load json {}: {}", "changes_from_previous", previous_circular_file, e),
                    }
                }
                Err(e) => error!("{}: When trying to open file {}: {}", "changes_from_previous", previous_circular_file, e),
            }
        },
        None => {}
    }
    // TODO: if this is not available, then search for same name (title) but date before current date:

    return Document::default();
}

fn update_doc_with_changes(
    llm_params: &mut LLMParameters,
    doc: &mut Document,
    prompt_insights_changes: &str,
    prompt_changes_summary: &str
) {
    const PLUGIN_NAME: &str = "changes_from_previous";
    let genai_fn = llm_params.sumarize_fn;

    let previous_doc = get_previous_doc(&doc);
    if previous_doc.text.len() < MIN_ACCEPTABLE_SUMMARY_CHARS {
        info!("{}: Unable to identify previous version to compare changes with new circular - '{}'", PLUGIN_NAME, doc.title);
        return
    }

    let insights_changes_prompt = format!(
        "{}\nOld Circular:\n{}\nNew Circular: {}",
        prompt_insights_changes,
        previous_doc.text,
        doc.text
    );

    let summary_changes_prompt = format!(
        "{}\nOld Circular:\n{}\n\nNew Circular:\n{}",
        prompt_changes_summary,
        previous_doc.text,
        doc.text
    );

    info!("{}: Generating table of changes from document - '{}'", PLUGIN_NAME, doc.title);
    llm_params.prompt = insights_changes_prompt.to_string();

    match doc.generated_content.get("changes_parawise_comparison") {
        Some(existing_changes) => {
            if existing_changes.to_string().len() < MIN_ACCEPTABLE_SUMMARY_CHARS {
                let extracted_changes_comparison = genai_fn(
                    insights_changes_prompt.as_str(),
                    llm_params
                );
                doc.generated_content.insert("changes_parawise_comparison".to_string(), extracted_changes_comparison);
            }
        }
        None => {
            let extracted_changes_comparison = genai_fn(
                insights_changes_prompt.as_str(),
                llm_params
            );
            doc.generated_content.insert("changes_parawise_comparison".to_string(), extracted_changes_comparison);
        }
    }

    info!("{}: Generating summary of changes in previous vs new document - '{}'", PLUGIN_NAME, doc.title);
    llm_params.prompt = summary_changes_prompt.to_string();
    match doc.generated_content.get("summary_changes") {
        Some(existing_summary_changes) => {
            if existing_summary_changes.to_string().len() < MIN_ACCEPTABLE_SUMMARY_CHARS {
                let extracted_summary_changes = genai_fn(
                    summary_changes_prompt.as_str(),
                    llm_params
                );
                doc.generated_content.insert("summary_changes".to_string(), extracted_summary_changes);
            }
        }
        None => {
            let extracted_summary_changes = genai_fn(
                summary_changes_prompt.as_str(),
                llm_params
            );
            doc.generated_content.insert("summary_changes".to_string(), extracted_summary_changes);
        }
    }
    info!("{}: Completed processing document - '{}'.", PLUGIN_NAME, doc.title);
}


// --- SEBI document processing

fn run_sebi_scanner(tx: Sender<Document>, cfg: Arc<config::Config>){

    const PLUGIN_NAME: &str = "sebi";
    let enabled = get_plugin_cfg!(PLUGIN_NAME, "enabled", &cfg).unwrap().parse::<bool>().unwrap();

    if enabled == true {

        info!("{}: Starting plugin.", PLUGIN_NAME);
        let database_filename = get_cfg!("completed_urls_datafile", &cfg, "hzn_scan_urls.db");
        let data_folder = get_cfg!("data_dir", &cfg, "data");
        let pdf_folder = get_cfg!("pdf_data_dir", &cfg, "data/master_data");

        let mut counter = 0;
        let mut netw_params = read_network_parameters(&cfg);
        netw_params.referrer_url = Some("https://www.sebi.gov.in/".to_string());
        let client = make_http_client(&netw_params);

        let mut already_retrieved_urls = get_urls_from_database(database_filename.as_str(), PLUGIN_NAME);
        info!("For Plugin {}: Got {} previously retrieved urls from table.", PLUGIN_NAME, already_retrieved_urls.len());

        let max_pages = get_plugin_cfg!(PLUGIN_NAME, "max_pages", &cfg).unwrap().parse::<u64>().unwrap();
        // let maxitemsinpage = get_plugin_cfg!(PLUGIN_NAME, "items_per_page", &cfg).unwrap().parse::<u64>().unwrap();

        let section_listing_url_params = vec![
            // sid, ssid, smid, section_name
            ( 1, 1, 0, "Acts"),
            ( 1, 2, 0, "Rules"),
            ( 1, 3, 0, "Regulations"),
            ( 1, 4, 0, "General Orders"),
            ( 1, 5, 0, "Guidelines"),
            ( 1, 6, 0, "Master Circulars"),
            ( 1, 7, 0, "Circulars"),
            ( 1, 8, 0, "Gazette notifications"),
            ( 2, 9, 3, "Settlement Orders"),
            ( 2, 9, 77, "Special Court Orders"),
            ( 2, 9, 7, "Court Orders"),
            ( 2, 9, 6, "Orders of AO"),
            ( 2, 9, 2, "Orders of chairpersons"),
            ( 2, 9, 133, "Orders of ED"),
            ( 2, 1, 0, "Informal Guidance"),
            ( 2, 5, 0, "Recovery Proceedings"),
        ];

        for (sid, ssid, smid, section_name) in section_listing_url_params {
            for pageno in 1..(max_pages + 1) {

                // retrieve content from this url and extract vector of documents, mainly individual urls to retrieve.
                let content = get_sebi_urllist(&client, sid.to_string(), ssid.to_string(), smid.to_string(), section_name, pageno.to_string());
                let url_listing_page = format!("https://www.sebi.gov.in/sebiweb/home/HomeAction.do?doListing=yes&sid={}&ssid={}&smid={}", sid, ssid, smid);

                let count_of_docs = sebi_retrieve_docs(
                    content,
                    &tx,
                    url_listing_page.as_str(),
                    section_name,
                    &mut already_retrieved_urls,
                    &client,
                    &netw_params,
                    data_folder.as_str(),
                    pdf_folder.as_str()
                );

                counter += count_of_docs;
            }
        }
        info!("Completed retrieving {} document for custom plugin - rbi_new.", counter);
    }
}

fn get_sebi_urllist(
    client: &Client,
    sid: String,
    ssid: String,
    smid: String,
    sub_section_name: &str,
    page_no: String
) -> String{

    let listing_url = "https://www.sebi.gov.in/sebiweb/ajax/home/getnewslistinfo.jsp";

    // Make payload
    let params = [
        ("nextValue","1"),
        ("next", "n"),
        ("search", ""),
        ("fromDate", ""),
        ("toDate", ""),
        ("fromYear", ""),
        ("toYear", ""),
        ("deptId", "-1"),
        ("sid", sid.as_str()),
        ("ssid", ssid.as_str()),
        ("smid", smid.as_str()),
        ("ssidhidden", ssid.as_str()),
        ("intmid", "-1"),
        ("sText", "Legal"),
        ("ssText", sub_section_name),
        ("smText", ""),
        ("doDirect", page_no.as_str()),
    ];

    // get response
    match client
        .post(listing_url)
        .json(&params)
        .send()
    {
        Ok(resp) => {
            match resp.text(){
                Ok(resptext) => return resptext,
                Err(_) => {}
            }
        }
        Err(e) => error!("sebi: When getting list of urls: {}", e)
    }

    String::new()
}

pub fn sebi_retrieve_docs (
    content: String,
    tx: &Sender<document::Document>,
    url_listing_page: &str,
    section_name: &str,
    already_retrieved_urls: &mut HashSet<String>,
    client: &reqwest::blocking::Client,
    netw_params: &NetworkParameters,
    data_folder: &str,
    pdf_folder: &str) -> usize
{
    let mut counter: usize=0;

    const PLUGIN_NAME: &str = "sebi";
    let rows_selector = scraper::Selector::parse("table.dataTable>tbody>tr").unwrap();
    info!("{}: Retrieving url listing from: {}", PLUGIN_NAME, url_listing_page);
    let html_document = scraper::Html::parse_document(&content.as_str());

    'rows_loop: for row_each in html_document.select(&rows_selector){

        let mut this_new_doc = extract_sebi_doc_from_row(row_each, url_listing_page);

        this_new_doc.module = PLUGIN_NAME.to_string();
        this_new_doc.plugin_name = "Securities and Exchange Board of India".to_string();
        this_new_doc.source_author = "Securities and Exchange Board of India".to_string();
        this_new_doc.section_name = section_name.to_string();

        this_new_doc.data_proc_flags = document::DATA_PROC_CLASSIFY_INDUSTRY |
            document::DATA_PROC_CLASSIFY_MARKET | document::DATA_PROC_CLASSIFY_PRODUCT |
            document::DATA_PROC_EXTRACT_NAME_ENTITY | document::DATA_PROC_SUMMARIZE |
            document::DATA_PROC_EXTRACT_ACTIONS;

        if already_retrieved_urls.contains(&this_new_doc.url){
            info!("{}: Ignoring already retrieved url: {}", PLUGIN_NAME, this_new_doc.url);
            continue 'rows_loop;
        }

        if let Some(proper_url) = check_and_fix_url(this_new_doc.url.as_str(), "https://www.sebi.gov.in/"){
            this_new_doc.url = proper_url;
        }else{
            info!("{}: Ignoring invalid url: {}", PLUGIN_NAME, this_new_doc.url);
            continue 'rows_loop;
        }

        extract_content_from_sebi_page(&mut this_new_doc, client, netw_params, data_folder);

        _ = already_retrieved_urls.insert(this_new_doc.url.clone());

        let filename = make_unique_filename(&this_new_doc, "json");
        let json_file_path = Path::new(data_folder).join(filename);
        this_new_doc.filename = String::from(
            json_file_path.as_path().to_str().expect("Trying to convert path to string")
        );

        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            load_pdf_content(&mut this_new_doc, &client, pdf_folder);
        }));
        if result.is_err() {
            if let Err(errvar) = result {
                error!("{}: When reading PDF of document '{}' the error was: {:?}", PLUGIN_NAME, this_new_doc.title, errvar);
            }
        }
        info!("{}: Retrieved document titled: '{}', with content text length: {}",
            PLUGIN_NAME, this_new_doc.title, this_new_doc.text.len());

        match tx.send(this_new_doc) {
            Result::Ok(_res) => {
                counter += 1;
            },
            Err(e) => error!("{}: When sending document via channel: {}", PLUGIN_NAME, e)
        }
    }
    return counter;
}

fn extract_sebi_doc_from_row(row_each: ElementRef, url_listing_page: &str) -> Document {
    let mut doc = Document::default();
    doc.links_inward.push(url_listing_page.to_string());

    let alink_selector = scraper::Selector::parse("a").unwrap();
    let cell_selector = scraper::Selector::parse("td").unwrap();

    for cell in row_each.select(&cell_selector) {

        for alink in cell.select(&alink_selector) {
            info!("title: {}", alink.inner_html());
            doc.title = alink.inner_html().trim().to_string();
            info!("Link: {}", alink.attr("href").unwrap_or_else(|| ""));
            doc.url = alink.attr("href").unwrap_or_else(|| "").to_string();
        }

        if cell.select(&alink_selector).count() == 0 {
            let date_str = cell.inner_html();
            match NaiveDate::parse_from_str(date_str.as_str(), "%b %d, %Y"){
                Ok(naive_date) => {
                    doc.publish_date_ms = to_local_datetime(naive_date).timestamp();
                    doc.publish_date = naive_date.format("%Y-%m-%d").to_string();
                },
                Err(date_err) => {
                    error!("Could not parse date '{}', error: {}", date_str.as_str(), date_err)
                }
            }
        }

    }
    doc
}


fn extract_content_from_sebi_page(new_doc: &mut Document, client: &Client, netw_params: &NetworkParameters, _data_folder: &str){

    // extract content from url:
    let content = http_get(&(new_doc.url), client, netw_params.retry_times, netw_params.wait_time_min);
    let html_document = scraper::Html::parse_document(&content.as_str());

    let unique_circ_no_select = scraper::Selector::parse("div.id_area").expect("Construct circular no selector");
    let iframe_select = scraper::Selector::parse("iframe").expect("Construct iframe selector");
    let date_select = scraper::Selector::parse("div.date_value>h5").expect("Construct date selector");
    let span_select = scraper::Selector::parse("span").expect("Construct span selector");

    if let Some(uniquediv) = html_document.select(&unique_circ_no_select).nth(0) {
        if uniquediv.has_children(){
            if let Some(second_span) = uniquediv.select(&span_select).nth(1){
                new_doc.unique_id = second_span.inner_html();
            }
        }
    }

    if let Some(datenode) = html_document.select(&date_select).nth(0){
        let date_str = datenode.inner_html();
        match NaiveDate::parse_from_str(date_str.as_str(), "%b %d, %Y"){
            Ok(naive_date) => {
                new_doc.publish_date_ms = to_local_datetime(naive_date).timestamp();
                new_doc.publish_date = naive_date.format("%Y-%m-%d").to_string();
            },
            Err(date_err) => {
                error!("Could not parse date '{}', error: {}", date_str.as_str(), date_err)
            }
        }
    }

    if let Some(iframe) = html_document.select(&iframe_select).nth(0) {
        if let Some(src_attr) = iframe.attr("src") {
            new_doc.pdf_url = src_attr.replace("../../../web/?file=", "").to_string();
        }
    }

}

// irdai
fn run_irdai_scanner(tx: Sender<Document>, cfg: Arc<config::Config>){

    let enabled = get_plugin_cfg!("irdai", "enabled", &cfg).unwrap().parse::<bool>().unwrap();

    if enabled == true {
        let database_filename = get_cfg!("completed_urls_datafile", &cfg, "hzn_scan_urls.db");
        let data_folder = get_cfg!("data_dir", &cfg, "data");
        let pdf_folder = get_cfg!("pdf_data_dir", &cfg, "data/master_data");

        let mut counter = 0;
        let mut netw_params = read_network_parameters(&cfg);
        netw_params.referrer_url = Some("https://irdai.gov.in/".to_string());
        let client = make_http_client(&netw_params);

        let mut already_retrieved_urls = get_urls_from_database(database_filename.as_str(), "irdai");
        info!("For Plugin {}: Got {} previously retrieved urls from table.", "irdai", already_retrieved_urls.len());

        let mut rng = rand::rng();

        let max_pages = get_plugin_cfg!("irdai", "max_pages", &cfg).unwrap().parse::<u64>().unwrap();
        let maxitemsinpage = get_plugin_cfg!("irdai", "items_per_page", &cfg).unwrap().parse::<u64>().unwrap();

        let listing_urls = vec![
            ("https://irdai.gov.in/circulars", "Circular"),
            ("https://irdai.gov.in/notifications", "Notifications"),
            ("https://irdai.gov.in/guidelines", "Guidelines"),
            ("https://irdai.gov.in/exposure-drafts", "Exposure Draft"),
            ("https://irdai.gov.in/updated-regulations", "Updated Regulations"),
            ("https://irdai.gov.in/consolidated-gazette-notified-regulations", "Gazette Notified Regulations"),
            ("https://irdai.gov.in/rules", "Rules"),
            ("https://irdai.gov.in/acts", "Acts"),
            ("https://irdai.gov.in/warnings-and-penalties", "Enforcement Actions"),
            ("https://irdai.gov.in/orders1", "Orders"),
            ("https://irdai.gov.in/web/guest/whats-new", "Whats New section"),
        ];

        for (starter_url, section_name) in listing_urls {

            info!("irdai: identifying listing from section: {}", section_name);

            for pageno in 1..(max_pages + 1) {
                let urlargs = format!("?p_p_id=com_irdai_document_media_IRDAIDocumentMediaPortlet&p_p_lifecycle=0&p_p_state=normal&p_p_mode=view&_com_irdai_document_media_IRDAIDocumentMediaPortlet_delta={}&_com_irdai_document_media_IRDAIDocumentMediaPortlet_resetCur=false&_com_irdai_document_media_IRDAIDocumentMediaPortlet_cur={}", maxitemsinpage, pageno);

                let mut listing_url_with_args = String::from(starter_url);
                listing_url_with_args.push_str(&urlargs);

                // retrieve content from this url and extract vector of documents, mainly individual urls to retrieve.
                let content = http_get(&listing_url_with_args, &client, (&netw_params).retry_times, rng.random_range((&netw_params).wait_time_min..=((&netw_params).wait_time_min * 3)));

                let count_of_docs = get_docs_from_listing_page(
                    content,
                    &tx,
                    &listing_url_with_args,
                    section_name,
                    &mut already_retrieved_urls,
                    &client,
                    &netw_params,
                    data_folder.as_str(),
                    pdf_folder.as_str(),
                    "irdai",
                    "https://irdai.gov.in/"
                );
                counter += count_of_docs;
            }
        }
        info!("Completed retrieving {} document for plugin - irdai.", counter);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test2(){
        let cfg = Config::builder().build().unwrap();
        let netw_params = read_network_parameters(&cfg);
        let client = make_http_client(&netw_params);

        let sid = String::from("1");
        let ssid = String::from("7");
        let smid = String::from("0");
        let page_no = String::from("0");

        let response = get_sebi_urllist(&client,
            sid,
            ssid,
            smid,
            "Circulars",
            page_no
        );
        println!("{}", response);
        assert_eq!(1,1);
    }

}
