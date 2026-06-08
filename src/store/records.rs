// file: store/records.rs
// Purpose:
//   Typed insert/upsert helpers over the canonical `documents` and `mentions` tables
//   (migration 0001). Used by the Phase-0 `mod_mentions` plugin and, later, by the
//   structured-extraction emitters. Keeping the SQL here (rather than in each plugin) means
//   the store layer remains the single owner of schema access.

use rusqlite::Connection;

use crate::analysis::{provisional_entity_id, DocAnalysis, EntityMention};

/// A row for the canonical `documents` table.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DocumentRow {
    pub doc_id: String,
    pub url: String,
    pub source: String,
    pub title: String,
    pub lang: String,
    pub pubdate_ms: i64,
    pub pubdate: String,
    pub plugin: String,
    pub section: String,
    pub cluster_id: String,
    pub tone: f64,
    pub word_count: i64,
}

/// Insert or replace a document row (keyed by `doc_id`), stamping the ingest time.
pub fn upsert_document(conn: &Connection, d: &DocumentRow) -> Result<usize, String> {
    conn.execute(
        "INSERT INTO documents
            (doc_id, url, source, title, lang, pubdate_ms, pubdate, plugin, section,
             cluster_id, tone, word_count, ingested_ts)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)
         ON CONFLICT(doc_id) DO UPDATE SET
            url=?2, source=?3, title=?4, lang=?5, pubdate_ms=?6, pubdate=?7, plugin=?8,
            section=?9, cluster_id=?10, tone=?11, word_count=?12, ingested_ts=?13",
        rusqlite::params![
            d.doc_id, d.url, d.source, d.title, d.lang, d.pubdate_ms, d.pubdate, d.plugin,
            d.section, d.cluster_id, d.tone, d.word_count, chrono::Utc::now().timestamp(),
        ],
    )
    .map_err(|e| format!("upsert_document({}): {}", d.doc_id, e))
}

/// Insert a mention linking a document to a story/event cluster (GDELT Mentions analog).
pub fn insert_mention(
    conn: &Connection,
    cluster_id: &str,
    doc_id: &str,
    mention_ts: i64,
    source: &str,
    confidence: f64,
) -> Result<usize, String> {
    conn.execute(
        "INSERT INTO mentions (cluster_id, doc_id, mention_ts, source, confidence)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![cluster_id, doc_id, mention_ts, source, confidence],
    )
    .map_err(|e| format!("insert_mention({}): {}", doc_id, e))
}

/// Emit all structured-analysis facts for one document into the canonical fact tables
/// (`amounts`, `counts`, `dates_ref`, `themes`, `gcam`). Intended to be called inside a
/// transaction owned by `mod_emit_tables` so many documents flush as one batch. The
/// `documents`/`mentions` rows remain owned by `mod_mentions`; this only writes the per-fact
/// tables keyed on `doc_id`.
pub fn emit_analysis(conn: &Connection, doc_id: &str, a: &DocAnalysis) -> Result<usize, String> {
    let mut n = 0usize;
    for m in &a.amounts {
        conn.execute(
            "INSERT INTO amounts (doc_id, value, currency, unit, object, char_offset)
             VALUES (?1,?2,?3,?4,?5,?6)",
            rusqlite::params![doc_id, m.value, m.currency, m.unit, m.object, m.char_offset as i64],
        )
        .map_err(|e| format!("insert amount({}): {}", doc_id, e))?;
        n += 1;
    }
    for m in &a.counts {
        conn.execute(
            "INSERT INTO counts (doc_id, count_type, number, object, geo_feature_id, char_offset)
             VALUES (?1,?2,?3,?4,NULL,?5)",
            rusqlite::params![doc_id, m.count_type, m.number, m.object, m.char_offset as i64],
        )
        .map_err(|e| format!("insert count({}): {}", doc_id, e))?;
        n += 1;
    }
    for d in &a.dates_referenced {
        conn.execute(
            "INSERT INTO dates_ref (doc_id, resolution, year, month, day, char_offset)
             VALUES (?1,?2,?3,?4,?5,?6)",
            rusqlite::params![doc_id, d.resolution, d.year, d.month, d.day, d.char_offset as i64],
        )
        .map_err(|e| format!("insert date_ref({}): {}", doc_id, e))?;
        n += 1;
    }
    for t in &a.themes {
        conn.execute(
            "INSERT INTO themes (doc_id, theme, char_offset) VALUES (?1,?2,?3)",
            rusqlite::params![doc_id, t.theme, t.char_offset as i64],
        )
        .map_err(|e| format!("insert theme({}): {}", doc_id, e))?;
        n += 1;
    }
    for g in &a.gcam {
        conn.execute(
            "INSERT INTO gcam (doc_id, dict_id, dim_id, key, score) VALUES (?1,?2,?3,?4,?5)",
            rusqlite::params![doc_id, g.dict_id, g.dim_id, g.key, g.score],
        )
        .map_err(|e| format!("insert gcam({}): {}", doc_id, e))?;
        n += 1;
    }
    for l in &a.locations {
        conn.execute(
            "INSERT INTO locations (doc_id, name, feature_id, lat, lon, country, adm1, adm2, char_offset)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            rusqlite::params![
                doc_id, l.name, l.feature_id, l.lat, l.lon, l.country, l.adm1, l.adm2, l.char_offset as i64
            ],
        )
        .map_err(|e| format!("insert location({}): {}", doc_id, e))?;
        n += 1;
    }
    for e in a.organizations.iter().chain(a.persons.iter()) {
        n += emit_entity_mention(conn, doc_id, e)?;
    }
    Ok(n)
}

/// Persist one entity mention plus a minimal `entities` master row. Until `mod_entity_resolve`
/// supplies a real LEI/CIN-backed id, the entity is keyed by a provisional surface-form id so
/// co-mentions across documents group consistently.
fn emit_entity_mention(conn: &Connection, doc_id: &str, e: &EntityMention) -> Result<usize, String> {
    let entity_id = e.entity_id.clone().unwrap_or_else(|| provisional_entity_id(&e.surface_form));
    // Minimal master row; INSERT OR IGNORE so a later resolved row is never clobbered here.
    conn.execute(
        "INSERT OR IGNORE INTO entities (entity_id, type, canonical_name, name_norm, status)
         VALUES (?1, ?2, ?3, ?4, 'provisional')",
        rusqlite::params![
            entity_id,
            e.entity_type,
            e.surface_form,
            crate::analysis::norm_name(&e.surface_form)
        ],
    )
    .map_err(|err| format!("upsert entity({}): {}", entity_id, err))?;
    conn.execute(
        "INSERT INTO entity_mentions (doc_id, entity_id, surface_form, char_offset, salience)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![doc_id, entity_id, e.surface_form, e.char_offset as i64, e.salience],
    )
    .map_err(|err| format!("insert entity_mention({}): {}", doc_id, err))?;
    Ok(2)
}

/// Insert a co-occurrence (or other) edge between two entities into `entity_edges`.
#[allow(clippy::too_many_arguments)]
pub fn insert_edge(
    conn: &Connection,
    src: &str,
    dst: &str,
    edge_type: &str,
    doc_id: &str,
    date_ms: i64,
    tone: f64,
    weight: f64,
    source: &str,
) -> Result<usize, String> {
    conn.execute(
        "INSERT INTO entity_edges
            (src_entity_id, dst_entity_id, edge_type, doc_id, date_ms, tone, weight, source)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        rusqlite::params![src, dst, edge_type, doc_id, date_ms, tone, weight, source],
    )
    .map_err(|e| format!("insert edge({}->{}): {}", src, dst, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        store::migrate(&c).unwrap();
        c
    }

    #[test]
    fn test_upsert_document_inserts_then_updates() {
        let c = db();
        let mut d = DocumentRow { doc_id: "D1".into(), title: "first".into(), tone: -1.0, ..Default::default() };
        assert_eq!(upsert_document(&c, &d).unwrap(), 1);

        let title: String = c.query_row("SELECT title FROM documents WHERE doc_id='D1'", [], |r| r.get(0)).unwrap();
        assert_eq!(title, "first");

        // upsert again with a new title — should not create a duplicate row
        d.title = "second".into();
        upsert_document(&c, &d).unwrap();
        let n: i64 = c.query_row("SELECT COUNT(*) FROM documents", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
        let title2: String = c.query_row("SELECT title FROM documents WHERE doc_id='D1'", [], |r| r.get(0)).unwrap();
        assert_eq!(title2, "second");
    }

    #[test]
    fn test_emit_analysis_writes_fact_tables() {
        use crate::analysis::{AmountMention, CountMention, DateRef, DocAnalysis, GcamScore, ThemeMention};
        let c = db();
        let a = DocAnalysis {
            amounts: vec![AmountMention { value: 5e10, currency: "INR".into(), unit: "crore".into(), object: String::new(), char_offset: 3 }],
            counts: vec![CountMention { count_type: String::new(), number: 15.0, object: "banks".into(), char_offset: 7 }],
            dates_referenced: vec![DateRef { resolution: "day".into(), year: 2026, month: 6, day: 10, char_offset: 1 }],
            themes: vec![ThemeMention { theme: "FIN_BANKING".into(), char_offset: 0 }],
            gcam: vec![GcamScore { dict_id: "finlex".into(), dim_id: "tone".into(), key: "v".into(), score: 1.2 }],
            ..Default::default()
        };
        let n = emit_analysis(&c, "D1", &a).unwrap();
        assert_eq!(n, 5);
        for (tbl, expect) in [("amounts", 1), ("counts", 1), ("dates_ref", 1), ("themes", 1), ("gcam", 1)] {
            let got: i64 = c
                .query_row(&format!("SELECT COUNT(*) FROM {} WHERE doc_id='D1'", tbl), [], |r| r.get(0))
                .unwrap();
            assert_eq!(got, expect, "table {}", tbl);
        }
        let v: f64 = c.query_row("SELECT value FROM amounts WHERE doc_id='D1'", [], |r| r.get(0)).unwrap();
        assert_eq!(v, 5e10);
    }

    #[test]
    fn test_insert_mentions_for_cluster() {
        let c = db();
        insert_mention(&c, "C1", "D1", 100, "src1", 100.0).unwrap();
        insert_mention(&c, "C1", "D2", 200, "src2", 90.0).unwrap();
        let count: i64 = c.query_row("SELECT COUNT(*) FROM mentions WHERE cluster_id='C1'", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 2, "two documents mention cluster C1");
    }
}
