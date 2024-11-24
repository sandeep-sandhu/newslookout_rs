// file: utils.rs
// Purpose:

extern crate pdf_extract;
extern crate lopdf;

use std::string::String;
use std::{collections, fs};
use std::io::BufWriter;
use std::io::Write;
use std::path;
use std::fs::File;
use std::ops::Add;
use std::{any::Any, env};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::borrow::BorrowMut;
use std::collections::HashMap;
use nom::AsBytes;
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
use pdf_extract::extract_text_from_mem;
use regex::Regex;
use rusqlite::{Row, Rows};
use rusqlite::params;
use scraper::{ElementRef};

use crate::{document, network, utils};
use crate::document::{Document};


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

pub fn save_to_disk_as_json(received: &Document, json_file_path: &str) {

    debug!("Writing document from url: {:?}", received.url);

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

/// Generates a unique filename from the document structure fields.
/// Start building the filename with module and section name.
/// Then append only last 64 characters or url resource after stripping out special charcters.
/// To this, calculate and append the hash value of the url, then append publish date to get
/// the unique filename
///
/// # Arguments
///
/// * `doc_struct`: The document to use for generating the filename
/// * `extension` : The filename extension
///
/// returns: String
///
/// # Examples
///
/// let filename:String = make_unique_filename(mydoc, "json");
///
pub fn make_unique_filename(doc_struct: &document::Document, extension: &str) -> String{
    // limit name to given characters in length:
    let max_characters: usize = 64;
    let mut filename_prefix = format!("{}_{}_", doc_struct.module, doc_struct.section_name);

    let mut hasher = std::hash::DefaultHasher::new();
    doc_struct.url.hash(&mut hasher);
    let mut unique_string = hasher.finish().to_string();
    unique_string.push_str("_");
    unique_string.push_str(doc_struct.publish_date.as_str());

    match doc_struct.url.rfind('/') {
        Some(slash_pos_in_url) =>{
            let mut url_resname = (&doc_struct.url[(slash_pos_in_url+1)..])
                .replace(".html", "")
                .replace(".htm", "")
                .replace(".php", "")
                .replace(".aspx", "")
                .replace(".asp", "")
                .replace(".jsp", "")
                .replace("/", "")
                .replace("\\", "")
                .replace("?", "_")
                .replace("*", "")
                .replace(":", "")
                .replace("%20", "_")
                .replace("%E2%82%B9", "")
                .replace("+", "_")
                .replace("'", "")
                .replace("–", "_")
                .replace("__", "_");
            url_resname = url_resname.chars().rev().take(max_characters).collect();
            url_resname = url_resname.chars().rev().collect();
            filename_prefix.push_str(url_resname.as_str());
        }
        None => {
            info!("Could not get unique resource string from url: {}", doc_struct.url);
        }
    }
    return format!("{}_{}.{}", filename_prefix, unique_string.to_string(), extension);
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



pub fn get_files_listing_from_dir(data_folder_name: &str, file_extension: &str) -> Vec<PathBuf> {
    let mut all_file_paths: Vec<PathBuf> = Vec::new();
    // get json file listing from data folder
    match fs::read_dir(data_folder_name) {
        Err(e) => error!("When getting list of all {} files in data folder {}, error:{}",
            file_extension, data_folder_name, e),
        Ok(dir_entries) => {
            all_file_paths = dir_entries // Filter out all those directory entries which couldn't be read
                .filter_map(|res| res.ok())
                // Map the directory entries to paths
                .map(|dir_entry| dir_entry.path())
                // Filter out all paths with extensions other than `json`
                .filter_map(|path| {
                    if path.extension().map_or(false, |ext| ext == file_extension) {
                        Some(path)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
        }
    }
    return all_file_paths;
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
pub fn insert_urls_info_to_database(config: &config::Config, processed_docinfos: &[Document]) -> usize {
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
            let mut counter: usize = 0;
            debug!("Started writing urls into table.");
            for doc_info in processed_docinfos {
                let mut pubdate_yyyymmdd = String::from("1970-01-01");
                match DateTime::from_timestamp(doc_info.publish_date_ms, 0){
                    Some(pub_datetime) => pubdate_yyyymmdd = format!("{}", pub_datetime.format("%Y-%m-%d")),
                    None => error!("Invalid timestamp {} given for published date of url {}, using default", doc_info.publish_date_ms, doc_info.url)
                }
                match conn.execute(
                    "INSERT INTO completed_urls (url, plugin, pubdate, section_name, title, unique_id, filename) VALUES (?, ?, ?, ?, ?, ?, ?)",
                    [doc_info.url.as_str(), doc_info.module.as_str(), pubdate_yyyymmdd.as_str(), doc_info.section_name.as_str(), doc_info.title.as_str(), doc_info.unique_id.as_str(), doc_info.filename.as_str()],
                ){
                    Result::Ok(_) => counter += 1,
                    Err(e) => error!("When inserting new url {} to table: {}", doc_info.url, e)
                }
            }
            debug!("Closing database connection after writing {} urls to table.", counter);
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


/// Extract content of the associated PDF file mentioned in the document's pdf_url attribute
/// Converts to text and saves it to the document 'text' attribute.
/// The PDF content is saved as a file in the data_folder.
///
/// # Arguments
///
/// * `input_doc`: The document to read
/// * `client`: The HTTP(S) client for fectching the web content via GET requests
/// * `data_folder`: The data folder where the PDF files are to be saved.
///
/// returns: ()
pub fn load_pdf_content(
    mut input_doc: &mut Document,
    client: &reqwest::blocking::Client,
    data_folder: &str)
{
    if input_doc.pdf_url.len() > 4 {
        let pdf_filename = make_unique_filename(&input_doc, "pdf");
        // save to file in data_folder, make full path by joining folder to unique filename
        let pdf_file_path = Path::new(data_folder).join(&pdf_filename);
        // check if pdf already exists, if so, do not retrieve again:
        if Path::exists(pdf_file_path.as_path()){
            info!("Not retrieving PDF since it already exists: {:?}", pdf_file_path);
            if input_doc.text.len() > 1 {
                let txt_filename = make_unique_filename(&input_doc, "txt");
                let txt_file_path = Path::new(data_folder).join(&txt_filename);
                input_doc.text = extract_text_from_pdf(pdf_file_path, txt_file_path);
            }
        }else {
            // get pdf content, and its plaintext output
            let (pdf_data, plaintext) = retrieve_pdf_content(&input_doc.pdf_url, client);
            input_doc.text = plaintext;
            debug!("From url {}: retrieved pdf file from link: {} of length {} bytes",
                input_doc.url, input_doc.pdf_url, pdf_data.len()
            );
            if pdf_data.len() > 1 {
                // persist to disk
                match File::create(&pdf_file_path) {
                    Ok(mut pdf_file) => {
                        debug!("Created pdf file: {:?}, now starting to write data for: '{}' ",
                            pdf_file_path, input_doc.title
                        );
                        match pdf_file.write_all(pdf_data.as_bytes()) {
                            Ok(_write_res) => info!("From url {} wrote {} bytes to file: {}",
                                input_doc.pdf_url, pdf_data.len(),
                                pdf_file_path.as_os_str().to_str().unwrap()),
                            Err(write_err) => error!("When writing PDF file to disk: {}",
                                write_err)
                        }
                    },
                    Err(file_err) => {
                        error!("When creating pdf file: {}", file_err);
                    }
                }
            }
        }
    }
}

pub fn extract_text_from_pdf(pdf_file_path: PathBuf, txt_file_path: PathBuf) -> String {
    // read PDF data from file:
    // create a text file to hold output from pdf extraction:
    let mut output_file = BufWriter::new(File::create(&txt_file_path)
        .expect("could not create text file"));
    // prepare buffer for text
    let mut output = pdf_extract::PlainTextOutput::new(
        &mut output_file as &mut dyn std::io::Write);
    // load the pdf file
    let doc = pdf_extract::Document::load(pdf_file_path).unwrap();
    debug!("Converting pdf to text file: {:?}", txt_file_path);
    // extract the text:
    pdf_extract::output_doc(&doc, output.borrow_mut())
        .expect("Could not convert to text file");
    let plaintext = fs::read_to_string(&txt_file_path)
        .expect("Could not read text from pdf.");
    match fs::remove_file(txt_file_path){
        Ok(_) => {}
        Err(e) => {error!("When deleting txt file extract from pdf: {}", e)}
    }
    return plaintext;
}

pub fn retrieve_pdf_content(pdf_url: &str, client: &reqwest::blocking::Client) -> (bytes::Bytes, String) {
    let pdf_data = network::http_get_binary(&pdf_url.to_string(), client);
    // convert to text, populate text field
    match extract_text_from_mem(pdf_data.as_bytes()) {
        Result::Ok(plaintext) => {
            return (pdf_data, plaintext);
        },
        Err(outerr) => {
            error!("When converting pdf content into text: {}", outerr);
        }
    }
    return (pdf_data, String::new());
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

pub fn split_by_regex(input_text: String, regex_pattn: Regex) -> Vec<String> {

    let mut text_parts_stage1: Vec<String> = Vec::new();

    // initialise previous match start to beginning of text
    let mut previous_start_idx: usize = 0;
    let mut previous_end_idx: usize = 0;
    let mut curr_match_start: usize = 0;

    // search for pattern in text beginning from the end of previous matching pattern:
    for item in regex_pattn.find_iter(&input_text[previous_end_idx..]){
        let range = item.range();
        // calculate absolute index of current match start:
        curr_match_start = range.start;
        debug!("curr_match_start = {}, item={:?}", curr_match_start, item);
        // append to collection, the part from previous match start till this current match start
        text_parts_stage1.push(input_text[previous_start_idx..curr_match_start].to_string());
        // update previous match starts and ends for next iteration:
        previous_start_idx = curr_match_start;
        previous_end_idx = range.end;
    }
    text_parts_stage1.push(input_text[previous_start_idx..].to_string());

    return text_parts_stage1;
}

pub fn split_by_word_count(text: &str, max_words_per_split: usize, previous_overlap: usize, some_regex: Option<Regex>) -> Vec<String> {

    let mut buffer_wc:usize = 0;
    let mut buffer = String::new();
    let mut overlap_buffer = String::from("");
    let mut overlap_buffer_wc:usize = 0;
    let mut previous_overlap_text = String::from("");
    let mut text_parts_stage1: Vec<String> = text.split("\n\n").map(|x| x.to_string() ).collect::<Vec<String>>();

    //if some_regex is not None, then:
    if let Some(initial_split_regex) = some_regex {
        // first split by regex:
        let text_parts_init: Vec<String> = split_by_regex(text.to_string(), initial_split_regex);
        debug!("Regex initial split into #{} parts", text_parts_init.len());
        // then split by double lines, those parts that are prceeding regex:
        let mut flag_is_first = true;
        for part in text_parts_init{
            if flag_is_first == true {
                // split only the first part by double lines, put into vector:
                debug!("___Splitting first part: {}", part);
                let splits = part.split("\n\n").map(|x| x.to_string() ).collect::<Vec<String>>();
                text_parts_stage1 = splits.clone();
                debug!("Resulting splits: {:?}", text_parts_stage1);
                flag_is_first = false;
            }else{
                // all other parts, append to end of vector
                debug!("___Adding remaining parts: {}", part);
                text_parts_stage1.push(part);
            }
        }
    }

    let mut text_parts_stage2: Vec<String> = Vec::new();
    // merge these split parts based on word count limit:
    for text_block in text_parts_stage1 {
        let this_blocks_word_count:usize = word_count(text_block.as_str());
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
                previous_overlap_text = get_last_n_words(text_block.as_str(), previous_overlap);
            }
        };
    }
    // add remainder into array of text parts:
    if buffer_wc > 0 {
        // add remainder into previously added part in vector: text_parts_stage2
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

pub fn get_text_using_ocr(){
    // TODO: extract text from image:

    // let mut tesseract_args = rusty_tesseract::Args {
    //     lang: "eng".to_string(),
    //
    //     //map of config variables
    //     //this example shows a whitelist for the normal alphabet. Multiple arguments are allowed.
    //     //available arguments can be found by running 'rusty_tesseract::get_tesseract_config_parameters()'
    //     config_variables: HashMap::from([(
    //         "tessedit_char_whitelist".into(),
    //         "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ".into(),
    //     )]),
    //     dpi: Some(150),       // specify DPI for input image
    //     psm: Some(6),         // define page segmentation mode 6 (i.e. "Assume a single uniform block of text")
    //     oem: Some(3),         // define optical character recognition mode 3 (i.e. "Default, based on what is available")
    // };
}

pub fn check_and_fix_url(url_to_check: &str, base_url: &str) -> Option<String> {
    const ROOT_FOLDER: &str = "/";
    const JAVASCRIPT_CODE: &str = "javascript:";

    // check if url starts with base_url, then all is ok
    if url_to_check.starts_with(base_url) {
        return Some(url_to_check.to_string());
    }
    else if url_to_check.starts_with(ROOT_FOLDER) {

        // else, if relative url starts with root folder, then replace root folder with base url
        let mut full_url = String::from(base_url);

        // remove root folder part and add remaining part of url string to the base_url:
        full_url.push_str(&url_to_check[ROOT_FOLDER.len()..]);

        return Some(full_url.to_string());
    }

    // reject javascript code links:
    if url_to_check.starts_with(JAVASCRIPT_CODE) {
        return None;
    }

    // default is to return None if none of these conditions match
    return None;
}

// Description of Tests:
// These unit tests verify the functions in this module.
#[cfg(test)]
mod tests {
    use regex::Regex;
    use crate::{document, utils};
    use crate::utils::{append_with_last_element, check_and_fix_url, get_last_n_words, make_unique_filename, split_by_regex};

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
        let para2_expected_answer = vec![" The\n\n quick\n\n brown", "brown  fox\n\n jumped", "jumped  over\n\n the\n\n 1  1  lazy\n\n dog.\n\n"];
        let result2 = utils::split_by_word_count(para2, 3, 1, None);
        // debug!("Word split result = {:?}", result2);
        assert_eq!(result2, para2_expected_answer, "Did not split text into parts by word limit and overlap");
    }

    #[test]
    fn test_split_by_word_count_with_overlap_long(){
        let para3 = "one \n\n two \n\n three \n\n four \n\n five \n\n six \n\n seven \n\n eight \n\n nine \n\n ten \n\n eleven \n\n twelve \n\n thirteen \n\n fourteen \n\n fifteen \n\n";
        // test for overlap inclusion:
        let para3_expected_answer = vec![" one \n\n two \n\n three \n\n four \n\n five ", "  four   five   six \n\n seven \n\n eight ", "  seven   eight   nine \n\n ten \n\n eleven ", "  ten   eleven   twelve \n\n thirteen \n\n fourteen    thirteen   fourteen   fifteen \n\n"];
        let result3 = utils::split_by_word_count(para3, 5, 2, None);
        // debug!("Word split result = {:?}", result3);
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
        example1.filename = "master-direction-on-currency-distribution-amp-exchange-scheme-cdes-for-bank-branches-including-currency-chests-based-on-performance-in-rendering-customer-service-to-members-of-public-12055".to_string();
        example1.url = "https://website.rbi.org.in/web/rbi/-/notifications/master-direction-on-currency-distribution-amp-exchange-scheme-cdes-for-bank-branches-including-currency-chests-based-on-performance-in-rendering-customer-service-to-members-of-public-12055".to_string();
        example1.module = "mod_dummy".to_string();
        example1.section_name = "some_section".to_string();
        example1.publish_date_ms = 1010;
        let example1_expected_filename = "mod_dummy_some_section_ormance-in-rendering-customer-service-to-members-of-public-12055_1400996662519724217_1970-01-01.json";
        let example1_result = make_unique_filename(&example1, "json");
        // debug!("output filename: {}", example1_result);
        assert_eq!(example1_result, example1_expected_filename.to_string(), "Unable to set the filename correctly.")
    }

    #[test]
    fn test_split_be_regex(){
        let example1 = "one two three\n \nAnnex 2\nfour five\nsix\n \nAppendix C\nseven\neight nine ten\n eleven".to_string();
        //                                 ^12        ^21               ^36          ^47
        let regexpattn = Regex::new(
            r"(\n \nAnnex[ure]* |\n \nAppendix )"
        ).expect("A valid regex for annexures");
        let resultvec = split_by_regex(example1, regexpattn);
        let expected_result1 = vec![
            "one two three".to_string(),
            "\n \nAnnex 2\nfour five\nsix".to_string(),
            "\n \nAppendix C\nseven\neight nine ten\n eleven".to_string()
        ];
        assert_eq!(resultvec[0],expected_result1[0]);
        assert_eq!(resultvec[1],expected_result1[1]);
        assert_eq!(resultvec[2],expected_result1[2]);
    }

    #[test]
    fn test_check_and_fix_url(){
        let example1 = "javascript:runfunction();";
        let example2 = "/one/relative/url";
        let example3 = "another/relative/url";
        let example4 = "https://website.rbi.org.in/valid/url";
        let base_url = "https://website.rbi.org.in/";
        assert_eq!(check_and_fix_url(example1, base_url),
                   None,
                   "Could not detect javascript code link.");
        assert_eq!(check_and_fix_url(example2, base_url),
                   Some("https://website.rbi.org.in/one/relative/url".to_string()),
                   "Could not fix relative url starting from root.");
        assert_eq!(check_and_fix_url(example3, base_url),
                   None,
                   "Could not detect relative url not starting at root.");
        assert_eq!(check_and_fix_url(example4, base_url),
                   Some("https://website.rbi.org.in/valid/url".to_string()),
                   "Could not detect valid url.");
    }

}