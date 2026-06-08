// file: mod_tone.rs
// Purpose:
//   Phase-1 document tone panel (roadmap Stage 5 / D5, GDELT V1.5TONE analog). Supersedes the
//   single cheap tone number produced by `mod_mentions` with a full six-field panel
//   (tone / positive / negative / polarity / activity / word_count) computed from finance-news
//   lexicons, plus a small GCAM-style sub-panel emitted as `GcamScore` rows. Deterministic and
//   dependency-free. Writes onto `doc.analysis.tone` and `doc.analysis.gcam`; persistence is
//   done later by `mod_emit_tables`.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use config::Config;
use log::{error, info};

use crate::analysis::{GcamScore, ToneScores};
use crate::document::Document;

pub const PLUGIN_NAME: &str = "mod_tone";

const MIN_TEXT_LEN: usize = 40;

/// Positive-sentiment finance lexicon (lowercased word stems, matched whole-word).
const POSITIVE: &[&str] = &[
    "gain", "gains", "gained", "rise", "rises", "rose", "rising", "growth", "grew", "grow",
    "profit", "profits", "profitable", "surplus", "boost", "boosts", "boosted", "upgrade",
    "upgraded", "approve", "approved", "approval", "strong", "stronger", "strengthen", "recovery",
    "recover", "recovered", "expansion", "expand", "expanded", "rally", "rallied", "surge",
    "surged", "improve", "improved", "improvement", "beat", "outperform", "record", "robust",
    "ease", "eased", "easing", "optimistic", "upbeat", "favourable", "favorable", "buoyant",
];
/// Negative-sentiment finance lexicon.
const NEGATIVE: &[&str] = &[
    "loss", "losses", "fall", "falls", "fell", "falling", "decline", "declined", "declines",
    "fraud", "penalty", "penalties", "default", "defaults", "defaulted", "ban", "banned", "crisis",
    "slump", "slumped", "downgrade", "downgraded", "weak", "weaker", "weaken", "deficit", "probe",
    "fine", "fined", "plunge", "plunged", "crash", "crashed", "slowdown", "lawsuit", "scam",
    "breach", "violation", "violations", "miss", "missed", "shortfall", "distress", "insolvent",
    "bankrupt", "downturn", "sluggish", "pessimistic", "bearish",
];
/// Activity/intensity lexicon (GDELT "activity reference density" analog).
const ACTIVE: &[&str] = &[
    "announce", "announced", "launch", "launched", "impose", "imposed", "raise", "raised", "cut",
    "approve", "investigate", "investigated", "order", "ordered", "direct", "directed", "issue",
    "issued", "file", "filed", "acquire", "acquired", "merge", "merged", "sign", "signed",
    "appoint", "appointed", "resign", "resigned", "ban", "fine", "penalise", "penalize", "seize",
    "seized", "freeze", "froze",
];

pub fn process_data(
    tx: Sender<Document>,
    rx: Receiver<Document>,
    _config: &Config,
    _api_mutexes: &mut HashMap<String, Arc<Mutex<isize>>>,
) {
    info!("{}: Starting tone-panel scoring.", PLUGIN_NAME);
    let mut docs = 0usize;
    for mut doc in rx {
        if doc.text.len() >= MIN_TEXT_LEN {
            let scores = score_tone(&doc.text);
            let gcam = gcam_for(&scores);
            let mut analysis = doc.analysis.take().unwrap_or_default();
            analysis.tone = Some(scores);
            analysis.gcam.extend(gcam);
            doc.analysis = Some(analysis);
            docs += 1;
        }
        if let Err(e) = tx.send(doc) {
            error!("{}: when forwarding doc: {}", PLUGIN_NAME, e);
        }
    }
    info!("{}: Completed. Scored tone for {} document(s).", PLUGIN_NAME, docs);
}

/// Compute the six-field tone panel. Percentages are over total word count.
/// tone = positive% - negative% (in [-100, 100], then rescaled to GDELT's -10..10 convention
/// is left to consumers; we keep the percentage-point form consistent with GKG V1.5TONE).
pub fn score_tone(text: &str) -> ToneScores {
    let mut words = 0usize;
    let mut pos = 0usize;
    let mut neg = 0usize;
    let mut act = 0usize;
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
        if ACTIVE.contains(&w.as_str()) {
            act += 1;
        }
    }
    if words == 0 {
        return ToneScores::default();
    }
    let wf = words as f64;
    let positive = 100.0 * pos as f64 / wf;
    let negative = 100.0 * neg as f64 / wf;
    ToneScores {
        tone: positive - negative,
        positive,
        negative,
        polarity: positive + negative,
        activity: 100.0 * act as f64 / wf,
        self_group: 0.0,
        word_count: words,
    }
}

/// Emit a small GCAM-style sub-panel mirroring the tone fields, so the `gcam` table is
/// populated in the same GDELT-compatible `dict_id`/`dim_id`/`key`(c|v) shape used later.
fn gcam_for(s: &ToneScores) -> Vec<GcamScore> {
    vec![
        GcamScore { dict_id: "finlex".into(), dim_id: "wc".into(), key: "c".into(), score: s.word_count as f64 },
        GcamScore { dict_id: "finlex".into(), dim_id: "tone".into(), key: "v".into(), score: s.tone },
        GcamScore { dict_id: "finlex".into(), dim_id: "pos".into(), key: "v".into(), score: s.positive },
        GcamScore { dict_id: "finlex".into(), dim_id: "neg".into(), key: "v".into(), score: s.negative },
        GcamScore { dict_id: "finlex".into(), dim_id: "act".into(), key: "v".into(), score: s.activity },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_positive_text() {
        let s = score_tone("The bank reported strong profit growth and a record surplus this year.");
        assert!(s.tone > 0.0, "expected positive tone, got {}", s.tone);
        assert!(s.positive > 0.0 && s.negative == 0.0);
        assert!(s.word_count > 0);
    }

    #[test]
    fn test_negative_text() {
        let s = score_tone("Regulator imposed a penalty after fraud and default; shares crash on the news.");
        assert!(s.tone < 0.0, "expected negative tone, got {}", s.tone);
        assert!(s.negative > 0.0);
        assert!(s.activity > 0.0, "imposed should count as activity");
    }

    #[test]
    fn test_polarity_is_pos_plus_neg() {
        let s = score_tone("growth and profit but also loss and fraud reported here today");
        assert!((s.polarity - (s.positive + s.negative)).abs() < 1e-9);
    }

    #[test]
    fn test_gcam_panel_shape() {
        let s = score_tone("strong growth and profit reported by the company this quarter today");
        let g = gcam_for(&s);
        assert!(g.iter().any(|x| x.dim_id == "tone" && x.key == "v"));
        assert!(g.iter().any(|x| x.dim_id == "wc" && x.key == "c"));
        assert_eq!(g.len(), 5);
    }
}
