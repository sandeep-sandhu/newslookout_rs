// file: mod_themes.rs
// Purpose:
//   Phase-1 controlled-vocabulary theme tagger (roadmap Stage 5 / D6). Tags each document with
//   GDELT-style theme codes by matching a versioned, embedded "themebook" of phrases against
//   the article text. Deterministic and dependency-free (whole-word, case-insensitive phrase
//   matching), so it is fully unit-testable. Results land on `doc.analysis.themes`; persistence
//   to the `themes` table is handled later by `mod_emit_tables`.
//
//   The themebook is intentionally finance/regulatory-leaning (the corpus is Indian financial
//   news + regulator circulars). It is versioned via THEMEBOOK_VERSION so downstream consumers
//   can reason about vocabulary drift.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use config::Config;
use log::{error, info};

use crate::analysis::ThemeMention;
use crate::document::Document;

pub const PLUGIN_NAME: &str = "mod_themes";
pub const THEMEBOOK_VERSION: &str = "2026.06.1";

const MIN_TEXT_LEN: usize = 40;

/// (theme code, trigger phrases). Phrases are lowercased and matched on word boundaries.
const THEMEBOOK: &[(&str, &[&str])] = &[
    ("ECON_INTEREST_RATE", &["interest rate", "repo rate", "reverse repo", "lending rate", "rate hike", "rate cut"]),
    ("ECON_INFLATION", &["inflation", "consumer price", "wholesale price", "cpi", "wpi", "price rise"]),
    ("ECON_MONETARY_POLICY", &["monetary policy", "policy rate", "liquidity", "open market operation", "crr", "slr"]),
    ("ECON_GROWTH", &["gdp", "economic growth", "gross domestic product", "industrial output"]),
    ("FIN_BANKING", &["bank", "banking", "lender", "deposit", "credit growth", "npa", "non-performing asset"]),
    ("FIN_MARKETS", &["stock market", "sensex", "nifty", "equity", "bourses", "share price", "bond yield"]),
    ("FIN_IPO", &["ipo", "initial public offering", "public issue", "listing gains"]),
    ("FIN_MERGER", &["merger", "acquisition", "takeover", "amalgamation", "stake sale", "buyout"]),
    ("REG_ENFORCEMENT", &["penalty", "fine", "enforcement", "show cause", "adjudication", "disgorgement", "debarred"]),
    ("REG_GUIDELINE", &["circular", "circulars", "guideline", "guidelines", "notification", "notifications", "master direction", "framework", "regulation", "regulations"]),
    ("FRAUD_FINANCIAL", &["fraud", "scam", "ponzi", "embezzlement", "misappropriation", "round-tripping"]),
    ("CRIME_MONEY_LAUNDERING", &["money laundering", "anti-money laundering", "aml", "terror financing", "pmla"]),
    ("INSOLVENCY", &["insolvency", "bankruptcy", "liquidation", "resolution plan", "ibc", "nclt"]),
    ("TAX", &["income tax", "gst", "goods and services tax", "tax evasion", "direct tax", "cbdt"]),
    ("CRYPTO", &["cryptocurrency", "bitcoin", "crypto", "virtual digital asset", "blockchain"]),
    ("CORP_GOVERNANCE", &["corporate governance", "board of directors", "insider trading", "related party"]),
];

pub fn process_data(
    tx: Sender<Document>,
    rx: Receiver<Document>,
    _config: &Config,
    _api_mutexes: &mut HashMap<String, Arc<Mutex<isize>>>,
) {
    info!("{}: Starting theme tagging (themebook v{}).", PLUGIN_NAME, THEMEBOOK_VERSION);
    let mut docs = 0usize;
    let mut tags = 0usize;
    for mut doc in rx {
        if doc.text.len() >= MIN_TEXT_LEN {
            let themes = tag_themes(&doc.text);
            if !themes.is_empty() {
                let mut analysis = doc.analysis.take().unwrap_or_default();
                tags += themes.len();
                analysis.themes.extend(themes);
                doc.analysis = Some(analysis);
                docs += 1;
            }
        }
        if let Err(e) = tx.send(doc) {
            error!("{}: when forwarding doc: {}", PLUGIN_NAME, e);
        }
    }
    info!("{}: Completed. Tagged {} document(s) with {} theme mention(s).", PLUGIN_NAME, docs, tags);
}

/// Return one `ThemeMention` per (theme, first-occurrence) found in the text. A theme is
/// recorded at most once (at its earliest offset) even if several of its phrases match.
pub fn tag_themes(text: &str) -> Vec<ThemeMention> {
    let lower = text.to_lowercase();
    let bytes = lower.as_bytes();
    let mut out: Vec<ThemeMention> = Vec::new();
    for (theme, phrases) in THEMEBOOK {
        let mut best: Option<usize> = None;
        for phrase in *phrases {
            if let Some(off) = find_whole_word(&lower, bytes, phrase) {
                best = Some(best.map_or(off, |b| b.min(off)));
            }
        }
        if let Some(off) = best {
            out.push(ThemeMention { theme: (*theme).to_string(), char_offset: off });
        }
    }
    out.sort_by_key(|t| t.char_offset);
    out
}

/// Find the first whole-word/phrase occurrence of `needle` in `haystack` (already lowercased),
/// requiring non-alphanumeric boundaries on both sides so "bank" does not match "embankment".
fn find_whole_word(haystack: &str, bytes: &[u8], needle: &str) -> Option<usize> {
    let nlen = needle.len();
    let mut start = 0;
    while let Some(rel) = haystack[start..].find(needle) {
        let idx = start + rel;
        let before_ok = idx == 0 || !is_word_byte(bytes[idx - 1]);
        let after = idx + nlen;
        let after_ok = after >= bytes.len() || !is_word_byte(bytes[after]);
        if before_ok && after_ok {
            return Some(idx);
        }
        start = idx + 1;
        if start >= haystack.len() {
            break;
        }
    }
    None
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn themes_of(text: &str) -> Vec<String> {
        tag_themes(text).into_iter().map(|t| t.theme).collect()
    }

    #[test]
    fn test_tags_banking_and_inflation() {
        let t = themes_of("The bank warned that rising inflation may force a repo rate hike.");
        assert!(t.contains(&"FIN_BANKING".to_string()), "got {:?}", t);
        assert!(t.contains(&"ECON_INFLATION".to_string()), "got {:?}", t);
        assert!(t.contains(&"ECON_INTEREST_RATE".to_string()), "got {:?}", t);
    }

    #[test]
    fn test_aml_and_enforcement() {
        let t = themes_of("SEBI issued a penalty under anti-money laundering guidelines for fraud.");
        assert!(t.contains(&"REG_ENFORCEMENT".to_string()), "got {:?}", t);
        assert!(t.contains(&"CRIME_MONEY_LAUNDERING".to_string()), "got {:?}", t);
        assert!(t.contains(&"FRAUD_FINANCIAL".to_string()), "got {:?}", t);
        assert!(t.contains(&"REG_GUIDELINE".to_string()), "got {:?}", t);
    }

    #[test]
    fn test_whole_word_boundary() {
        // "embankment" contains "bank" but must not trigger FIN_BANKING on its own.
        let t = themes_of("Workers repaired the river embankment after the floods.");
        assert!(!t.contains(&"FIN_BANKING".to_string()), "embankment falsely matched: {:?}", t);
    }

    #[test]
    fn test_theme_deduped_and_sorted() {
        let t = tag_themes("bank bank bank deposit");
        let banking: Vec<_> = t.iter().filter(|m| m.theme == "FIN_BANKING").collect();
        assert_eq!(banking.len(), 1, "theme should appear once");
    }

    #[test]
    fn test_no_themes_neutral_text() {
        assert!(tag_themes("The weather was pleasant and the children played in the park.").is_empty());
    }
}
