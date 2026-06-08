// file: mod_extract_quant.rs
// Purpose:
//   Phase-1 deterministic quantitative extractor (roadmap Stage 5 / D7). Pulls structured
//   numeric facts out of article text with regexes only — no model, fully deterministic and
//   testable — and writes them onto the `DocAnalysis` sidecar:
//     * amounts  — monetary values, normalised to a base unit (crore=1e7, lakh=1e5,
//                  million=1e6, billion=1e9, thousand=1e3, trillion=1e12), currency-tagged.
//     * counts   — "<number> <plural-noun>" quantities (GDELT V1COUNTS analog).
//     * dates    — calendar dates referenced in the text (distinct from the publish date).
//   The canonical-table persistence of these facts is done later by `mod_emit_tables`; this
//   plugin only enriches `doc.analysis` and forwards the document.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};

use config::Config;
use log::{error, info};
use regex::Regex;

use crate::analysis::{AmountMention, CountMention, DateRef};
use crate::document::Document;

pub const PLUGIN_NAME: &str = "mod_extract_quant";

/// Minimum text length before extraction is attempted.
const MIN_TEXT_LEN: usize = 40;

pub fn process_data(
    tx: Sender<Document>,
    rx: Receiver<Document>,
    _config: &Config,
    _api_mutexes: &mut HashMap<String, Arc<Mutex<isize>>>,
) {
    info!("{}: Starting quantitative extraction (amounts/counts/dates).", PLUGIN_NAME);
    let mut docs = 0usize;
    let mut facts = 0usize;

    for mut doc in rx {
        if doc.text.len() >= MIN_TEXT_LEN {
            let amounts = extract_amounts(&doc.text);
            let counts = extract_counts(&doc.text);
            let dates = extract_dates(&doc.text);
            if !amounts.is_empty() || !counts.is_empty() || !dates.is_empty() {
                let mut analysis = doc.analysis.take().unwrap_or_default();
                facts += amounts.len() + counts.len() + dates.len();
                analysis.amounts.extend(amounts);
                analysis.counts.extend(counts);
                analysis.dates_referenced.extend(dates);
                doc.analysis = Some(analysis);
                docs += 1;
            }
        }
        if let Err(e) = tx.send(doc) {
            error!("{}: when forwarding doc: {}", PLUGIN_NAME, e);
        }
    }
    info!("{}: Completed. Enriched {} document(s) with {} quantitative fact(s).", PLUGIN_NAME, docs, facts);
}

// ---------------------------------------------------------------------------
// Amounts
// ---------------------------------------------------------------------------

/// Currency symbol/word (optional) + number + scale word (optional). We keep a match only
/// when it carries a currency or a scale word, so bare numbers are not treated as amounts.
fn amount_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)(?:(₹|rs\.?|inr|usd|us\$|\$|eur|€|gbp|£)\s*)?([0-9][0-9,]*(?:\.[0-9]+)?)\s*(crores?|cr\b|lakhs?|lacs?|million|mn\b|billion|bn\b|trillion|tn\b|thousand)?",
        )
        .expect("amount regex")
    })
}

/// Multiplier for a (lowercased) scale word.
fn scale_multiplier(scale: &str) -> Option<f64> {
    let s = scale.trim_end_matches('.').to_lowercase();
    let m = match s.as_str() {
        "crore" | "crores" | "cr" => 1e7,
        "lakh" | "lakhs" | "lac" | "lacs" => 1e5,
        "thousand" => 1e3,
        "million" | "mn" => 1e6,
        "billion" | "bn" => 1e9,
        "trillion" | "tn" => 1e12,
        _ => return None,
    };
    Some(m)
}

/// Normalise a currency token to an ISO-ish code.
fn normalise_currency(cur: &str) -> String {
    match cur.trim_end_matches('.').to_lowercase().as_str() {
        "₹" | "rs" | "inr" => "INR".to_string(),
        "$" | "us$" | "usd" => "USD".to_string(),
        "€" | "eur" => "EUR".to_string(),
        "£" | "gbp" => "GBP".to_string(),
        other => other.to_uppercase(),
    }
}

pub fn extract_amounts(text: &str) -> Vec<AmountMention> {
    let mut out = Vec::new();
    for caps in amount_re().captures_iter(text) {
        let currency_tok = caps.get(1).map(|m| m.as_str());
        let num_tok = match caps.get(2) {
            Some(m) => m.as_str(),
            None => continue,
        };
        let scale_tok = caps.get(3).map(|m| m.as_str()).filter(|s| !s.is_empty());

        // Require a currency or a scale word; otherwise it's just a bare number.
        if currency_tok.is_none() && scale_tok.is_none() {
            continue;
        }
        let raw: f64 = match num_tok.replace(',', "").parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let unit = scale_tok.map(|s| s.trim_end_matches('.').to_lowercase()).unwrap_or_default();
        let mult = scale_tok.and_then(scale_multiplier).unwrap_or(1.0);
        out.push(AmountMention {
            value: raw * mult,
            currency: currency_tok.map(normalise_currency).unwrap_or_default(),
            unit,
            object: String::new(),
            char_offset: caps.get(0).map(|m| m.start()).unwrap_or(0),
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Counts
// ---------------------------------------------------------------------------

fn count_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b([0-9][0-9,]*)\s+([a-z]{3,})\b").expect("count regex")
    })
}

/// Words that follow a number but are *scales/currencies* (handled as amounts) — never counts.
const COUNT_STOP: &[&str] = &[
    "crore", "crores", "lakh", "lakhs", "lac", "lacs", "million", "billion", "trillion",
    "thousand", "percent", "per", "rs", "inr", "usd", "eur", "gbp",
];

pub fn extract_counts(text: &str) -> Vec<CountMention> {
    let mut out = Vec::new();
    for caps in count_re().captures_iter(text) {
        let num_tok = &caps[1];
        let noun = caps[2].to_lowercase();
        // Only crude plurals ("banks", "companies"), and not scale/currency words.
        if !noun.ends_with('s') || COUNT_STOP.contains(&noun.as_str()) {
            continue;
        }
        let number: f64 = match num_tok.replace(',', "").parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        out.push(CountMention {
            count_type: String::new(),
            number,
            object: noun,
            char_offset: caps.get(0).map(|m| m.start()).unwrap_or(0),
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Dates referenced in the text
// ---------------------------------------------------------------------------

fn month_num(mon: &str) -> Option<u32> {
    let m = match &mon.to_lowercase()[..3.min(mon.len())] {
        "jan" => 1, "feb" => 2, "mar" => 3, "apr" => 4, "may" => 5, "jun" => 6,
        "jul" => 7, "aug" => 8, "sep" => 9, "oct" => 10, "nov" => 11, "dec" => 12,
        _ => return None,
    };
    Some(m)
}

/// ISO dates and month-name dates ("10 June 2026", "June 10, 2026", "June 2026").
fn iso_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b(\d{4})-(\d{2})-(\d{2})\b").expect("iso date regex"))
}
fn dmy_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b(\d{1,2})\s+(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*\.?\s+(\d{4})\b")
            .expect("dmy regex")
    })
}
fn mdy_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*\.?\s+(\d{1,2})(?:st|nd|rd|th)?,?\s+(\d{4})\b")
            .expect("mdy regex")
    })
}

pub fn extract_dates(text: &str) -> Vec<DateRef> {
    let mut out = Vec::new();

    for c in iso_re().captures_iter(text) {
        out.push(DateRef {
            resolution: "day".into(),
            year: c[1].parse().unwrap_or(0),
            month: c[2].parse().unwrap_or(0),
            day: c[3].parse().unwrap_or(0),
            char_offset: c.get(0).map(|m| m.start()).unwrap_or(0),
        });
    }
    for c in dmy_re().captures_iter(text) {
        if let Some(month) = month_num(&c[2]) {
            out.push(DateRef {
                resolution: "day".into(),
                year: c[3].parse().unwrap_or(0),
                month,
                day: c[1].parse().unwrap_or(0),
                char_offset: c.get(0).map(|m| m.start()).unwrap_or(0),
            });
        }
    }
    for c in mdy_re().captures_iter(text) {
        if let Some(month) = month_num(&c[1]) {
            out.push(DateRef {
                resolution: "day".into(),
                year: c[3].parse().unwrap_or(0),
                month,
                day: c[2].parse().unwrap_or(0),
                char_offset: c.get(0).map(|m| m.start()).unwrap_or(0),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_amount_rupees_crore() {
        let a = extract_amounts("The fine of Rs 5,000 crore was imposed on the bank.");
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].currency, "INR");
        assert_eq!(a[0].unit, "crore");
        assert_eq!(a[0].value, 5000.0 * 1e7);
    }

    #[test]
    fn test_amount_symbol_lakh_and_dollar_million() {
        let a = extract_amounts("They paid ₹1.5 lakh and raised $2 million in funding.");
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].currency, "INR");
        assert_eq!(a[0].value, 1.5 * 1e5);
        assert_eq!(a[1].currency, "USD");
        assert_eq!(a[1].value, 2.0 * 1e6);
    }

    #[test]
    fn test_amount_scale_without_currency() {
        let a = extract_amounts("Production rose to 10 crore units this quarter.");
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].currency, "");
        assert_eq!(a[0].value, 10.0 * 1e7);
    }

    #[test]
    fn test_bare_number_is_not_amount() {
        let a = extract_amounts("There were 250 people present at the venue today.");
        assert!(a.is_empty(), "bare number must not be an amount: {:?}", a);
    }

    #[test]
    fn test_counts_plural_noun() {
        let c = extract_counts("The regulator inspected 15 banks and 3 companies last week.");
        let objects: Vec<&str> = c.iter().map(|m| m.object.as_str()).collect();
        assert!(objects.contains(&"banks"), "got {:?}", objects);
        assert!(objects.contains(&"companies"), "got {:?}", objects);
        assert_eq!(c.iter().find(|m| m.object == "banks").unwrap().number, 15.0);
    }

    #[test]
    fn test_counts_skip_scale_words() {
        // "5 crore" is an amount, not a count; "crore" must not surface as a count object.
        let c = extract_counts("A grant of 5 crore was announced.");
        assert!(c.iter().all(|m| m.object != "crore"), "scale word leaked into counts: {:?}", c);
    }

    #[test]
    fn test_dates_iso_and_names() {
        let d = extract_dates("Filed on 2026-06-10, effective 1 July 2026 and June 15, 2026.");
        assert!(d.iter().any(|x| x.year == 2026 && x.month == 6 && x.day == 10));
        assert!(d.iter().any(|x| x.year == 2026 && x.month == 7 && x.day == 1));
        assert!(d.iter().any(|x| x.year == 2026 && x.month == 6 && x.day == 15));
    }
}
