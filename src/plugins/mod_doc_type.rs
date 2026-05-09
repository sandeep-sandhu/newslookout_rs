// file: mod_doc_type.rs
// Purpose: Data processing plugin - classify documents by type based on source module.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use config::Config;
use log::{debug, error, info};
use regex::Regex;
use crate::document;

pub const PLUGIN_NAME: &str = "mod_doc_type";

/// Executes this function of the module in the separate thread launched by the pipeline to
/// classify documents by type (doc_type) based on the source module and document metadata.
///
/// Module routing:
/// - "mod_en_in_rbi" or "rbi_new" → classify_rbi_document_type
/// - "mod_en_in_sebi" or "sebi"   → classify_sebi_document_type
/// - "mod_en_in_irdai" or "irdai" → classify_irdai_document_type
///
/// # Arguments
///
/// * `tx`: Queue transmitter for the next thread
/// * `rx`: Queue receiver for this thread
/// * `app_config`: The application's configuration object
/// * `api_mutexes`: Map of mutexes for rate-limiting API access
///
/// returns: ()
///
pub fn process_data(tx: Sender<document::Document>, rx: Receiver<document::Document>, _app_config: &Config, _api_mutexes: &mut HashMap<String, Arc<Mutex<isize>>>) {
    info!("{}: Starting module - Document classification.", PLUGIN_NAME);
    let mut doc_counter: u32 = 0;

    for mut doc in rx {

        if doc.module.eq_ignore_ascii_case("mod_en_in_rbi") || doc.module.eq_ignore_ascii_case("rbi_new") {
            let doc_type = classify_rbi_document_type(doc.title.as_str(), doc.section_name.as_str());
            doc.classification.insert("doc_type".to_string(), doc_type.to_string());
        } else if doc.module.eq_ignore_ascii_case("mod_en_in_sebi") || doc.module.eq_ignore_ascii_case("sebi") {
            let doc_type = classify_sebi_document_type(doc.title.as_str(), doc.url.as_str(), doc.section_name.as_str());
            doc.classification.insert("doc_type".to_string(), doc_type.to_string());
        } else if doc.module.eq_ignore_ascii_case("mod_en_in_irdai") || doc.module.eq_ignore_ascii_case("irdai") {
            let doc_type = classify_irdai_document_type(doc.title.as_str(), doc.url.as_str(), doc.section_name.as_str());
            doc.classification.insert("doc_type".to_string(), doc_type.to_string());
        }

        // for future use, add categorisation rules for other modules/websites:

        match tx.send(doc) {
            Result::Ok(_) => { doc_counter += 1; },
            Err(e) => error!("{}: When sending processed doc via tx: {}", PLUGIN_NAME, e)
        }
    }
    info!("{}: Completed processing a total of {} documents.", PLUGIN_NAME, doc_counter);
}


pub(crate) fn classify_rbi_document_type(title: &str, section_name: &str) -> &'static str {

    // process speeches first:
    match section_name {
        "Speeches" => return "speech",
        "speeches" => return "speech",
        _ => debug!("Unknown section type '{}'", section_name)
    }
    let auctions_pattn: Regex = Regex::new(r"(auction under |auction held|Auction Result|Underwriting Auction|Auction of State|Auction of Government|Auction Results|Treasury Bills auction|Auctions Conducted on)").unwrap();
    let repo_revrepo_pattn: Regex = Regex::new(r"(Variable Rate Reverse Repo|Variable Rate Repo)").unwrap();
    let mm_operation_pattn: Regex = Regex::new(r"(Money Market Operations|Reserve Money for)").unwrap();
    let bonds_pattn: Regex = Regex::new(r"(Sovereign Gold Bond \(SGB\) Scheme|Sovereign Gold Bonds,|Buyback of Government of India Dated Securities)").unwrap();
    let survey_pattn: Regex = Regex::new(r"(Launch[esing]* [of ]*[\w\s\d\(\)-]+ Survey|^Survey on )").unwrap();
    let int_rate_pattn: Regex = Regex::new(r"(Rate of interest on Government of India|Lending and Deposit Rates of Scheduled Commercial Banks)").unwrap();
    let mkt_data_pattn: Regex = Regex::new(r"Foreign Exchange Turnover Data:").unwrap();

    // mark doctype based on patterns:
    if auctions_pattn.is_match(title) {
        return "market_action";
    }
    if repo_revrepo_pattn.is_match(title) {
        return "market_action";
    }
    if mm_operation_pattn.is_match(title) {
        return "market_action";
    }
    if bonds_pattn.is_match(title) {
        return "market_action";
    }
    if survey_pattn.is_match(title) {
        return "survey";
    }
    if int_rate_pattn.is_match(title) {
        return "market_action";
    }
    if mkt_data_pattn.is_match(title) {
        return "market_data";
    }
    // if nothing matches, then by default this document is of type regulatory-notification:
    return "regulatory-notification";
}

pub(crate) fn classify_irdai_document_type(_title: &str, _url: &str, _section_name: &str) -> &'static str {
    // default type regulatory-notification:
    "regulatory-notification"
}

pub(crate) fn classify_sebi_document_type(title: &str, url: &str, section_name: &str) -> &'static str {

    // process speeches first:
    match section_name {
        "Speeches" => return "speech",
        "speeches" => return "speech",
        _ => debug!("Unknown section type '{}'", section_name)
    }

    let notice_pattn: Regex = Regex::new(r"(Notice of Demand under Recovery Certificate|Notice of Demand for Recovery Certificate|Notices of Attachment dated|Notice of Demand dated|Public Notice For E-Auction|Warning letter issued to)").unwrap();
    let indiv_order_pattn: Regex = Regex::new(r"(Remittance Order for Recovery Certificate|General Remittance Order dated|Completion Order for Recovery Certificate|Completion of Recovery Certificate|Release Order for Recovery Certificate|SEBI Order for Compliance|Settlement Order in respect of|Recovery Proceedings under|Adjudication Order)").unwrap();
    let appeal1_pattn: Regex = Regex::new(r"(Appeal No)").unwrap();
    let appeal2_pattn: Regex = Regex::new(r"(filed by)").unwrap();
    let prospectus_pattn: Regex = Regex::new(r"(Prospectus|Addendum to DRHP)").unwrap();
    let filings_url_pattn: Regex = Regex::new(r"(/filings/public-issues/|/filings/takeovers/|/filings/rights-issues/|/filings/invit-public-issues/)").unwrap();

    if notice_pattn.is_match(title) {
        return "individual_notice";
    } else if indiv_order_pattn.is_match(title) {
        return "individual_order";
    } else if appeal1_pattn.is_match(title) && appeal2_pattn.is_match(title) {
        return "individual_appeal";
    } else if prospectus_pattn.is_match(title) {
        return "prospectus";
    } else if filings_url_pattn.is_match(url) {
        return "filings";
    }
    // by default this document is of type regulatory-notification:
    "regulatory-notification"
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_rbi_speech() {
        assert_eq!(classify_rbi_document_type("Governor's speech at conference", "Speeches"), "speech");
        assert_eq!(classify_rbi_document_type("Some speech", "speeches"), "speech");
    }

    #[test]
    fn test_classify_rbi_market_action() {
        assert_eq!(classify_rbi_document_type("Result of auction under OMO", "Notifications"), "market_action");
        assert_eq!(classify_rbi_document_type("Variable Rate Reverse Repo auction", "Notifications"), "market_action");
        assert_eq!(classify_rbi_document_type("Money Market Operations for the week", "Press Release"), "market_action");
        assert_eq!(classify_rbi_document_type("Sovereign Gold Bond (SGB) Scheme", "Notifications"), "market_action");
        assert_eq!(classify_rbi_document_type("Rate of interest on Government of India securities", "Notifications"), "market_action");
    }

    #[test]
    fn test_classify_rbi_survey() {
        assert_eq!(classify_rbi_document_type("Launch of Consumer Confidence Survey", "Reports"), "survey");
    }

    #[test]
    fn test_classify_rbi_market_data() {
        assert_eq!(classify_rbi_document_type("Foreign Exchange Turnover Data: December 2024", "Press Release"), "market_data");
    }

    #[test]
    fn test_classify_rbi_default() {
        assert_eq!(classify_rbi_document_type("Master Direction on KYC", "Notifications"), "regulatory-notification");
    }

    #[test]
    fn test_classify_sebi_speech() {
        assert_eq!(classify_sebi_document_type("Chairperson speech", "https://sebi.gov.in/speeches/1", "Speeches"), "speech");
    }

    #[test]
    fn test_classify_sebi_notice() {
        assert_eq!(
            classify_sebi_document_type("Notice of Demand under Recovery Certificate No. 123", "https://sebi.gov.in/orders/1", "Orders"),
            "individual_notice"
        );
    }

    #[test]
    fn test_classify_sebi_default() {
        assert_eq!(
            classify_sebi_document_type("Circular on Mutual Funds", "https://sebi.gov.in/circulars/1", "Circulars"),
            "regulatory-notification"
        );
    }

    #[test]
    fn test_classify_irdai_default() {
        assert_eq!(classify_irdai_document_type("Any IRDAI document", "https://irdai.gov.in/circulars/1", "Circulars"), "regulatory-notification");
    }
}
