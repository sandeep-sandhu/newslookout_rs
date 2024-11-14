// file: utils.rs
// Purpose:

extern crate pdf_extract;
extern crate lopdf;

use std::string::String;
use std::collections;
use std::io::BufWriter;
use std::io::Write;
use std::path;
use std::fs::File;
use std::ops::Add;
use std::{any::Any, env};
use std::path::Path;
use log::{debug, error, info, LevelFilter, warn};
use log4rs::append::file::FileAppender;
use log4rs::append::rolling_file::policy::compound::CompoundPolicy;
use log4rs::append::rolling_file::policy::compound::roll::fixed_window::FixedWindowRoller;
use log4rs::append::rolling_file::policy::compound::trigger::size::SizeTrigger;
use log4rs::append::rolling_file::RollingFileAppender;
use log4rs::encode::pattern::PatternEncoder;
use log4rs::filter::threshold::ThresholdFilter;
use log4rs::config::{Appender, Root};

use config::{Config, Environment, FileFormat, Map, Value};
use config::builder::ConfigBuilder;
use chrono::{DateTime, Local, MappedLocalTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use rusqlite::{Row, Rows};
use rusqlite::params;
use scraper::{ElementRef};
use uuid::Uuid;

use crate::{document, utils};
use crate::document::{DocInfo, Document};


pub fn read_config(cfg_file: String) -> Config{
    let mut cfg_builder = Config:: builder();
    // cfg_builder = cfg_builder.set_default("default", "1");
    cfg_builder = cfg_builder.add_source(Environment::default().prefix("NEWSLOOKOUT_"));
    cfg_builder = cfg_builder.add_source(config::File::new(&cfg_file, FileFormat::Toml));
    // Add a default configuration file
    match cfg_builder.build() {
        Ok(config) => {
            // use your config
            return config;
        },
        Err(e) => {
            // something went wrong, return error to the calling function
            panic!("Error reading configuration - {}", e)
        }
    }
}

pub fn save_to_disk_as_json(received: &Document, json_file_path: &str) -> DocInfo {

    debug!("Writing document from url: {:?}", received.url);
    let mut docinfo_for_sql = DocInfo{
        plugin_name: received.module.clone(),
        url: received.url.clone(),
        pdf_url: received.pdf_url.clone(),
        title: received.title.clone(),
        unique_id: received.unique_id.clone(),
        publish_date_ms: received.publish_date_ms,
        filename: received.filename.clone(),
        section_name: received.section_name.clone(),
    };

    info!("Writing document to file: {}", received.filename);
    // serialize json to string
    match serde_json::to_string_pretty(&received){
        Ok(json_data) => {
            // persist to json
            match File::create(&json_file_path){
                Ok(mut file) => {
                    match file.write_all(json_data.as_bytes()) {
                        Ok(_write_res) => {
                            debug!("Wrote document from {}, titled '{}' to file: {:?}", received.url, received.title, json_file_path);
                            docinfo_for_sql.filename = received.filename.clone();
                            return docinfo_for_sql;
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
    return docinfo_for_sql;
}

/// Removes any unnecessary whitespaces from the string and returns the cleaned string
///
/// # Arguments
///
/// * `text`: The string to cleanup
///
/// returns: String
///
pub fn clean_text(text: String) -> String {
    let x: Vec<&str> = text.split_whitespace().collect();
    x.join(" ").trim().to_string()
}

/// Generates a unique filename from the document structure fields
///
/// # Arguments
///
/// * `doc_struct`: The document to use for generating the filename
///
/// returns: String
///
/// # Examples
///
/// let filename:String = make_unique_filename(mydoc);
///
pub fn make_unique_filename(doc_struct: &document::Document, extension: &str) -> String{
    match doc_struct.url.rfind('/') {
        Some(slash_pos_in_url) =>{
            let url_resname = (&doc_struct.url[(slash_pos_in_url+1)..])
                .replace(".html", "")
                .replace(".htm", "")
                .replace(".php", "")
                .replace(".aspx", "")
                .replace(".asp", "")
                .replace(".jsp", "");
            if url_resname.len() >1{
                return format!("{}_{}.{}", doc_struct.module, url_resname, extension);
            }else{
                return format!("{}_index.json", doc_struct.module);
            }
        }
        None => {
            info!("Could not get unique resource string from url: {}", doc_struct.url);
            match Uuid::parse_str(&doc_struct.url) {
                Ok(uuid_str) => {
                    return format!("{}_{}.json", doc_struct.module, uuid_str.to_string());
                },
                Err(e) => {
                    error!("Could not generate uuid from url: {}", e);
                    // add current timestamp:
                    let curr_timestamp = Utc::now().timestamp();
                    return format!("{}_{}_{}.json", doc_struct.module, doc_struct.publish_date_ms, curr_timestamp);
                }
            }
        }
    }
}

/// Gets all the texts inside an HTML element
pub fn get_text_from_element(elem: ElementRef) -> String {
    let mut output_string = String::new();
    for text in elem.text() {
        output_string = output_string.add(text);
    }
    output_string
}

/// Converts naive date to local date-timestamp
///
/// # Arguments
///
/// * `date`: The NaiveDate to convert
///
/// returns: DateTime<Local>
///
/// # Examples
///
/// ```
///
/// ```
pub fn to_local_datetime(date: NaiveDate) -> DateTime<Local> {
    let datetime = date.and_time(NaiveTime::default());
    match datetime.and_local_timezone(Local) {
        MappedLocalTime::Single(dt) => return dt,
        MappedLocalTime::Ambiguous(dt0, _dt1) => return dt0,
        MappedLocalTime::None => panic!("Invalid date, cannot convert to timestamp")
    }
}


/// Extract the plugin's parameters from its entry in the application's config file.
///
/// # Arguments
///
/// * `plugin_map`: The plugin map of all plugins
///
/// returns: (String, String, bool, isize)
pub fn extract_plugin_params(plugin_map: Map<String, Value>) -> (String, String, bool, isize) {
    let mut plugin_enabled: bool = false;
    let mut plugin_priority: isize = 99;
    let mut plugin_name = String::from("");
    let mut plugin_type = String::from("retriever");

    match plugin_map.get("name") {
        Some(name_str) => {
            plugin_name = name_str.to_string();
        },
        None => {
            error!("Unble to get plugin name from the config! Using default value of '{}'", plugin_name);
        }
    }
    match plugin_map.get("enabled") {
        Some(&ref enabled_str) => {
            match enabled_str.clone().into_bool(){
                Result::Ok(plugin_enabled_bool) => plugin_enabled = plugin_enabled_bool,
                Err(e) => error!("In config file, for plugin {}, fix the invalid value of plugin state, value should be either true or false: {}", plugin_name, e)
            }
        },
        None => {
            error!("Could not interpret whether enabled state is true or false for plugin {}", plugin_name)
        }
    }
    match plugin_map.get("type") {
        Some(plugin_type_str) => {
            plugin_type = plugin_type_str.to_string();
        }
        None => {
            error!("Invalid/missing plugin type in config, Using default value = '{}'",
                            plugin_type);
        }
    }
    match plugin_map.get("priority") {
        Some(&ref priority_str) => {
            match priority_str.clone().into_int(){
                Result::Ok(priority_int ) => plugin_priority = priority_int as isize,
                Err(e) => error!("In config file, for plugin {}, fix the priority value of plugin state; value should be positive integer: {}", plugin_name, e)
            }
        },
        None => {
            error!("Could not interpret priority for plugin {}", plugin_name)
        }
    }
    return (plugin_name, plugin_type, plugin_enabled, plugin_priority)
}


/// Get configuration parameters for network communication
///
/// # Arguments
///
/// * `config_clone`:
///
/// returns: (u64, u64, u64, String)
///
/// # Examples
///
/// ```
///
/// ```
pub fn get_network_params(config_clone: &Config) -> (u64, u64, u64, String, Option<String>) {
    let mut fetch_timeout_seconds: u64 = 20;
    let mut user_agent:String = String::from("Mozilla");
    let mut proxy_url:Option<String> = None;
    let retry_times: u64 = 3;
    let wait_time: u64 = 2;

    match config_clone.get_int("fetch_timeout") {
        Ok(config_timeout) =>{
            if config_timeout > 0{
                fetch_timeout_seconds = config_timeout.unsigned_abs();
            }
        },
        Err(ex) =>{
            info!("Using default timeout of {} due to error fetching timeout from config: {}",
                fetch_timeout_seconds,
                ex)
        }
    }

    match config_clone.get_string("user_agent") {
        Ok(user_agent_configured) => {
            user_agent.clear();
            user_agent.push_str(&user_agent_configured);
        },
        Err(e) => {
            error!("When extracting user agent from config: {:?}", e)
        }
    }

    match config_clone.get_string("proxy_server_url") {
        Ok(proxy_server_url) => {
            proxy_url = Some(proxy_server_url);
        },
        Err(e) => {
            info!("Could not identify proxy server url from config, not using proxy, error was: {:?}", e)
        }
    }

    return (fetch_timeout_seconds, retry_times, wait_time, user_agent, proxy_url);
}

pub fn get_data_folder(config: &Config) -> std::path::PathBuf {
    match config.get_string("data_dir") {
        Ok(dirname) => {
            let dirpath = std::path::Path::new(dirname.as_str());
            if std::path::Path::is_dir(dirpath){
                return dirpath.to_path_buf()
            }
        },
        Err(e) => error!("When getting data folder name: {}", e)
    }
    // return present working directory
    let path_currdir = env::current_dir().expect("give proper argument");
    return path_currdir;
}

pub fn get_database_filename(config: &Config) -> String {
    match config.get_string("completed_urls_datafile") {
        Ok(dirname) => return dirname,
        Err(e) => error!("When getting database filename: {}", e)
    }
    return "newslookout_urls.db".to_string();
}

/// Get already retrieved URLs from the database for the given plugin/module
///
/// # Arguments
///
/// * `config`: The application's configuration
/// * `module_name`: The name of the plugin/module
///
/// returns: HashSet<String, RandomState>
pub fn get_urls_from_database(sqlite_filename: &str, module_name: &str) -> collections::HashSet<String> {
    let mut urls_already_retrieved: collections::HashSet<String> = collections::HashSet::new();
        match rusqlite::Connection::open(sqlite_filename) {
            Ok(conn) => {
                // filter and return results for specified module only:
                let sql_string = format!("select distinct url from completed_urls where plugin = '{}'", module_name);
                match conn.prepare(sql_string.as_str()){
                    Result::Ok(mut sql_stmt) =>{
                        match sql_stmt.query([]){
                            Result::Ok(mut result_rows) => {
                                while let Ok(Some(row)) = result_rows.next() {
                                    match row.get(0){
                                        Result::Ok(first_col) => {
                                            urls_already_retrieved.insert(first_col);
                                        },
                                        Err(e) => error!("Could not get first column from result row: {}", e)
                                    }
                                }
                            },
                            Err(e) => error!("When running the query to get urls: {}", e)
                        };
                    },
                    Err(e) => error!("When preparing SQL statement to get URLs already retrieved: {}", e)
                }
            },
            Err(er) => error!("When opening connection to database {}: {}", sqlite_filename, er),
        }
    return urls_already_retrieved;
}

/// Insert all urls retrieved in this session into the database table.
///
/// # Arguments
///
/// * `config`: The application configuration
/// * `processed_docinfos`: The DocInfo struct that contains the url and corresponding details
///
/// returns: u64
pub fn insert_urls_info_to_database(config: &config::Config, processed_docinfos: &Vec<document::DocInfo>) -> u64 {
    // get the database filename form config file, or else create one in present directory "newslookout_urls.db":
    let mut database_fullpath = String::from("newslookout_urls.db");
    match config.get_string("completed_urls_datafile") {
        Ok(sqlite_filename) => {
            database_fullpath = sqlite_filename;
        },
        Err(e) => error!("Unable to get database details from configuration file, so using default value, error: {}", e)
    }
    //open connection and write to table:
    match rusqlite::Connection::open(database_fullpath) {
        Result::Ok(conn) =>{
            let _ = conn.execute("CREATE TABLE IF NOT EXISTS completed_urls (
                                                url varchar(256) not null primary key,
                                                plugin varchar(50) not null,
                                                pubdate varchar(10),
                                                section_name varchar(100),
                                                title varchar(100),
                                                unique_id varchar(256),
                                                filename varchar(256)
                                             )",
                                 []);
            let mut counter: u64 = 0;
            debug!("Started writing urls into table.");
            for doc_info in processed_docinfos {
                let mut pubdate_yyyymmdd = String::from("1970-01-01");
                match DateTime::from_timestamp(doc_info.publish_date_ms, 0){
                    Some(pub_datetime) => pubdate_yyyymmdd = format!("{}", pub_datetime.format("%Y-%m-%d")),
                    None => error!("Invalid timestamp {} given for published date of url {}, using default", doc_info.publish_date_ms, doc_info.url)
                }
                match conn.execute(
                    "INSERT INTO completed_urls (url, plugin, pubdate, section_name, title, unique_id, filename) VALUES (?, ?, ?, ?, ?, ?, ?)",
                    [doc_info.url.as_str(), doc_info.plugin_name.as_str(), pubdate_yyyymmdd.as_str(), doc_info.section_name.as_str(), doc_info.title.as_str(), doc_info.unique_id.as_str(), doc_info.filename.as_str()],
                ){
                    Result::Ok(_) => counter += 1,
                    Err(e) => error!("When inserting new url {} to table: {}", doc_info.url, e)
                }
            }
            info!("Closing database connection after writing {} urls to table.", counter);
            let _ = conn.close();
            // return count of records inserted
            return counter;
        },
        Err(conn_err) => {
            error!("When writing to database: {}", conn_err);
        }
    }
    return 0;
}

pub fn extract_text_from_html(html_content: &str) -> String{
    let html_root_elem = scraper::html::Html::parse_document(html_content);
    // TODO: apply text density calculations, and
    // position based heuristics to identify relevant content to extract
    return get_text_from_element(html_root_elem.root_element());
}

fn check_valid_word(word: &str, alpha_pattn: &regex::Regex) -> bool {
    // ignore if alphanumeric or numeric or punctuations
    return alpha_pattn.is_match(word) & (word.len()>0)
}

pub fn word_count(text_str: &str) -> usize{
    let mut counter: usize = 0;
    if let Ok(alpha_pattn) = regex::Regex::new("[A-Za-z]"){
        for word in text_str.replace("\n", " ").split_whitespace(){
            // TODO: use memoization here
            if check_valid_word(word, &alpha_pattn) {
                counter += 1;
            }
        }
    }
    return counter;
}

pub fn get_last_n_words(text_str: &str, count_n:usize) -> String {
    // TODO: fix this since data is being changed (avoid removing newlines)
    let last_n_words_rev: Vec<&str> = text_str.split_whitespace().rev().take(count_n).collect();
    if count_n > last_n_words_rev.len(){
        info!("get_last_n_words: extracted only {} words, it is less than required {} words.", last_n_words_rev.len(), count_n);
    }
    return last_n_words_rev.into_iter().rev().collect::<Vec<&str>>().join(" ");
}

pub fn split_by_word_count(text: &str, max_words_per_split: usize, previous_overlap: usize) -> Vec<String> {

    let mut buffer_wc:usize = 0;
    let mut buffer = String::new();
    let mut overlap_buffer = String::from("");
    let mut overlap_buffer_wc:usize = 0;
    let mut previous_overlap_text = String::from("");

    // initial split by double lines:
    let text_parts_stage1 = text.split("\n\n");

    let mut text_parts_stage2: Vec<String> = Vec::new();
    // merge these split parts based on word count limit:
    for text_block in text_parts_stage1 {
        let this_blocks_word_count:usize = word_count(*&text_block);
        if (buffer_wc + previous_overlap >= max_words_per_split) &
            (this_blocks_word_count + buffer_wc < max_words_per_split) {
            // if so, then start keeping this and following blocks as overlap
            overlap_buffer = format!("{} {}", overlap_buffer, text_block);
            overlap_buffer_wc += this_blocks_word_count;
        }
        // check if buffer_wc + this_wc > max_words, if so put buffer in vector
        if (this_blocks_word_count + buffer_wc) > max_words_per_split {
            // add the buffer to vector
            text_parts_stage2.push(buffer);
            // empty buffer and add current text block to buffer:
            // buffer.clear(); // not required as its implicit
            // add overlap_buffer
            if overlap_buffer_wc > 0{
                buffer = format!("{} {} {}", overlap_buffer, previous_overlap_text, text_block);
                buffer_wc = overlap_buffer_wc + word_count(previous_overlap_text.as_str()) + this_blocks_word_count;
                overlap_buffer.clear();
                overlap_buffer_wc = 0;
            }else{
                buffer = format!("{} {}", previous_overlap_text, text_block);
                buffer_wc = word_count(previous_overlap_text.as_str()) + this_blocks_word_count;
            }
        } else{
            // append current text block to buffer:
            if buffer_wc > 0 {
                buffer = format!("{}\n\n{}", buffer, text_block);
                // increment buffer word count:
                buffer_wc += this_blocks_word_count;
            }else{
                // for first iteration where buffer_wc = 0
                // add overlap_buffer
                if overlap_buffer_wc > 0 {
                    buffer = format!("{} {} {}", overlap_buffer, previous_overlap_text, text_block);
                    // increment buffer word count:
                    buffer_wc = buffer_wc + overlap_buffer_wc + word_count(previous_overlap_text.as_str()) + this_blocks_word_count;
                    overlap_buffer.clear();
                    overlap_buffer_wc = 0;
                }else {
                    buffer = format!("{} {}", previous_overlap_text, text_block);
                    // increment buffer word count:
                    buffer_wc = buffer_wc + word_count(previous_overlap_text.as_str()) + this_blocks_word_count;
                }
            }
        }
        if previous_overlap > 0 {
            if this_blocks_word_count < previous_overlap{
                previous_overlap_text = text_block.to_string();
            }else {
                previous_overlap_text = get_last_n_words(text_block, previous_overlap);
            }
        };
    }
    // add remainder into array of text parts:
    if buffer_wc > 0 {
        // TODO: add remainder into previously added part in vector: text_parts_stage2
        append_with_last_element(&mut text_parts_stage2, buffer);
        // add as the last element of array
        // text_parts_stage2.push(buffer);
    }

    return text_parts_stage2;
}

/// Appends the given string to the last element of a vector of strings.
///
/// # Arguments
///
/// * `stringvec`: The vector to be appended to.
/// * `text_to_append`: The text to append to the last element
///
/// returns: &mut Vec<String, Global>
fn append_with_last_element(stringvec: &mut Vec<String>, text_to_append: String) {
    // append text into previously added part in vector
    let last_location = stringvec.len();
    if let Some(lastelem) = stringvec.last(){
        let replacement = format!("{} {}", lastelem, text_to_append);
        let _old = std::mem::replace(&mut stringvec[last_location-1], replacement);
    }
    if last_location == 0{
        stringvec.push(text_to_append);
    }
    //return stringvec;
}

/// Retrieve the queried parameter from this plugin's configuration
///
/// # Arguments
///
/// * `app_config`: The config loaded from the application's config file.
/// * `plugin_name`: The name of this plugin
/// * `param_key`: The parameter to be queried
///
/// returns: Option<String>
pub fn get_plugin_config(app_config: &Config, plugin_name: &str, param_key: &str) -> Option<String> {
    match app_config.get_array("plugins"){
        Result::Ok(plugins) =>{
            for plugin in plugins {
                match plugin.into_table(){
                    Ok(plugin_map ) => {
                        match plugin_map.get("name") {
                            Some(name_val) =>{
                                if name_val.to_string().eq(plugin_name) {
                                    // get the param for given key from this plugin_map:
                                    match plugin_map.get(param_key) {
                                        Some(param_val) => {
                                            return Some(param_val.to_string());
                                        },
                                        None =>{
                                            error!("When retrieving value for key {}", param_key);
                                            return None;
                                        }
                                    }
                                }
                            },
                            None => {
                                error!("When extracting name parameter of plugin.");
                            }
                        }
                    },
                    Err(e) => {
                        error!("When getting individual plugin config: {}", e);
                        return None;
                    }
                }
            }
        },
        Err(e)=> {
            error!("When retrieving plugins config for all plugins: {}", e);
            return None;
        }
    }
    return None;
}

/// Get user context (prompts) from application configuration.
///
/// # Arguments
///
/// * `app_config`:
///
/// returns: (String, String, String, String)
pub fn get_contexts_from_config(app_config: &Config) -> (String, String, String, String){

    let mut summary_part_context: String = String::from("Summarise the following text concisely.\n\nTEXT:\n");
    match app_config.get_string("summary_part_context") {
        Ok(param_val_str) => summary_part_context = param_val_str,
        Err(e) => error!("Could not load parameter 'summary_part_context' from config file, using default, error: {}", e)
    }

    let mut insights_part_context: String = String::from("Read the following text and extract actions from it.\n\nTEXT:\n");
    match app_config.get_string("insights_part_context") {
        Ok(param_val_str) => insights_part_context = param_val_str,
        Err(e) => error!("Could not load parameter 'insights_part_context' from config file, using default, error: {}", e)
    }

    let mut summary_exec_context: String = String::from("Summarise the following text concisely.\n\nTEXT:\n");
    match app_config.get_string("summary_exec_context") {
        Ok(param_val_str) => summary_exec_context = param_val_str,
        Err(e) => error!("Could not load parameter 'summary_exec_context' from config file, using default, error: {}", e)
    }

    let mut system_context: String = String::from("You are an expert in analysing news and documents.");
    match app_config.get_string("system_context") {
        Ok(param_val_str) => system_context = param_val_str,
        Err(e) => error!("Could not load parameter 'system_context' from config file, using default, error: {}", e)
    }

    return (summary_part_context, insights_part_context, summary_exec_context, system_context);
}


pub fn build_llm_prompt(model_name: &str, system_context: &str, user_context: &str, input_text: &str) -> String {
    if model_name.contains("llama") {
        return prepare_llama_prompt(system_context, user_context, input_text);
    } else if model_name.contains("gemma") {
        return prepare_gemma_prompt(system_context, user_context, input_text);
    }
    else {
        return format!("{}\n{}\n{}", system_context, user_context, input_text).to_string();
    }
}

pub fn prepare_gemma_prompt(system_context: &str, user_context: &str, input_text: &str) -> String{
    return format!("<start_of_turn>user\
        {}\
        \
        {}<end_of_turn><start_of_turn>model", user_context, input_text).to_string();
}

pub fn prepare_llama_prompt(system_context: &str, user_context: &str, input_text: &str) -> String {
    return format!("<|begin_of_text|><|start_header_id|>system<|end_header_id|>{}\
        <|eot_id|><|start_header_id|>user<|end_header_id|>{}\
        \n\n{}<|eot_id|> <|start_header_id|>assistant<|end_header_id|>", system_context, user_context, input_text).to_string();
}


// Description of Tests:
// These unit tests verify the functions in this module.
#[cfg(test)]
mod tests {
    use crate::{document, utils};
    use crate::utils::{append_with_last_element, get_last_n_words, make_unique_filename};

    #[test]
    fn test_to_local_datetime() {
        assert_eq!(1,1);
    }

    #[test]
    fn test_get_text_from_element() {
        assert_eq!(1,1);
    }

    #[test]
    fn test_clean_text() {
        assert_eq!(1,1);
    }

    #[test]
    fn test_get_data_folder() {
        assert_eq!(1,1);
    }

    #[test]
    fn test_word_count(){
        let test_para = "The quick brown fox jumped over the 1 lazy dog.";
        assert_eq!(utils::word_count(test_para), 9, "Wrong no of words counted.");
    }

    #[test]
    fn test_split_by_word_count_with_overlap(){
        let para2 = "The\n\n quick\n\n brown\n\n fox\n\n jumped\n\n over\n\n the\n\n 1\n\n lazy\n\n dog.\n\n";
        let para2_expected_answer = vec![" The\n\n quick\n\n brown", "brown  fox\n\n jumped", "jumped  over\n\n the\n\n 1", " 1  lazy\n\n dog.\n\n"];
        let result2 = utils::split_by_word_count(para2, 3, 1);
        println!("Word split result = {:?}", result2);
        assert_eq!(result2, para2_expected_answer, "Did not split text into parts by word limit and overlap");
    }

    #[test]
    fn test_split_by_word_count_with_overlap_long(){
        let para3 = "one \n\n two \n\n three \n\n four \n\n five \n\n six \n\n seven \n\n eight \n\n nine \n\n ten \n\n eleven \n\n twelve \n\n thirteen \n\n fourteen \n\n fifteen \n\n";
        // test for overlap inclusion:
        let para3_expected_answer = vec![" one \n\n two \n\n three \n\n four \n\n five ", "  four   five   six \n\n seven \n\n eight ", "  seven   eight   nine \n\n ten \n\n eleven ", "  ten   eleven   twelve \n\n thirteen \n\n fourteen ", "  thirteen   fourteen   fifteen \n\n"];
        let result3 = utils::split_by_word_count(para3, 5, 2);
        println!("Word split result = {:?}", result3);
        assert_eq!(result3, para3_expected_answer, "Did not split text into parts correctly by word limit and overlap");
    }

    #[test]
    fn test_get_last_n_words(){
        let para1 = "The\n\n quick\n\n brown\n\n fox\n\n jumped\n\n over\n\n the\n\n 1\n\n lazy\n\n dog.\n\n";
        let para1_expected_answer = "lazy dog.";
        assert_eq!(get_last_n_words(para1,2), para1_expected_answer, "Did not get last n words");
        let para2 = "The\n\n quick\n\n brown\n\n fox\n\n jumped\n\n over\n\n the\n\n 1\n\n lazy\n\n dog.\n\n";
        let para2_expected_answer = "The quick brown fox jumped over the 1 lazy dog.";
        assert_eq!(get_last_n_words(para2,12), para2_expected_answer, "Did not get last n words");
    }

    #[test]
    fn test_append_with_last_element(){
        let mut example_1_vec = vec![String::from("first"), String::from("second"), String::from("third")];
        let example_1_toadd = String::from("to append");
        let example_1_expected = vec![String::from("first"), String::from("second"), String::from("third to append")];
        append_with_last_element(&mut example_1_vec, example_1_toadd);

        assert_eq!(
            example_1_vec.last(),
            example_1_expected.last(),
            "New text could not be appended correctly to last element."
        );

        assert_eq!(
            example_1_vec.len(),
            example_1_expected.len(),
            "Vector size increased when appending to last element"
        );
    }

    #[test]
    fn test_append_with_last_element_blank_vector(){
        let mut example_1_vec: Vec<String> = Vec::new();
        let example_1_toadd = String::from("to append");
        let example_1_expected = vec![String::from("to append")];
        append_with_last_element(&mut example_1_vec, example_1_toadd);

        assert_eq!(
            example_1_vec.last(),
            example_1_expected.last(),
            "New text could not be appended correctly to last element."
        );

        assert_eq!(
            example_1_vec.len(),
            example_1_expected.len(),
            "Vector size increased when appending to last element"
        );
    }

    #[test]
    fn test_make_unique_filename(){
        let mut example1 = document::new_document();
        example1.filename = "one_value".to_string();
        example1.url = "https://www.www.com/sub/page-name.html".to_string();
        example1.module = "mod_dummy".to_string();
        example1.publish_date_ms = 1010;
        let example1_result = make_unique_filename(&example1, "json");
        println!("output filename: {}", example1_result);
        assert_eq!(example1_result, "mod_dummy_page-name.json".to_string(), "Unable to set the filename correctly.")
    }
}