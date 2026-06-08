// file: mod_geocode.rs
// Purpose:
//   Phase-2 geocoder (roadmap Stage 6 / D8, GDELT GKG locations analog). Resolves place names
//   mentioned in article text to coordinates using an embedded gazetteer of major Indian cities
//   and states plus a handful of global financial centres. Deterministic, dependency-free and
//   unit-testable; results land on `doc.analysis.locations` and are persisted to the `locations`
//   table by `mod_emit_tables`.
//
//   The embedded gazetteer is a high-precision starter set. The roadmap's full World Cities /
//   India PIN gazetteer is intended to load via a calamine-backed batch feed into a `geo` table;
//   that bulk loader is deferred until the data files are available, and `geocode_text` can then
//   consult that table in addition to this seed list without changing the plugin contract.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use config::Config;
use log::{error, info};

use crate::analysis::GeoMention;
use crate::document::Document;

pub const PLUGIN_NAME: &str = "mod_geocode";

const MIN_TEXT_LEN: usize = 40;

/// A gazetteer entry: (name, lat, lon, adm1 (state/region), country ISO2).
struct Place {
    name: &'static str,
    lat: f64,
    lon: f64,
    adm1: &'static str,
    country: &'static str,
}

/// Embedded high-precision gazetteer. Longer (multi-word) names are listed so they match before
/// any substring collisions; `geocode_text` also sorts matches by descending name length to
/// prefer "New Delhi" over a hypothetical "Delhi" overlap.
const GAZETTEER: &[Place] = &[
    // Major Indian cities
    Place { name: "New Delhi", lat: 28.6139, lon: 77.2090, adm1: "Delhi", country: "IN" },
    Place { name: "Mumbai", lat: 19.0760, lon: 72.8777, adm1: "Maharashtra", country: "IN" },
    Place { name: "Bengaluru", lat: 12.9716, lon: 77.5946, adm1: "Karnataka", country: "IN" },
    Place { name: "Bangalore", lat: 12.9716, lon: 77.5946, adm1: "Karnataka", country: "IN" },
    Place { name: "Kolkata", lat: 22.5726, lon: 88.3639, adm1: "West Bengal", country: "IN" },
    Place { name: "Chennai", lat: 13.0827, lon: 80.2707, adm1: "Tamil Nadu", country: "IN" },
    Place { name: "Hyderabad", lat: 17.3850, lon: 78.4867, adm1: "Telangana", country: "IN" },
    Place { name: "Pune", lat: 18.5204, lon: 73.8567, adm1: "Maharashtra", country: "IN" },
    Place { name: "Ahmedabad", lat: 23.0225, lon: 72.5714, adm1: "Gujarat", country: "IN" },
    Place { name: "Surat", lat: 21.1702, lon: 72.8311, adm1: "Gujarat", country: "IN" },
    Place { name: "Jaipur", lat: 26.9124, lon: 75.7873, adm1: "Rajasthan", country: "IN" },
    Place { name: "Lucknow", lat: 26.8467, lon: 80.9462, adm1: "Uttar Pradesh", country: "IN" },
    Place { name: "Kanpur", lat: 26.4499, lon: 80.3319, adm1: "Uttar Pradesh", country: "IN" },
    Place { name: "Nagpur", lat: 21.1458, lon: 79.0882, adm1: "Maharashtra", country: "IN" },
    Place { name: "Indore", lat: 22.7196, lon: 75.8577, adm1: "Madhya Pradesh", country: "IN" },
    Place { name: "Bhopal", lat: 23.2599, lon: 77.4126, adm1: "Madhya Pradesh", country: "IN" },
    Place { name: "Patna", lat: 25.5941, lon: 85.1376, adm1: "Bihar", country: "IN" },
    Place { name: "Chandigarh", lat: 30.7333, lon: 76.7794, adm1: "Chandigarh", country: "IN" },
    Place { name: "Kochi", lat: 9.9312, lon: 76.2673, adm1: "Kerala", country: "IN" },
    Place { name: "Gurugram", lat: 28.4595, lon: 77.0266, adm1: "Haryana", country: "IN" },
    Place { name: "Gurgaon", lat: 28.4595, lon: 77.0266, adm1: "Haryana", country: "IN" },
    Place { name: "Noida", lat: 28.5355, lon: 77.3910, adm1: "Uttar Pradesh", country: "IN" },
    // Indian states / UTs (region-level)
    Place { name: "Maharashtra", lat: 19.7515, lon: 75.7139, adm1: "Maharashtra", country: "IN" },
    Place { name: "Gujarat", lat: 22.2587, lon: 71.1924, adm1: "Gujarat", country: "IN" },
    Place { name: "Karnataka", lat: 15.3173, lon: 75.7139, adm1: "Karnataka", country: "IN" },
    Place { name: "Tamil Nadu", lat: 11.1271, lon: 78.6569, adm1: "Tamil Nadu", country: "IN" },
    Place { name: "Kerala", lat: 10.8505, lon: 76.2711, adm1: "Kerala", country: "IN" },
    Place { name: "Telangana", lat: 18.1124, lon: 79.0193, adm1: "Telangana", country: "IN" },
    Place { name: "West Bengal", lat: 22.9868, lon: 87.8550, adm1: "West Bengal", country: "IN" },
    Place { name: "Uttar Pradesh", lat: 26.8467, lon: 80.9462, adm1: "Uttar Pradesh", country: "IN" },
    Place { name: "Rajasthan", lat: 27.0238, lon: 74.2179, adm1: "Rajasthan", country: "IN" },
    Place { name: "Punjab", lat: 31.1471, lon: 75.3412, adm1: "Punjab", country: "IN" },
    // Global financial centres
    Place { name: "New York", lat: 40.7128, lon: -74.0060, adm1: "New York", country: "US" },
    Place { name: "London", lat: 51.5074, lon: -0.1278, adm1: "England", country: "GB" },
    Place { name: "Singapore", lat: 1.3521, lon: 103.8198, adm1: "Singapore", country: "SG" },
    Place { name: "Dubai", lat: 25.2048, lon: 55.2708, adm1: "Dubai", country: "AE" },
    Place { name: "Hong Kong", lat: 22.3193, lon: 114.1694, adm1: "Hong Kong", country: "HK" },
    Place { name: "Tokyo", lat: 35.6762, lon: 139.6503, adm1: "Tokyo", country: "JP" },
    Place { name: "Frankfurt", lat: 50.1109, lon: 8.6821, adm1: "Hesse", country: "DE" },
];

pub fn process_data(
    tx: Sender<Document>,
    rx: Receiver<Document>,
    _config: &Config,
    _api_mutexes: &mut HashMap<String, Arc<Mutex<isize>>>,
) {
    info!("{}: Starting geocoding ({} gazetteer entries).", PLUGIN_NAME, GAZETTEER.len());
    let mut docs = 0usize;
    let mut hits = 0usize;
    for mut doc in rx {
        if doc.text.len() >= MIN_TEXT_LEN {
            let locs = geocode_text(&doc.text);
            if !locs.is_empty() {
                let mut analysis = doc.analysis.take().unwrap_or_default();
                hits += locs.len();
                analysis.locations.extend(locs);
                doc.analysis = Some(analysis);
                docs += 1;
            }
        }
        if let Err(e) = tx.send(doc) {
            error!("{}: when forwarding doc: {}", PLUGIN_NAME, e);
        }
    }
    info!("{}: Completed. Geocoded {} place mention(s) across {} document(s).", PLUGIN_NAME, hits, docs);
}

/// Resolve gazetteer place names appearing as whole words in `text`. Each place is reported at
/// most once (earliest offset). Matching is case-insensitive and boundary-aware so "London" does
/// not match inside "Londonderry".
pub fn geocode_text(text: &str) -> Vec<GeoMention> {
    let lower = text.to_lowercase();
    let bytes = lower.as_bytes();

    // Prefer longer names first so multi-word places win over any shorter overlap.
    let mut order: Vec<&Place> = GAZETTEER.iter().collect();
    order.sort_by_key(|p| std::cmp::Reverse(p.name.len()));

    let mut out: Vec<GeoMention> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for p in order {
        if seen.contains(&p.name) {
            continue;
        }
        let needle = p.name.to_lowercase();
        if let Some(off) = find_whole_word(&lower, bytes, &needle) {
            seen.push(p.name);
            out.push(GeoMention {
                name: p.name.to_string(),
                feature_id: Some(format!("{}/{}", p.country, p.name.replace(' ', "_"))),
                lat: Some(p.lat),
                lon: Some(p.lon),
                country: p.country.to_string(),
                adm1: p.adm1.to_string(),
                adm2: String::new(),
                char_offset: off,
            });
        }
    }
    out.sort_by_key(|m| m.char_offset);
    out
}

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

    fn names(text: &str) -> Vec<String> {
        geocode_text(text).into_iter().map(|m| m.name).collect()
    }

    #[test]
    fn test_geocodes_indian_cities() {
        let g = geocode_text("The RBI headquarters in Mumbai issued a circular; offices in New Delhi reacted.");
        let n: Vec<&str> = g.iter().map(|m| m.name.as_str()).collect();
        assert!(n.contains(&"Mumbai"), "got {:?}", n);
        assert!(n.contains(&"New Delhi"), "got {:?}", n);
        let mum = g.iter().find(|m| m.name == "Mumbai").unwrap();
        assert_eq!(mum.country, "IN");
        assert_eq!(mum.adm1, "Maharashtra");
        assert!(mum.lat.unwrap() > 18.0 && mum.lat.unwrap() < 20.0);
    }

    #[test]
    fn test_global_centre_and_feature_id() {
        let g = geocode_text("Shares fell in London and New York after the announcement was made public.");
        let london = g.iter().find(|m| m.name == "London").expect("London");
        assert_eq!(london.country, "GB");
        assert_eq!(london.feature_id.as_deref(), Some("GB/London"));
    }

    #[test]
    fn test_whole_word_boundary() {
        // "Londonderry" must not match "London".
        let n = names("A report from Londonderry was published in the morning paper today here.");
        assert!(!n.contains(&"London".to_string()), "boundary failure: {:?}", n);
    }

    #[test]
    fn test_each_place_once_and_sorted() {
        let g = geocode_text("Mumbai, Mumbai, and again Mumbai dominated the headlines across the country.");
        assert_eq!(g.iter().filter(|m| m.name == "Mumbai").count(), 1);
    }

    #[test]
    fn test_no_match_neutral_text() {
        assert!(geocode_text("The committee reviewed the quarterly report and adjourned the meeting.").is_empty());
    }
}
