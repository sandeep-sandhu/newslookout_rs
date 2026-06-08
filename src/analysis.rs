// file: analysis.rs
// Purpose:
//   Sidecar structured-analysis fields attached to a `Document`, populated by the
//   structured-extraction data_processor plugins (NER, geocode, tone, themes, amounts,
//   events, ...). Kept separate from `Document` so the acquisition struct stays lean and
//   the analysis layer can evolve independently. Carried as `Document.analysis:
//   Option<DocAnalysis>` and emitted additively in the output JSON.
//
//   The field set mirrors GDELT's GKG coordinate systems (persons/orgs/locations/themes/
//   counts/amounts/quotations/dates/events/tone/GCAM) plus translation provenance, so the
//   data can be flattened into the canonical relational schema (see docs roadmap Part C).

use serde::{Deserialize, Serialize};

/// A named entity (person or organisation) detected in the document text.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct EntityMention {
    /// Exact text span as it appeared in the document.
    pub surface_form: String,
    /// Entity category: PERSON | ORG | GPE | MISC ...
    pub entity_type: String,
    /// Byte offset of the mention within `Document.text`.
    pub char_offset: usize,
    /// Relative importance of this entity in the document (0.0..1.0).
    pub salience: f64,
    /// Canonical entity id once resolved (LEI-first); `None` until `mod_entity_resolve`
    /// resolves it. False links are worse than nulls, so this stays `None` when uncertain.
    pub entity_id: Option<String>,
}

/// A geographic location mention, optionally resolved to coordinates / a feature id.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct GeoMention {
    pub name: String,
    /// GeoNames-style feature id, or `IN-<PIN>` / city key fallback.
    pub feature_id: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub country: String,
    pub adm1: String,
    pub adm2: String,
    pub char_offset: usize,
}

/// A controlled-vocabulary theme tag (see themebook, roadmap D6).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ThemeMention {
    pub theme: String,
    pub char_offset: usize,
}

/// A counted quantity ("20,000 soldiers"), GDELT V1COUNTS analog.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CountMention {
    pub count_type: String,
    pub number: f64,
    pub object: String,
    pub char_offset: usize,
}

/// A monetary / numeric amount with normalised value (crore=1e7, lakh=1e5).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AmountMention {
    pub value: f64,
    pub currency: String,
    pub unit: String,
    pub object: String,
    pub char_offset: usize,
}

/// A quotation with its (optionally resolved) speaker.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Quotation {
    pub speaker: String,
    pub speaker_entity_id: Option<String>,
    pub verb: String,
    pub quote: String,
    pub char_offset: usize,
}

/// A date referenced in the text (not the publication date).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DateRef {
    /// "year" | "month" | "day".
    pub resolution: String,
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub char_offset: usize,
}

/// A coded event (CAMEO-lite for general news, or a RegEventType for regulatory docs).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct EventRecord {
    pub event_type: String,
    pub actor1: String,
    pub actor2: String,
    /// Goldstein-style cooperation/conflict score (-10..+10), if assigned.
    pub goldstein: Option<f64>,
    /// QuadClass 1..4 (verbal/material cooperation/conflict), if assigned.
    pub quad_class: Option<u8>,
    pub char_offset: usize,
}

/// A single GCAM dimension score (GDELT-compatible `cX.Y`/`vX.Y` encoding).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct GcamScore {
    pub dict_id: String,
    pub dim_id: String,
    /// "c" = word count, "v" = scored value.
    pub key: String,
    pub score: f64,
}

/// Document-level tone panel (GDELT V1.5TONE analog), -10..+10 scale.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ToneScores {
    pub tone: f64,
    pub positive: f64,
    pub negative: f64,
    pub polarity: f64,
    pub activity: f64,
    pub self_group: f64,
    pub word_count: usize,
}

/// All structured-analysis outputs for a single document. Every field defaults to empty so
/// extractor plugins can populate incrementally and partially.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DocAnalysis {
    pub persons: Vec<EntityMention>,
    pub organizations: Vec<EntityMention>,
    pub locations: Vec<GeoMention>,
    /// All proper names beyond persons/orgs (named events, movements, legislation).
    pub all_names: Vec<String>,
    pub themes: Vec<ThemeMention>,
    pub counts: Vec<CountMention>,
    pub amounts: Vec<AmountMention>,
    pub quotes: Vec<Quotation>,
    pub dates_referenced: Vec<DateRef>,
    pub events: Vec<EventRecord>,
    pub tone: Option<ToneScores>,
    pub gcam: Vec<GcamScore>,
    /// ISO 639 language code of the source text (e.g. "hi", "mr", "en").
    pub lang: String,
    /// English translation of `Document.text` when the source was non-English.
    pub text_en: String,
}

impl DocAnalysis {
    /// True when no extractor has populated any field yet.
    pub fn is_empty(&self) -> bool {
        self.persons.is_empty()
            && self.organizations.is_empty()
            && self.locations.is_empty()
            && self.all_names.is_empty()
            && self.themes.is_empty()
            && self.counts.is_empty()
            && self.amounts.is_empty()
            && self.quotes.is_empty()
            && self.dates_referenced.is_empty()
            && self.events.is_empty()
            && self.tone.is_none()
            && self.gcam.is_empty()
            && self.lang.is_empty()
            && self.text_en.is_empty()
    }
}

/// Normalised key for an entity/place name: lowercase alphanumerics collapsed onto single
/// spaces. Shared by the NER recogniser and the store layer so the same surface form always
/// maps to the same key.
pub fn norm_name(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c.is_alphanumeric() {
            for lc in c.to_lowercase() {
                out.push(lc);
            }
            prev_space = false;
        } else if !prev_space {
            out.push(' ');
            prev_space = true;
        }
    }
    out.trim().to_string()
}

/// Deterministic provisional entity id for an unresolved entity, derived from its name. Stable
/// across runs/documents so co-mentions group together before `mod_entity_resolve` assigns a
/// real (LEI/CIN-backed) id.
pub fn provisional_entity_id(name: &str) -> String {
    format!("name:{}", norm_name(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_norm_and_provisional_id() {
        assert_eq!(norm_name("HDFC  Bank!"), "hdfc bank");
        assert_eq!(provisional_entity_id("Reserve Bank of India"), "name:reserve bank of india");
        assert_eq!(provisional_entity_id("HDFC Bank"), provisional_entity_id("hdfc  bank"));
    }

    #[test]
    fn test_default_is_empty() {
        let a = DocAnalysis::default();
        assert!(a.is_empty());
    }

    #[test]
    fn test_populated_not_empty() {
        let mut a = DocAnalysis::default();
        a.organizations.push(EntityMention {
            surface_form: "Reserve Bank of India".to_string(),
            entity_type: "ORG".to_string(),
            char_offset: 0,
            salience: 1.0,
            entity_id: None,
        });
        assert!(!a.is_empty());
    }

    #[test]
    fn test_serde_roundtrip() {
        let mut a = DocAnalysis::default();
        a.lang = "mr".to_string();
        a.tone = Some(ToneScores { tone: -2.5, negative: 3.0, word_count: 120, ..Default::default() });
        a.amounts.push(AmountMention {
            value: 20_000_000.0,
            currency: "INR".to_string(),
            unit: "crore".to_string(),
            object: "penalty".to_string(),
            char_offset: 42,
        });
        let json = serde_json::to_string(&a).unwrap();
        let back: DocAnalysis = serde_json::from_str(&json).unwrap();
        assert_eq!(a, back);
    }
}
