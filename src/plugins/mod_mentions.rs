// file: mod_mentions.rs
// Purpose:
//   Phase-0 "free signal" data_processor (roadmap Stage 4 / D10 + D5 starter). It does two
//   cheap, high-leverage things with data the pipeline already has:
//     1. Treats each SimHash near-duplicate cluster (assigned by mod_dedupe) as a story/event
//        and records every article as a GDELT-style *Mention* of that cluster — a free
//        attention/propagation signal.
//     2. Computes a cheap lexicon-based document tone (-10..+10) as a first sentiment column.
//   Both are written into the canonical `documents` / `mentions` tables (store layer), which
//   begins populating the relational schema ahead of the richer extractors in later stages.
//
//   This stage is intentionally dependency-free and deterministic (good for tests). The tone
//   lexicon is a small finance/news starter set; mod_tone (Stage 5) supersedes it with GCAM.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use config::Config;
use log::{error, info};

use crate::cfg::get_database_filename;
use crate::document::Document;
use crate::store::records::{insert_mention, upsert_document, DocumentRow};

pub const PLUGIN_NAME: &str = "mod_mentions";

/// Minimum text length (chars) before we bother computing tone / a mention.
const MIN_TEXT_LEN: usize = 80;

pub fn process_data(
    tx: Sender<Document>,
    rx: Receiver<Document>,
    config: &Config,
    _api_mutexes: &mut HashMap<String, Arc<Mutex<isize>>>,
) {
    info!("{}: Starting mentions + tone extraction.", PLUGIN_NAME);
    let db_path = get_database_filename(config);

    // Open one connection for the lifetime of this stage.
    let conn = match crate::store::open(&db_path) {
        Ok(c) => Some(c),
        Err(e) => {
            error!("{}: cannot open store '{}': {} — forwarding docs unmodified.", PLUGIN_NAME, db_path, e);
            None
        }
    };

    let mut count: usize = 0;
    for mut doc in rx {
        if let Some(ref conn) = conn {
            if doc.text.len() >= MIN_TEXT_LEN {
                let (tone, word_count) = lexicon_tone(&doc.text);
                let did = doc_id_for(&doc);
                let cluster = cluster_id_for(&doc, &did);

                // Record tone on the analysis sidecar too (kept in sync with the table).
                let mut analysis = doc.analysis.take().unwrap_or_default();
                let mut ts = analysis.tone.take().unwrap_or_default();
                ts.tone = tone;
                ts.word_count = word_count as usize;
                analysis.tone = Some(ts);
                doc.analysis = Some(analysis);

                let row = DocumentRow {
                    doc_id: did.clone(),
                    url: doc.url.clone(),
                    source: doc.source_name.first().cloned().unwrap_or_default(),
                    title: doc.title.clone(),
                    lang: String::new(),
                    pubdate_ms: doc.publish_date_ms,
                    pubdate: doc.publish_date.clone(),
                    plugin: doc.module.clone(),
                    section: doc.section_name.clone(),
                    cluster_id: cluster.clone(),
                    tone,
                    word_count: word_count as i64,
                };
                if let Err(e) = upsert_document(conn, &row) {
                    error!("{}: {}", PLUGIN_NAME, e);
                }
                let src = doc.source_name.first().cloned().unwrap_or_else(|| doc.module.clone());
                if let Err(e) = insert_mention(conn, &cluster, &did, doc.publish_date_ms, &src, 100.0) {
                    error!("{}: {}", PLUGIN_NAME, e);
                }
                count += 1;
            }
        }

        if let Err(e) = tx.send(doc) {
            error!("{}: when forwarding doc: {}", PLUGIN_NAME, e);
        }
    }
    info!("{}: Completed. Recorded {} document mention(s)/tone.", PLUGIN_NAME, count);
}

/// Stable document id: prefer the site-provided `unique_id`, else a hash of the URL.
pub fn doc_id_for(doc: &Document) -> String {
    if !doc.unique_id.trim().is_empty() {
        return doc.unique_id.trim().to_string();
    }
    format!("u{:016x}", fnv1a(&doc.url))
}

/// The cluster (story/event) a document belongs to. mod_dedupe tags near-duplicates with
/// `duplicate_of` = the unique_id of the representative document; non-duplicates form their
/// own singleton cluster keyed by their own doc_id.
pub fn cluster_id_for(doc: &Document, did: &str) -> String {
    match doc.classification.get("duplicate_of") {
        Some(rep) if !rep.trim().is_empty() => rep.trim().to_string(),
        _ => did.to_string(),
    }
}

fn fnv1a(s: &str) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut h = OFFSET;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(PRIME);
    }
    h
}

/// Small positive/negative finance-news lexicons (lowercased word stems).
const POSITIVE: &[&str] = &[
    "gain", "gains", "rise", "rises", "rose", "growth", "grew", "profit", "profits", "surplus",
    "boost", "boosts", "upgrade", "upgraded", "approve", "approved", "approval", "strong",
    "strengthen", "recovery", "recover", "expansion", "expand", "rally", "surge", "surged",
    "improve", "improved", "beat", "outperform", "record", "robust", "ease", "eased",
];
const NEGATIVE: &[&str] = &[
    "loss", "losses", "fall", "falls", "fell", "decline", "declined", "fraud", "penalty",
    "penalties", "default", "defaults", "ban", "banned", "crisis", "slump", "slumped",
    "downgrade", "downgraded", "weak", "weaken", "deficit", "probe", "fine", "fined",
    "plunge", "plunged", "crash", "slowdown", "lawsuit", "scam", "breach", "violation", "miss",
];

/// Cheap lexicon tone in [-10, 10] plus the word count.
/// tone = 10 * (pos - neg) / (pos + neg); 0 when no lexicon words are present.
pub fn lexicon_tone(text: &str) -> (f64, u64) {
    let mut pos = 0u64;
    let mut neg = 0u64;
    let mut words = 0u64;
    for raw in text.split(|c: char| !c.is_alphanumeric()) {
        if raw.is_empty() {
            continue;
        }
        words += 1;
        let w = raw.to_lowercase();
        if POSITIVE.contains(&w.as_str()) {
            pos += 1;
        } else if NEGATIVE.contains(&w.as_str()) {
            neg += 1;
        }
    }
    let tone = if pos + neg == 0 {
        0.0
    } else {
        10.0 * (pos as f64 - neg as f64) / (pos as f64 + neg as f64)
    };
    (tone, words)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;
    use rusqlite::Connection;

    #[test]
    fn test_tone_positive_negative_neutral() {
        let (pos, _) = lexicon_tone("The bank reported strong profit growth and a record surplus.");
        assert!(pos > 0.0, "expected positive tone, got {}", pos);

        let (neg, _) = lexicon_tone("Regulator imposes penalty after fraud and default; shares crash.");
        assert!(neg < 0.0, "expected negative tone, got {}", neg);

        let (neutral, wc) = lexicon_tone("The committee met on Tuesday to review the calendar.");
        assert_eq!(neutral, 0.0);
        assert!(wc > 0);
    }

    #[test]
    fn test_tone_bounds() {
        let (t, _) = lexicon_tone("gain gain gain profit surge rally");
        assert!(t <= 10.0 && t >= -10.0);
        assert_eq!(t, 10.0, "all-positive should be +10");
    }

    #[test]
    fn test_doc_id_prefers_unique_id() {
        let mut d = Document::default();
        d.unique_id = "RBI/2026/123".into();
        assert_eq!(doc_id_for(&d), "RBI/2026/123");

        let mut d2 = Document::default();
        d2.url = "https://x/y".into();
        let id = doc_id_for(&d2);
        assert!(id.starts_with('u') && id.len() == 17);
    }

    #[test]
    fn test_cluster_id_uses_duplicate_of() {
        let mut d = Document::default();
        d.classification.insert("duplicate_of".into(), "REP-1".into());
        assert_eq!(cluster_id_for(&d, "D9"), "REP-1");

        let d2 = Document::default();
        assert_eq!(cluster_id_for(&d2, "D9"), "D9", "singleton clusters on own id");
    }

    #[test]
    fn test_mentions_share_cluster_for_duplicates() {
        // Two docs in the same dedup cluster should produce two mentions of one cluster.
        let c: Connection = {
            let conn = Connection::open_in_memory().unwrap();
            store::migrate(&conn).unwrap();
            conn
        };
        // representative
        let rep = DocumentRow { doc_id: "REP".into(), cluster_id: "REP".into(), ..Default::default() };
        upsert_document(&c, &rep).unwrap();
        insert_mention(&c, "REP", "REP", 1, "wire", 100.0).unwrap();
        // duplicate pointing at REP
        let dup = DocumentRow { doc_id: "DUP".into(), cluster_id: "REP".into(), ..Default::default() };
        upsert_document(&c, &dup).unwrap();
        insert_mention(&c, "REP", "DUP", 2, "outlet", 100.0).unwrap();

        let mentions: i64 = c.query_row("SELECT COUNT(*) FROM mentions WHERE cluster_id='REP'", [], |r| r.get(0)).unwrap();
        assert_eq!(mentions, 2);
    }
}
