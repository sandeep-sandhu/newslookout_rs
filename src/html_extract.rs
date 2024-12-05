
// file: html_extract

use std::collections::HashMap;
use chrono::NaiveDate;
use log::{debug, error};
use regex::Regex;
use scraper::ElementRef;
use crate::document::{Document};
use crate::utils::{clean_text, get_text_from_element, to_local_datetime};



/// Extract plain text form HTML content
///
/// # Arguments
///
/// * `html_content`: The html data that needs to be extrcted from.
///
/// returns: String
pub fn extract_text_from_html(html_content: &str) -> String{
    let html_root_elem = scraper::html::Html::parse_document(html_content);
    // TODO: apply text density calculations, and
    // position based heuristics to identify relevant content to extract
    return get_text_from_element(html_root_elem.root_element());
}

/// Extract document details from a row of news article listings produced by liferay portal
///
/// # Arguments
///
/// * `row_each`: The Element object of the row from which the details of the document are to be extracted
/// * `source_url`: The source URL
///
/// returns: Document
pub fn extract_doc_from_row(row_each: ElementRef, source_url: &str) -> Document{

    let alink_selector = scraper::Selector::parse("a.mtm_list_item_heading").unwrap();
    let date_selector = scraper::Selector::parse("div.notification-date>span").unwrap();
    let doctitle_selector = scraper::Selector::parse("span.mtm_list_item_heading").unwrap();
    let pdf_link_selector = scraper::Selector::parse("a.matomo_download").unwrap();
    let description_snippet_selector = scraper::Selector::parse("div.notifications-description p").unwrap();

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

    let snippet_regex: Regex = Regex::new(
        r"(RBI[/A-Z]+\d{4}-\d{2,4}/\d*)(.+\d{4}-\d{2,4}[ ]*)((January|February|March|April|May|June|July|August|September|October|November|December)[\d ]+,[\d ]+)(.+)(Madam|Madam[ ]*/[ ]*Dear Sir|Dear Sir/|Dear Sir /|Madam / Dear Sir|Madam / Sir|$)"
    ).unwrap();

    for alink_elem in row_each.select(&alink_selector) {
        if let Some(href) = alink_elem.value().attr("href") {
            this_new_doc.url = href.parse().unwrap();

        }
    }

    // get published date:
    for date_div_elem in row_each.select(&date_selector) {
        date_str = clean_text(date_div_elem.inner_html());
        match NaiveDate::parse_from_str(date_str.as_str(), "%b %d, %Y"){
            Ok(naive_date) => {
                this_new_doc.publish_date_ms = to_local_datetime(naive_date).timestamp();
                this_new_doc.publish_date = naive_date.format("%Y-%m-%d").to_string();
            },
            Err(date_err) => {
                error!("Could not parse date '{}', error: {}", date_str.as_str(), date_err)
            }
        }
        debug!("From url {} , identified date = {}, timestamp = {}", this_new_doc.url, date_str, this_new_doc.publish_date_ms);
    }

    for title_span_elem in row_each.select(&doctitle_selector) {
        this_new_doc.title = clean_text(get_text_from_element(title_span_elem));
        this_new_doc.links_inward = vec![source_url.to_string()];
        debug!("Identified title: '{}' for url {}", this_new_doc.title, this_new_doc.url);
    }

    let mut snippet_text = String::from(" ");
    for snippet_elem in row_each.select(&description_snippet_selector) {
        let description_snippet = clean_text(
            get_text_from_element(snippet_elem)
        )
            .replace("\r\n", " ")
            .replace("\n", " ");
        snippet_text.push_str(" ");
        snippet_text.push_str(description_snippet.as_str());
        debug!("Retrieving parts from inner elements: {}", snippet_text);
        if let Some(caps) = snippet_regex.captures(snippet_text.as_str()) {
            let id_prefix = caps.get(1).unwrap().as_str();
            this_new_doc.unique_id = clean_text(caps.get(2).unwrap().as_str().to_string());
            let pubdate_longformat_str = caps.get(3).unwrap().as_str();
            this_new_doc.recipients = caps.get(5).unwrap().as_str().to_string();
            debug!("\tid_prefix: {},\n unique_id: {},\n pubdate_longformat_str: {},\n recipients: {}",
                    id_prefix, this_new_doc.unique_id, pubdate_longformat_str, this_new_doc.recipients);
        }
    }

    for pdf_url_elem in row_each.select(&pdf_link_selector) {
        if let Some(href) = pdf_url_elem.value().attr("href") {
            this_new_doc.pdf_url = href.parse().unwrap();
        }
    }
    return this_new_doc;
}