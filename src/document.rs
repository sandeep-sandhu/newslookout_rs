// file: document.rs

use std::collections::HashMap;
use chrono::Utc;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct DocInfo {
    pub plugin_name: String,
    pub section_name: String,
    pub url: String,
    pub pdf_url: String,
    pub title: String,
    pub unique_id: String,
    pub publish_date_ms: i64,
    pub filename: String,
}

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
    pub links_inward: Vec<String>,
    pub links_outwards: Vec<String>,
    pub text_parts: Vec<HashMap<String, String>>,
    pub classification: HashMap<String, String>,
    pub generated_content: HashMap<String, String>,
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
        links_inward: Vec::new(),
        links_outwards: Vec::new(),
        text_parts: Vec::new(),
        classification: HashMap::new(),
        filename: "".to_string(),
        generated_content: HashMap::new(),
    };
}


// Description of Tests:
// These unit tests verify that the serialization and deserialization of the Document work
// properly using serde_json.
#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_data_structures() {
        let mut data = new_document();
        data.text_parts = vec![ HashMap::from( [("id".to_string(),"1".to_string()), ("text".to_string(), "blank".to_string())] ) ];

        // Verify that data can be serialized and deserialized
        let serialized = serde_json::to_string(&data).unwrap();
        let deserialized: Document = serde_json::from_str(&serialized).unwrap();

        assert_eq!(data, deserialized);
    }
}