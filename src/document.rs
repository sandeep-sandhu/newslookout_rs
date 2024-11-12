// file: document.rs

use std::collections::HashMap;
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

// Description of Tests:
// These unit tests verify that the serialization and deserialization of the Document work
// properly using serde_json.
#[cfg(test)]
mod tests {

    use chrono::Utc;
    use super::*;

    #[test]
    fn test_data_structures() {
        let nowdatetime = Utc::now();

        let data = Document {
            module: "Some module".to_string(),
            plugin_name: "Some Plugin Name".to_string(),
            section_name: "some section".to_string(),
            url: "some url".to_string(),
            pdf_url: "some pdf url".to_string(),
            html_content: "some html".to_string(),
            title: "some title".to_string(),
            unique_id: "Some unique id".to_string(),
            referrer_text: "Some referrer".to_string(),
            text: "Some text content".to_string(),
            source_author: "Some author".to_string(),
            recipients: "Some recepients".to_string(),
            publish_date_ms: nowdatetime.timestamp(),
            publish_date: "1970-01-01".to_string(),
            links_inward: vec![],
            links_outwards: vec![],
            text_parts: vec![ HashMap::from( [("id".to_string(),"1".to_string()), ("text".to_string(), "blank".to_string())] ) ],
            classification: HashMap::new(),
            filename: "".to_string(),
            generated_content: HashMap::new(),
        };

        // let naive_date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d").unwrap();

        // Verify that data can be serialized and deserialized
        let serialized = serde_json::to_string(&data).unwrap();
        let deserialized: Document = serde_json::from_str(&serialized).unwrap();

        assert_eq!(data, deserialized);
    }
}