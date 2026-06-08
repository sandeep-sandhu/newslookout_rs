// file: mod_entity_graph.rs
// Purpose:
//   Phase-2/3 entity co-occurrence graph builder (roadmap Stage 6/7 / D2). For every document
//   it links the organisations (and persons) that are mentioned together into undirected
//   co-occurrence edges in `entity_edges`, stamped with the document's date and tone. Aggregating
//   these edges over the corpus yields the entity-relationship graph that powers
//   `mod_emit_graph` (GEXF) and downstream network analytics. Runs after `mod_ner` so the
//   organisations are present on `doc.analysis`; writes are batched per N documents in a single
//   transaction (roadmap point 9).

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use config::Config;
use log::{error, info};

use crate::analysis::{provisional_entity_id, DocAnalysis};
use crate::cfg::get_database_filename;
use crate::document::Document;
use crate::plugins::mod_mentions::doc_id_for;
use crate::store::records::insert_edge;

pub const PLUGIN_NAME: &str = "mod_entity_graph";

const BATCH_SIZE: usize = 50;
const EDGE_TYPE: &str = "cooccur";

/// One buffered document's worth of graph input.
struct GraphDoc {
    doc_id: String,
    date_ms: i64,
    tone: f64,
    entity_ids: Vec<String>,
}

pub fn process_data(
    tx: Sender<Document>,
    rx: Receiver<Document>,
    config: &Config,
    _api_mutexes: &mut HashMap<String, Arc<Mutex<isize>>>,
) {
    info!("{}: Starting entity co-occurrence graph build (batched).", PLUGIN_NAME);
    let db_path = get_database_filename(config);
    let mut conn = match crate::store::open(&db_path) {
        Ok(c) => Some(c),
        Err(e) => {
            error!("{}: cannot open store '{}': {} — forwarding docs unmodified.", PLUGIN_NAME, db_path, e);
            None
        }
    };

    let mut buffer: Vec<GraphDoc> = Vec::with_capacity(BATCH_SIZE);
    let mut total_edges = 0usize;

    for doc in rx {
        if conn.is_some() {
            if let Some(a) = doc.analysis.as_ref() {
                let ids = entity_ids_for(a);
                if ids.len() >= 2 {
                    buffer.push(GraphDoc {
                        doc_id: doc_id_for(&doc),
                        date_ms: doc.publish_date_ms,
                        tone: a.tone.as_ref().map(|t| t.tone).unwrap_or(0.0),
                        entity_ids: ids,
                    });
                    if buffer.len() >= BATCH_SIZE {
                        if let Some(ref mut c) = conn {
                            total_edges += flush(c, &mut buffer);
                        }
                    }
                }
            }
        }
        if let Err(e) = tx.send(doc) {
            error!("{}: when forwarding doc: {}", PLUGIN_NAME, e);
        }
    }
    if let Some(ref mut c) = conn {
        total_edges += flush(c, &mut buffer);
    }
    info!("{}: Completed. Wrote {} co-occurrence edge(s).", PLUGIN_NAME, total_edges);
}

/// Distinct provisional entity ids for a document (organisations then persons), de-duplicated,
/// preserving first-seen order.
fn entity_ids_for(a: &DocAnalysis) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for e in a.organizations.iter().chain(a.persons.iter()) {
        let id = e.entity_id.clone().unwrap_or_else(|| provisional_entity_id(&e.surface_form));
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids
}

/// All unordered pairs from a list of ids, normalised so the lexicographically-smaller id is the
/// source (keeps the undirected graph canonical and avoids A-B / B-A duplication).
pub fn unique_pairs(ids: &[String]) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for i in 0..ids.len() {
        for j in (i + 1)..ids.len() {
            let (a, b) = (&ids[i], &ids[j]);
            if a == b {
                continue;
            }
            if a <= b {
                pairs.push((a.clone(), b.clone()));
            } else {
                pairs.push((b.clone(), a.clone()));
            }
        }
    }
    pairs
}

/// Flush buffered documents' edges in one transaction. Returns edges written.
fn flush(conn: &mut rusqlite::Connection, buffer: &mut Vec<GraphDoc>) -> usize {
    if buffer.is_empty() {
        return 0;
    }
    let mut written = 0usize;
    let result = (|| -> Result<(), String> {
        let tx = conn.transaction().map_err(|e| format!("begin tx: {}", e))?;
        for gd in buffer.iter() {
            for (src, dst) in unique_pairs(&gd.entity_ids) {
                insert_edge(&tx, &src, &dst, EDGE_TYPE, &gd.doc_id, gd.date_ms, gd.tone, 1.0, PLUGIN_NAME)?;
                written += 1;
            }
        }
        tx.commit().map_err(|e| format!("commit tx: {}", e))?;
        Ok(())
    })();
    if let Err(e) = result {
        error!("{}: batch flush failed ({} docs): {}", PLUGIN_NAME, buffer.len(), e);
        crate::metrics::record_db_error();
        buffer.clear();
        return 0;
    }
    crate::metrics::record_db_writes(written as u64);
    buffer.clear();
    written
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::EntityMention;
    use crate::store;

    fn org(name: &str) -> EntityMention {
        EntityMention {
            surface_form: name.into(),
            entity_type: "ORG".into(),
            char_offset: 0,
            salience: 1.0,
            entity_id: None,
        }
    }

    #[test]
    fn test_unique_pairs_canonical_and_count() {
        let ids = vec!["b".to_string(), "a".to_string(), "c".to_string()];
        let p = unique_pairs(&ids);
        assert_eq!(p.len(), 3, "3 entities -> 3 pairs");
        // every pair is ordered src<=dst
        assert!(p.iter().all(|(s, d)| s <= d), "pairs not canonicalised: {:?}", p);
        assert!(p.contains(&("a".to_string(), "b".to_string())));
    }

    #[test]
    fn test_entity_ids_dedup() {
        let a = DocAnalysis {
            organizations: vec![org("HDFC Bank"), org("hdfc  bank"), org("Axis Bank")],
            ..Default::default()
        };
        let ids = entity_ids_for(&a);
        assert_eq!(ids.len(), 2, "normalised duplicates collapse: {:?}", ids);
    }

    #[test]
    fn test_flush_writes_edges() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        store::migrate(&conn).unwrap();
        let mut buffer = vec![GraphDoc {
            doc_id: "D1".into(),
            date_ms: 1000,
            tone: -2.0,
            entity_ids: vec![
                provisional_entity_id("Reserve Bank of India"),
                provisional_entity_id("SEBI surface"),
                provisional_entity_id("Axis Bank"),
            ],
        }];
        let n = flush(&mut conn, &mut buffer);
        assert_eq!(n, 3, "3 entities -> 3 edges");
        assert!(buffer.is_empty());
        let edges: i64 = conn.query_row("SELECT COUNT(*) FROM entity_edges WHERE edge_type='cooccur'", [], |r| r.get(0)).unwrap();
        assert_eq!(edges, 3);
    }

    #[test]
    fn test_single_entity_no_edges() {
        let a = DocAnalysis { organizations: vec![org("SBI")], ..Default::default() };
        assert!(unique_pairs(&entity_ids_for(&a)).is_empty());
    }
}
