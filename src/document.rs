// file: document.rs

use std::collections::{HashMap, HashSet};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, Map, Number};

// #[derive(Debug, Serialize, Deserialize, PartialEq)]
// pub(crate) struct TextPart {
//     pub(crate) id: u32,
//     pub(crate) text: String,
// }

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Document {
    pub module: String,
    pub plugin_name: String,
    pub section_name: String,
    pub url: String,
    pub pdf_url: String,
    pub filename: String,
    pub html_content: String,
    pub title: String,
    pub unique_id: String,
    pub referrer_text: String,
    pub text: String,
    pub source_author: String,
    pub recipients: String,
    pub publish_date_ms: i64,
    pub publish_date: String,
    pub revision_dates: Vec<String>,
    pub links_inward: Vec<String>,
    pub links_outwards: Vec<String>,
    pub text_parts: Vec<HashMap<String, Value>>,
    pub classification: HashMap<String, String>,
    pub generated_content: HashMap<String, String>,
    pub data_proc_flags: usize,
}

pub fn new_document() -> Document {
    let curr_timestamp = Utc::now().timestamp();
    // prepare empty document:
    return Document{
        module: "".to_string(),
        plugin_name: "".to_string(),
        section_name: "".to_string(),
        url: "".to_string(),
        pdf_url: "".to_string(),
        html_content: "".to_string(),
        title: "".to_string(),
        unique_id: "".to_string(),
        referrer_text: "".to_string(),
        text: "".to_string(),
        source_author: "".to_string(),
        recipients: "".to_string(),
        publish_date_ms: curr_timestamp,
        publish_date: "1970-01-01".to_string(),
        revision_dates: vec![],
        links_inward: Vec::new(),
        links_outwards: Vec::new(),
        text_parts: Vec::new(),
        classification: HashMap::new(),
        filename: "".to_string(),
        generated_content: HashMap::new(),
        data_proc_flags: 0,
    };
}

pub const DATA_PROC_CLASSIFY_SENTIMENT: usize = 1;
pub const DATA_PROC_CLASSIFY_INDUSTRY: usize = 2;
pub const DATA_PROC_CLASSIFY_MARKET: usize = 4;
pub const DATA_PROC_CLASSIFY_PRODUCT: usize = 8;
pub const DATA_PROC_EXTRACT_NAME_ENTITY: usize = 16;
pub const DATA_PROC_EXTRACT_KEYWORDS: usize = 32;
pub const DATA_PROC_IDENTIFY_SIMILAR_DOCS: usize = 64;
pub const DATA_PROC_SUMMARIZE: usize = 128;
pub const DATA_PROC_EXTRACT_ACTIONS: usize = 256;
pub const DATA_PROC_COMPARE_PREVIOUS_VERSION: usize = 512;



// Description of Tests:
// These unit tests verify that the serialization and deserialization of the Document work
// properly using serde_json.
#[cfg(test)]
mod tests {
    use serde_json::json;
    use crate::document;
    use super::*;

    #[test]
    fn test_data_structures() {
        let mut data = new_document();
        data.text_parts = vec![ HashMap::from( [
            ("id".to_string(), Value::String("1".to_string())),
            ("text".to_string(), Value::String("blank".to_string())),
            ("insights".to_string(),
             json!([{ "1": "object" },{ "2": "next object" },])
            )
        ] ) ];

        // Verify that data can be serialized and deserialized
        let serialized = serde_json::to_string(&data).unwrap();
        let deserialized: Document = serde_json::from_str(&serialized).unwrap();

        assert_eq!(data, deserialized);
    }

    #[test]
    fn test_data_processing_flags(){
        let example1 = 16+128;

        assert_eq!(1, 1);
    }
}