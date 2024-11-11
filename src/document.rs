// file: document.rs

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct DocInfo {
    pub(crate) plugin_name: String,
    pub(crate) section_name: String,
    pub(crate) url: String,
    pub(crate) pdf_url: String,
    pub(crate) title: String,
    pub(crate) unique_id: String,
    pub(crate) publish_date_ms: i64,
    pub(crate) filename: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct TextPart {
    pub(crate) id: u32,
    pub(crate) text: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct Document {
    pub(crate) module: String,
    pub(crate) plugin_name: String,
    pub(crate) section_name: String,
    pub(crate) url: String,
    pub(crate) pdf_url: String,
    pub(crate) html_content: String,
    pub(crate) title: String,
    pub(crate) unique_id: String,
    pub(crate) referrer_text: String,
    pub(crate) text: String,
    pub(crate) source_author: String,
    pub(crate) recipients: String,
    pub(crate) publish_date_ms: i64,
    pub(crate) links_inward: Vec<String>,
    pub(crate) links_outwards: Vec<String>,
    pub(crate) text_parts: Vec<HashMap<String, String>>,
    pub(crate) filename: String,
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
            links_inward: vec![],
            links_outwards: vec![],
            text_parts: vec![ HashMap::from( [("id".to_string(),"1".to_string()), ("text".to_string(), "blank".to_string())] ) ],
            filename: "".to_string(),
        };

        // let naive_date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d").unwrap();

        // Verify that data can be serialized and deserialized
        let serialized = serde_json::to_string(&data).unwrap();
        let deserialized: Document = serde_json::from_str(&serialized).unwrap();

        assert_eq!(data, deserialized);
    }
}