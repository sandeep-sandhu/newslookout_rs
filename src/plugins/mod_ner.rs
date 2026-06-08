// file: mod_ner.rs
// Purpose:
//   Phase-3 organization recogniser (roadmap Stage 7 / D1), deterministic rule-based variant.
//   A model-backed NER (ONNX/LLM-JSON) is planned, but this dependency-free recogniser gives the
//   entity graph and resolver something to work on now and is fully unit-testable. It finds
//   organizations two ways:
//     1. a curated gazetteer of Indian regulators / exchanges / major financial entities
//        (acronyms like RBI/SEBI and full names), and
//     2. corporate-suffix patterns ("<Capitalised words> Ltd/Limited/Bank/Corporation/...").
//   Each organization becomes an `EntityMention` on `doc.analysis.organizations` (and its surface
//   form is added to `all_names`). The canonical `entity_id` stays `None` here — false links are
//   worse than nulls — and a *provisional* surface-key id is assigned only at persistence time
//   (see `provisional_entity_id`) so `mod_entity_graph` / `mod_emit_tables` can group mentions
//   before `mod_entity_resolve` (LEI/CIN) supplies real ids.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};

use config::Config;
use log::{error, info};
use regex::Regex;

use crate::analysis::{norm_name, provisional_entity_id, EntityMention};
use crate::document::Document;

pub const PLUGIN_NAME: &str = "mod_ner";

const MIN_TEXT_LEN: usize = 40;

/// Curated organisation gazetteer: (surface/acronym, canonical name). Matched whole-word,
/// case-insensitively. Canonical name is used for the provisional id so "RBI" and
/// "Reserve Bank of India" collapse to one entity.
const KNOWN_ORGS: &[(&str, &str)] = &[
    ("RBI", "Reserve Bank of India"),
    ("Reserve Bank of India", "Reserve Bank of India"),
    ("SEBI", "Securities and Exchange Board of India"),
    ("Securities and Exchange Board of India", "Securities and Exchange Board of India"),
    ("IRDAI", "Insurance Regulatory and Development Authority of India"),
    ("NSE", "National Stock Exchange"),
    ("National Stock Exchange", "National Stock Exchange"),
    ("BSE", "Bombay Stock Exchange"),
    ("Bombay Stock Exchange", "Bombay Stock Exchange"),
    ("NABARD", "National Bank for Agriculture and Rural Development"),
    ("SIDBI", "Small Industries Development Bank of India"),
    ("SBI", "State Bank of India"),
    ("State Bank of India", "State Bank of India"),
    ("HDFC Bank", "HDFC Bank"),
    ("ICICI Bank", "ICICI Bank"),
    ("Axis Bank", "Axis Bank"),
    ("Kotak Mahindra Bank", "Kotak Mahindra Bank"),
    ("Punjab National Bank", "Punjab National Bank"),
    ("Bank of Baroda", "Bank of Baroda"),
    ("LIC", "Life Insurance Corporation of India"),
    ("CCIL", "Clearing Corporation of India"),
    ("NPCI", "National Payments Corporation of India"),
];

pub fn process_data(
    tx: Sender<Document>,
    rx: Receiver<Document>,
    _config: &Config,
    _api_mutexes: &mut HashMap<String, Arc<Mutex<isize>>>,
) {
    info!("{}: Starting rule-based organization NER.", PLUGIN_NAME);
    let mut docs = 0usize;
    let mut ents = 0usize;
    for mut doc in rx {
        if doc.text.len() >= MIN_TEXT_LEN {
            let orgs = extract_orgs(&doc.text);
            if !orgs.is_empty() {
                let mut analysis = doc.analysis.take().unwrap_or_default();
                ents += orgs.len();
                for o in &orgs {
                    if !analysis.all_names.contains(&o.surface_form) {
                        analysis.all_names.push(o.surface_form.clone());
                    }
                }
                analysis.organizations.extend(orgs);
                doc.analysis = Some(analysis);
                docs += 1;
            }
        }
        if let Err(e) = tx.send(doc) {
            error!("{}: when forwarding doc: {}", PLUGIN_NAME, e);
        }
    }
    info!("{}: Completed. Found {} organization mention(s) in {} document(s).", PLUGIN_NAME, ents, docs);
}

/// Matches "<Capitalised words> <corporate suffix>", e.g. "Tata Consultancy Services Ltd",
/// "Adani Power Limited", "Bajaj Finance".
fn org_suffix_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"\b([A-Z][A-Za-z&.]*(?:\s+(?:&\s+)?[A-Z][A-Za-z&.]*){0,5}\s+(?:Ltd|Limited|Pvt|Private|Inc|PLC|Corp|Corporation|Company|Industries|Holdings|Enterprises|Technologies|Motors|Finance|Financial|Securities|Insurance|Bank))\b\.?",
        )
        .expect("org suffix regex")
    })
}

/// Extract organisation mentions. Each distinct organisation (by normalised canonical name) is
/// returned once, at its earliest offset, with salience = occurrences / max-occurrences.
pub fn extract_orgs(text: &str) -> Vec<EntityMention> {
    // canonical-norm -> (surface_form, earliest_offset, count)
    let mut found: HashMap<String, (String, usize, u32)> = HashMap::new();

    // 1. Gazetteer hits.
    let lower = text.to_lowercase();
    let bytes = lower.as_bytes();
    for (surface, canonical) in KNOWN_ORGS {
        let needle = surface.to_lowercase();
        let mut start = 0;
        while let Some(rel) = lower[start..].find(&needle) {
            let idx = start + rel;
            let before_ok = idx == 0 || !is_word_byte(bytes[idx - 1]);
            let after = idx + needle.len();
            let after_ok = after >= bytes.len() || !is_word_byte(bytes[after]);
            if before_ok && after_ok {
                let key = norm_name(canonical);
                let e = found.entry(key).or_insert(((*canonical).to_string(), idx, 0));
                e.1 = e.1.min(idx);
                e.2 += 1;
            }
            start = idx + 1;
            if start >= lower.len() {
                break;
            }
        }
    }

    // 2. Corporate-suffix patterns.
    for caps in org_suffix_re().captures_iter(text) {
        if let Some(m) = caps.get(1) {
            let surface = m.as_str().trim_end_matches('.').trim().to_string();
            let key = norm_name(&surface);
            if key.is_empty() {
                continue;
            }
            let e = found.entry(key).or_insert((surface, m.start(), 0));
            e.1 = e.1.min(m.start());
            e.2 += 1;
        }
    }

    let max_count = found.values().map(|(_, _, c)| *c).max().unwrap_or(1).max(1);
    let mut out: Vec<EntityMention> = found
        .into_values()
        .map(|(surface, offset, count)| EntityMention {
            surface_form: surface,
            entity_type: "ORG".to_string(),
            char_offset: offset,
            salience: count as f64 / max_count as f64,
            entity_id: None,
        })
        .collect();
    out.sort_by_key(|m| m.char_offset);
    out
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surfaces(text: &str) -> Vec<String> {
        extract_orgs(text).into_iter().map(|m| m.surface_form).collect()
    }

    #[test]
    fn test_gazetteer_acronym_and_fullname_collapse() {
        let orgs = extract_orgs("The RBI met today; the Reserve Bank of India will publish minutes.");
        // Both refer to the same canonical entity → one mention.
        let rbi: Vec<_> = orgs.iter().filter(|m| m.surface_form == "Reserve Bank of India").collect();
        assert_eq!(rbi.len(), 1, "RBI and full name should collapse: {:?}", surfaces("The RBI met today; the Reserve Bank of India will publish minutes."));
    }

    #[test]
    fn test_corporate_suffix_pattern() {
        let s = surfaces("Shares of Tata Consultancy Services Ltd and Bajaj Finance rose sharply today.");
        assert!(s.iter().any(|x| x.contains("Tata Consultancy Services Ltd")), "got {:?}", s);
        assert!(s.iter().any(|x| x.contains("Bajaj Finance")), "got {:?}", s);
    }

    #[test]
    fn test_provisional_id_stable_and_normalised() {
        assert_eq!(provisional_entity_id("Reserve Bank of India"), "name:reserve bank of india");
        assert_eq!(provisional_entity_id("HDFC Bank"), provisional_entity_id("hdfc  bank"));
    }

    #[test]
    fn test_salience_in_unit_range() {
        let orgs = extract_orgs("SBI SBI SBI dominated; Axis Bank appeared once in the report here.");
        assert!(orgs.iter().all(|m| m.salience > 0.0 && m.salience <= 1.0));
        let sbi = orgs.iter().find(|m| m.surface_form == "State Bank of India").unwrap();
        assert_eq!(sbi.salience, 1.0, "most frequent org has salience 1.0");
    }

    #[test]
    fn test_no_orgs_in_plain_text() {
        assert!(extract_orgs("the weather was calm and the river flowed gently past the village").is_empty());
    }
}
