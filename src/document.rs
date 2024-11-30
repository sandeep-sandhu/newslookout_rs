// file: document.rs

use std::collections::{HashMap, HashSet};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, Map, Number};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Document {
    /// Module name
    pub module: String,
    /// Descriptive name of the module
    pub plugin_name: String,
    /// Name of the section of the website from which this document was taken
    pub section_name: String,
    /// URL of this document
    pub url: String,
    /// URL of the PDF of this document
    pub pdf_url: String,
    /// File path where this document is to be saved.
    pub filename: String,
    /// Content of this document in HTML format
    pub html_content: String,
    /// Title of this document
    pub title: String,
    /// Unique identifier of this document/article, may have been provided by the website
    pub unique_id: String,
    // In some cases, the document is referred by another document, and it can be captured here
    pub referrer_text: String,
    /// The content of this document in plain text without any formatting
    pub text: String,
    /// Author or source of this document/news article
    pub source_author: String,
    /// The intended recepients of this document/message or article
    pub recipients: String,
    /// The timestamp of the publication of this document, milliseconds since epoch 1970-01-01
    pub publish_date_ms: i64,
    /// The date of publication of this document, YYYY-MM-DD
    pub publish_date: String,
    /// The revision dates of this document
    pub revision_dates: Vec<String>,
    /// The documents referring to this document
    pub links_inward: Vec<String>,
    /// The documents/links of other documents referred in this document
    pub links_outwards: Vec<String>,
    /// The plain text of the contents split and stored in this array
    pub text_parts: Vec<HashMap<String, Value>>,
    /// The categories of this document
    pub classification: HashMap<String, String>,
    /// The executive summary and any similar other content generated are stored in this Map
    pub generated_content: HashMap<String, String>,
    /// The data processing flags (stored as a bit mask) indicate what type of processing is
    /// required for this document. It will be used by each data processing plugin to identify
    /// whether to process this document or not.
    pub data_proc_flags: usize,
}

/// Creates a new empty document object with default attributes
pub fn new_document() -> Document {

    let curr_timestamp = Utc::now().timestamp();

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

/// Flag to indicate whether sentiment classification is to be run on the contents of this document
pub const DATA_PROC_CLASSIFY_SENTIMENT: usize = 1;
/// Flag to indicate whether this document should be classified by industry type
pub const DATA_PROC_CLASSIFY_INDUSTRY: usize = 2;
/// Flag to indicate whether this document should be classified by market type
pub const DATA_PROC_CLASSIFY_MARKET: usize = 4;
/// Flag to indicate whether this document should be classified by product type
pub const DATA_PROC_CLASSIFY_PRODUCT: usize = 8;
/// Flag to indicate whether Names and Entities would be extracted for this document
pub const DATA_PROC_EXTRACT_NAME_ENTITY: usize = 16;
/// Flag to indicate whether keywords would be extracted for this document
pub const DATA_PROC_EXTRACT_KEYWORDS: usize = 32;
/// Flag to indicate whether this document should be compared and similar documents identified
pub const DATA_PROC_IDENTIFY_SIMILAR_DOCS: usize = 64;
/// Flag to indicate whether this document should be summarised using NLP models
pub const DATA_PROC_SUMMARIZE: usize = 128;




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