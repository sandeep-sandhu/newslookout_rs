// file: split_text

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use config::Config;
use log::{debug, error, info};
use regex::Regex;
use serde_json::{json, Value};
use crate::document::Document;
use crate::utils::{clean_text, get_text_from_element, split_by_word_count, to_local_datetime};
use crate::cfg::{get_plugin_config, };

pub const PLUGIN_NAME: &str = "split_text";
const PUBLISHER_NAME: &str = "Split document text";

/// Process documents received on channel rx and,
/// transmit the updated documents to tx.
///
/// # Arguments
///
/// * `tx`: Queue transmitter for the next thread
/// * `rx`: Queue receiver for this thread
/// * `config`: The application's configuration object
///
/// returns: ()
///
pub(crate) fn process_data(tx: Sender<Document>, rx: Receiver<Document>, app_config: &Config){

    info!("{}: Getting configuration.", PLUGIN_NAME);
    let mut min_word_limit_to_split: u64 = 600;
    match get_plugin_config(&app_config, PLUGIN_NAME, "min_word_limit_to_split"){
        Some(min_word_limit_to_split_str) => {
            match min_word_limit_to_split_str.parse::<u64>(){
                Ok(configintvalue) => min_word_limit_to_split =configintvalue, Err(e)=>{}
            }
        }, None => {}
    };
    let mut doc_counter: usize = 0;

    for mut doc in rx {

        debug!("{}: Started processing document titled - {}", PLUGIN_NAME, doc.title);
        check_and_split_text(&mut doc, min_word_limit_to_split, 50);

        // send updated document into the output queue:
        match tx.send(doc) {
            Result::Ok(_) => {doc_counter += 1;},
            Err(e) => error!("{}: When sending processed doc via tx: {}", PLUGIN_NAME, e)
        }
    }

    info!("{}: Completed processing {} documents.", PLUGIN_NAME, doc_counter);
}


/// Checks the document and splits the text into multiple parts based on the given word count.
/// Only text with more words than given count will be split.
///
/// # Arguments
///
/// * `doc_to_process`: The document object to read the text from and update after splitting the
/// text into parts
/// * `min_word_limit_to_split`: The minimum word limit below which the text will not be split into
/// parts
///
/// returns: ()
pub fn check_and_split_text(doc_to_process: &mut Document, min_word_limit_to_split: u64, previous_text_overlap: usize){

    // first replace single space lines to double newline characters: "\n \n" -> "\n\n"
    // doc_to_process.text = doc_to_process.text.replace("\n \n", "\n\n");
    let double_line_regex: Regex = Regex::new(r"\n\s+\n").unwrap();
    match double_line_regex.replace_all(doc_to_process.text.as_str(), "\n\n"){
        Cow::Borrowed(same_expr) => {doc_to_process.text = same_expr.to_string()}
        Cow::Owned(replaced) => {doc_to_process.text = replaced}
    }

    let mut initial_split_regex: Option<Regex> = None;
    // if doc_to_process.module==crate::plugins::rbi::PLUGIN_NAME {
    //     initial_split_regex = Some(
    //         Regex::new(
    //             r"(\n[ ]*\nAnnex[ure]* |\n[ ]*\nAppendix |[ ]+Page \d+ of \d+[ ]+ANNEXURE |[ ]+Page \d+ of \d+[ ]+APPENDIX )"
    //         ).expect("A valid regex for annexures and appendices")
    //     );
    // }

    let mut text_part_counter: usize = 1;

    if (doc_to_process.text.len() > 1) && (doc_to_process.text_parts.len() == 0) {

        debug!("Splitting document '{}' into parts.", doc_to_process.title);
        for text_part in split_by_word_count(
            doc_to_process.text.as_str(), min_word_limit_to_split as usize, previous_text_overlap, initial_split_regex
        ){
            if text_part.trim().len()> 1 {
                doc_to_process.text_parts.push(
                    HashMap::from([
                        ("id".to_string(), Value::String(text_part_counter.to_string())),
                        ("text".to_string(), Value::String(text_part)),
                        ("insights".to_string(), json!([])),
                    ])
                );
                text_part_counter += 1;
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use postgres::types::ToSql;
    use crate::document::new_document;
    use crate::plugins::split_text;
    use crate::plugins::split_text::check_and_split_text;

    #[test]
    fn test_check_and_split_text() {
        let mut test_rbi_doc = new_document();
        test_rbi_doc.module = "nothing".to_string();
        test_rbi_doc.module = crate::plugins::rbi::PLUGIN_NAME.to_string();
        test_rbi_doc.text = String::from("v4n57yp 9m934u\n \nDear sir madam, \
        \n\n  Appendix 232\n blah more blah and some extraa text to go along\n\n Annexure 111\n\
        sample calculation\n\n \nLot of more words that should not be split at all page 2 and\
        still more words\n ");
        check_and_split_text(&mut test_rbi_doc, 2, 1);
        assert_eq!(test_rbi_doc.text_parts[0].get("text").unwrap().as_str().unwrap(), String::from(" v4n57yp 9m934u"));
        assert_eq!(test_rbi_doc.text_parts[1].get("text").unwrap().as_str().unwrap(), String::from("9m934u Dear sir madam, "));
        assert_eq!(test_rbi_doc.text_parts[2].get("text").unwrap().as_str().unwrap(), String::from("madam,   Appendix 232\n blah more blah and some extraa text to go along"));
        assert_eq!(test_rbi_doc.text_parts[3].get("text").unwrap().as_str().unwrap(), String::from("along  Annexure 111\nsample calculation calculation Lot of more words that should not be split at all page 2 andstill more words\n "));

        test_rbi_doc.text_parts = Vec::new();
        test_rbi_doc.text = String::from("blah more blah\n\n and some extraa\n\n text to go.\n\n nothing here to see.");
        check_and_split_text(&mut test_rbi_doc, 2, 1);
        // TODO: fix this
        // for part in test_rbi_doc.text_parts{
        //     println!("--> {}", part.get("text").unwrap());
        // }
        // assert_eq!(1, 0);
    }
}
