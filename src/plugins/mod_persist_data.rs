use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use config::Config;
use log::{debug, error, info, warn};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;
use crate::document;
use crate::get_plugin_cfg;

pub const PLUGIN_NAME: &str = "mod_persist_data";


pub(crate) fn process_data(tx: Sender<document::Document>, rx: Receiver<document::Document>, app_config: &Config, _api_mutexes: &mut HashMap<String, Arc<Mutex<isize>>>){

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
                // Save CSV-like market data to SQLite before writing to zip
                save_market_data_to_sqlite(&doc, app_config);
                let updated_doc = save_to_zip(doc, data_folder_name.as_str());
                match tx.send(updated_doc) {
                    Result::Ok(_) => { counter += 1; },
                    Err(e) => error!("{}: When sending processed doc via tx: {}", PLUGIN_NAME, e)
                }
            },
            "database" => {
                debug!("Writing document to database.");
                let updated_doc = write_to_database(doc, app_config);
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

/// Generates a globally-unique entry filename stem: {module}_{url_hash:016x}
/// Uses only the URL hash so entries never contain article subject or section names.
fn make_entry_stem(doc: &document::Document) -> String {
    let mut hasher = std::hash::DefaultHasher::new();
    doc.url.hash(&mut hasher);
    let url_hash = hasher.finish();
    format!("{}_{:016x}", doc.module, url_hash)
}

/// Collects all entry names already present in the zip at `zip_path`.
/// Returns an empty set if the file does not exist or cannot be opened.
fn existing_zip_entries(zip_path: &Path) -> HashSet<String> {
    let mut names = HashSet::new();
    if !zip_path.exists() {
        return names;
    }
    let file = match File::open(zip_path) {
        Ok(f) => f,
        Err(e) => {
            warn!("Could not open zip {:?} to read existing entries: {}", zip_path, e);
            return names;
        }
    };
    match zip::ZipArchive::new(file) {
        Ok(mut archive) => {
            (0..archive.len()).for_each(|i| {
                if let Ok(entry) = archive.by_index_raw(i) {
                    if let Ok(name) = entry.name() {
                        names.insert(name.into_owned());
                    }
                }
            });
        }
        Err(e) => {
            warn!("Could not read zip archive {:?}: {}", zip_path, e);
        }
    }
    names
}

/// Saves the document as JSON (and optionally HTML) entries inside a dated zip archive
/// stored under a year-based subdirectory.
///
/// Archive: {data_folder}/{YYYY}/{YYYY-MM-DD}.zip
/// Entries: {module}_{url_hash:016x}.json      (always written, unless already present)
///          {module}_{url_hash:016x}.html       (written when html_content is non-empty)
///
/// `received.filename` is updated to `{YYYY}/{YYYY-MM-DD}.zip/{entry}.json`.
fn save_to_zip(mut received: document::Document, data_folder_name: &str) -> document::Document {
    let stem = make_entry_stem(&received);
    let entry_name = format!("{}.json", stem);
    let html_entry_name = format!("{}.html", stem);

    // Extract the four-digit year from publish_date (YYYY-MM-DD); fall back to "1970".
    let year = received.publish_date.get(..4).unwrap_or("1970");
    let zip_name = format!("{}.zip", received.publish_date);

    // Build year subdirectory and full zip path.
    let year_dir = Path::new(data_folder_name).join(year);
    if let Err(e) = fs::create_dir_all(&year_dir) {
        error!("When creating year directory {:?}: {}", year_dir, e);
        return received;
    }
    let zip_path = year_dir.join(&zip_name);

    // Record the canonical filename for later database lookup.
    received.filename = format!("{}/{}/{}", year, zip_name, entry_name);
    info!("Writing document '{}' to zip entry: {}", received.title, received.filename);

    let json_data = match serde_json::to_string_pretty(&received.to_output_json()) {
        Ok(data) => data,
        Err(e) => {
            error!("When serialising document to JSON: {}", e);
            return received;
        }
    };

    // Read existing entry names so we can skip duplicates.
    let existing_entries = existing_zip_entries(&zip_path);
    if existing_entries.contains(&entry_name) {
        warn!(
            "Skipping duplicate zip entry '{}' in {} (document already stored).",
            entry_name, zip_name
        );
        return received;
    }

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

    // Write JSON entry
    match zip.start_file(&entry_name, options) {
        Ok(_) => {},
        Err(e) => {
            error!("When starting zip entry {}: {}", entry_name, e);
            return received;
        }
    }
    match zip.write_all(json_data.as_bytes()) {
        Ok(_) => debug!("Wrote '{}' to {}", entry_name, zip_name),
        Err(e) => error!("When writing to zip entry {}: {}", entry_name, e)
    }

    // Write HTML entry if raw HTML was captured and not already present.
    if !received.html_content.is_empty() && !existing_entries.contains(&html_entry_name) {
        match zip.start_file(&html_entry_name, options) {
            Ok(_) => match zip.write_all(received.html_content.as_bytes()) {
                Ok(_) => debug!("Wrote HTML '{}' to {}", html_entry_name, zip_name),
                Err(e) => error!("When writing HTML to zip entry {}: {}", html_entry_name, e)
            },
            Err(e) => error!("When starting HTML zip entry {}: {}", html_entry_name, e)
        }
    }

    match zip.finish() {
        Ok(_) => {},
        Err(e) => error!("When finalising zip {}: {}", zip_name, e)
    }

    received
}

/// Saves CSV-formatted market data from `doc.text` to a SQLite database.
///
/// Only runs when:
///  - `doc.module` is `"mod_in_nse"` or `"mod_in_bse"`, AND
///  - `doc.text` begins with a non-empty line that contains at least one comma.
///
/// The target database path is read from the `market_data_db` config key; when
/// absent it falls back to the `completed_urls_datafile` value with `"_market.db"`
/// appended.
fn save_market_data_to_sqlite(doc: &document::Document, app_config: &Config) {
    // Only handle NSE / BSE documents.
    if doc.module != "mod_in_nse" && doc.module != "mod_in_bse" {
        return;
    }

    // Check that doc.text starts with a CSV-like line (non-empty and contains a comma).
    let first_line = doc.text.lines().next().unwrap_or("").trim();
    if first_line.is_empty() || !first_line.contains(',') {
        return;
    }

    // Resolve the SQLite database path.
    let db_path = match app_config.get_string("market_data_db") {
        Ok(p) if !p.is_empty() => p,
        _ => {
            // Fall back: completed_urls_datafile + "_market.db"
            let base = app_config
                .get_string("completed_urls_datafile")
                .unwrap_or_else(|_| "newslookout_urls.db".to_string());
            format!("{}_market.db", base)
        }
    };

    info!(
        "{}: Saving CSV market data for '{}' (module={}) to SQLite: {}",
        PLUGIN_NAME, doc.title, doc.module, db_path
    );

    // Use the proper NSE schema for NSE documents; generic schema for BSE/others.
    let result = if doc.module == "mod_in_nse" {
        crate::market_data::save_nse_csv_to_sqlite(&doc.text, &db_path)
    } else {
        crate::market_data::save_csv_to_sqlite(&doc.text, &doc.module, &doc.publish_date, &db_path)
    };

    if let Err(e) = result {
        error!(
            "{}: When saving market data to SQLite {}: {}",
            PLUGIN_NAME, db_path, e
        );
    }
}

fn write_to_database(doc: document::Document, _app_config: &Config) -> document::Document {
    // TODO: implement this
    doc
}
