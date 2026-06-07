// file: mod_dedupe.rs
// Near-duplicate detection for the data-processing pipeline.
//
// News is heavily syndicated: the same wire story (AP, Reuters, PTI) is republished by
// many outlets with minor edits, so exact-hash dedup is insufficient. This stage computes
// a 64-bit SimHash of each document's text and flags a document as a near-duplicate when
// its Hamming distance to a previously seen document is within a small threshold.
//
// Behaviour is non-destructive: duplicates are tagged (classification["near_duplicate"]
// = "true" with the matched unique_id) and still forwarded, so downstream persistence and
// the completed-urls table remain consistent. The document URL is also canonicalized.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use config::Config;
use log::{error, info};
use crate::document::Document;
use crate::utils::canonicalize_url;

pub(crate) const PLUGIN_NAME: &str = "mod_dedupe";

/// Maximum Hamming distance (out of 64 bits) at which two documents are considered
/// near-duplicates. 0 = identical SimHash; small values catch minor edits.
const SIMHASH_THRESHOLD: u32 = 3;
/// Documents shorter than this (chars) are not near-dup checked — too little signal.
const MIN_TEXT_FOR_SIMHASH: usize = 200;

pub(crate) fn process_data(
    tx: Sender<Document>,
    rx: Receiver<Document>,
    _config: &Config,
    _api_mutexes: &mut HashMap<String, Arc<Mutex<isize>>>,
) {
    info!("{}: Starting near-duplicate detection.", PLUGIN_NAME);

    // Seen SimHashes paired with the unique_id of the document that produced them.
    let mut seen: Vec<(u64, String)> = Vec::new();
    let mut dup_count: usize = 0;

    for mut doc in rx {
        // Normalize the URL for stable storage/dedup keys.
        if !doc.url.is_empty() {
            doc.url = canonicalize_url(&doc.url);
        }

        if doc.text.len() >= MIN_TEXT_FOR_SIMHASH {
            let hash = simhash(&doc.text);
            if let Some(matched_id) = seen
                .iter()
                .find(|(h, _)| hamming_distance(*h, hash) <= SIMHASH_THRESHOLD)
                .map(|(_, id)| id.clone())
            {
                dup_count += 1;
                doc.classification.insert("near_duplicate".to_string(), "true".to_string());
                doc.classification.insert("duplicate_of".to_string(), matched_id.clone());
                info!(
                    "{}: Near-duplicate detected (url={}) of unique_id={}",
                    PLUGIN_NAME, doc.url, matched_id
                );
            } else {
                seen.push((hash, doc.unique_id.clone()));
            }
        }

        if let Err(e) = tx.send(doc) {
            error!("{}: When sending processed doc via tx: {}", PLUGIN_NAME, e);
        }
    }

    info!("{}: Completed. Flagged {} near-duplicate(s).", PLUGIN_NAME, dup_count);
}

/// Compute a 64-bit SimHash over whitespace-delimited word tokens of `text`.
pub fn simhash(text: &str) -> u64 {
    let mut counts = [0i32; 64];

    for token in text.split_whitespace() {
        // Normalize tokens to reduce sensitivity to case/punctuation.
        let norm: String = token
            .chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(|c| c.to_lowercase())
            .collect();
        if norm.is_empty() {
            continue;
        }
        let h = token_hash(&norm);
        for (i, count) in counts.iter_mut().enumerate() {
            if (h >> i) & 1 == 1 {
                *count += 1;
            } else {
                *count -= 1;
            }
        }
    }

    let mut fingerprint: u64 = 0;
    for (i, count) in counts.iter().enumerate() {
        if *count > 0 {
            fingerprint |= 1u64 << i;
        }
    }
    fingerprint
}

/// Stable per-token 64-bit hash (FNV-1a) — independent of the process's RandomState so
/// SimHashes are comparable and reproducible across runs.
fn token_hash(token: &str) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for byte in token.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Number of differing bits between two 64-bit fingerprints.
pub fn hamming_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_text_same_hash() {
        let t = "the quick brown fox jumped over the lazy dog repeatedly in the field";
        assert_eq!(simhash(t), simhash(t));
        assert_eq!(hamming_distance(simhash(t), simhash(t)), 0);
    }

    #[test]
    fn test_minor_edit_is_near_duplicate() {
        let a = "Reserve Bank of India raised the repo rate by 25 basis points today citing \
                 persistent inflation pressures across food and fuel categories nationwide.";
        // Same story with a one-word edit.
        let b = "Reserve Bank of India raised the repo rate by 25 basis points today citing \
                 persistent inflation pressures across food and fuel categories countrywide.";
        let dist = hamming_distance(simhash(a), simhash(b));
        assert!(dist <= SIMHASH_THRESHOLD, "expected near-dup, distance was {}", dist);
    }

    #[test]
    fn test_different_text_not_near_duplicate() {
        let a = "Reserve Bank of India raised the repo rate by 25 basis points today citing \
                 persistent inflation pressures across food and fuel categories nationwide.";
        let b = "Scientists in Antarctica discovered a new species of cold-water jellyfish \
                 thriving beneath the ice shelf during a summer research expedition this year.";
        let dist = hamming_distance(simhash(a), simhash(b));
        assert!(dist > SIMHASH_THRESHOLD, "expected distinct, distance was {}", dist);
    }

    #[test]
    fn test_token_hash_stable() {
        // Must be deterministic across calls (and processes).
        assert_eq!(token_hash("rupee"), token_hash("rupee"));
        assert_ne!(token_hash("rupee"), token_hash("dollar"));
    }
}
