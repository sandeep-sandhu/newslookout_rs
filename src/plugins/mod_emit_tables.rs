// file: mod_emit_tables.rs
// Purpose:
//   Phase-1 emitter (roadmap Stage 5 / F1). Persists the structured facts that the upstream
//   extractors (`mod_extract_quant`, `mod_themes`, `mod_tone`, and later NER/geo) have placed
//   on `doc.analysis` into the canonical fact tables (`amounts`/`counts`/`dates_ref`/`themes`/
//   `gcam`) via the store layer. Writes are *batched*: documents are buffered and flushed in a
//   single SQLite transaction once the buffer fills or the stream ends, reducing disk I/O
//   (roadmap point 9). This plugin is the canonical-table sink and runs near the end of the
//   data_processor chain, after all enrichment but before vectorstore.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use config::Config;
use log::{error, info};

use crate::analysis::DocAnalysis;
use crate::cfg::get_database_filename;
use crate::document::Document;
use crate::plugins::mod_mentions::doc_id_for;
use crate::store::records::emit_analysis;

pub const PLUGIN_NAME: &str = "mod_emit_tables";

/// Flush the buffer to disk once this many documents have accumulated.
const BATCH_SIZE: usize = 50;

pub fn process_data(
    tx: Sender<Document>,
    rx: Receiver<Document>,
    config: &Config,
    _api_mutexes: &mut HashMap<String, Arc<Mutex<isize>>>,
) {
    info!("{}: Starting canonical-table emit (batched, size={}).", PLUGIN_NAME, BATCH_SIZE);
    let db_path = get_database_filename(config);
    let mut conn = match crate::store::open(&db_path) {
        Ok(c) => Some(c),
        Err(e) => {
            error!("{}: cannot open store '{}': {} — forwarding docs unmodified.", PLUGIN_NAME, db_path, e);
            None
        }
    };

    // Buffer of (doc_id, analysis) awaiting a transactional flush.
    let mut buffer: Vec<(String, DocAnalysis)> = Vec::with_capacity(BATCH_SIZE);
    let mut total_facts = 0usize;
    let mut total_docs = 0usize;

    for doc in rx {
        // Buffer the facts (clone only the sidecar, which is small) before forwarding.
        if conn.is_some() {
            if let Some(analysis) = doc.analysis.as_ref() {
                if !analysis.is_empty() {
                    buffer.push((doc_id_for(&doc), analysis.clone()));
                    if buffer.len() >= BATCH_SIZE {
                        if let Some(ref mut c) = conn {
                            let (d, f) = flush(c, &mut buffer);
                            total_docs += d;
                            total_facts += f;
                        }
                    }
                }
            }
        }
        if let Err(e) = tx.send(doc) {
            error!("{}: when forwarding doc: {}", PLUGIN_NAME, e);
        }
    }

    // Final flush of whatever remains when the stream ends.
    if let Some(ref mut c) = conn {
        let (d, f) = flush(c, &mut buffer);
        total_docs += d;
        total_facts += f;
    }
    info!("{}: Completed. Persisted {} fact(s) for {} document(s).", PLUGIN_NAME, total_facts, total_docs);
}

/// Flush the buffer in a single transaction. Returns (documents_written, facts_written).
/// On a transaction error the buffer is still cleared (the docs were already forwarded) and the
/// error logged, so a single bad batch cannot wedge the pipeline.
fn flush(conn: &mut rusqlite::Connection, buffer: &mut Vec<(String, DocAnalysis)>) -> (usize, usize) {
    if buffer.is_empty() {
        return (0, 0);
    }
    let mut facts = 0usize;
    let mut docs = 0usize;
    let result = (|| -> Result<(), String> {
        let tx = conn.transaction().map_err(|e| format!("begin tx: {}", e))?;
        for (doc_id, analysis) in buffer.iter() {
            facts += emit_analysis(&tx, doc_id, analysis)?;
            docs += 1;
        }
        tx.commit().map_err(|e| format!("commit tx: {}", e))?;
        Ok(())
    })();
    if let Err(e) = result {
        error!("{}: batch flush failed ({} docs): {}", PLUGIN_NAME, buffer.len(), e);
        crate::metrics::record_db_error();
        buffer.clear();
        return (0, 0);
    }
    crate::metrics::record_db_writes(facts as u64);
    buffer.clear();
    (docs, facts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{AmountMention, DocAnalysis, ThemeMention};
    use crate::store;

    #[test]
    fn test_flush_writes_and_clears_buffer() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        store::migrate(&conn).unwrap();

        let mut buffer = vec![
            (
                "D1".to_string(),
                DocAnalysis {
                    amounts: vec![AmountMention { value: 1e7, currency: "INR".into(), unit: "crore".into(), object: String::new(), char_offset: 0 }],
                    themes: vec![ThemeMention { theme: "FIN_BANKING".into(), char_offset: 0 }],
                    ..Default::default()
                },
            ),
            (
                "D2".to_string(),
                DocAnalysis {
                    themes: vec![ThemeMention { theme: "TAX".into(), char_offset: 5 }],
                    ..Default::default()
                },
            ),
        ];

        let (docs, facts) = flush(&mut conn, &mut buffer);
        assert_eq!(docs, 2);
        assert_eq!(facts, 3);
        assert!(buffer.is_empty(), "buffer must be cleared after flush");

        let amounts: i64 = conn.query_row("SELECT COUNT(*) FROM amounts", [], |r| r.get(0)).unwrap();
        let themes: i64 = conn.query_row("SELECT COUNT(*) FROM themes", [], |r| r.get(0)).unwrap();
        assert_eq!(amounts, 1);
        assert_eq!(themes, 2);
    }

    #[test]
    fn test_flush_empty_is_noop() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        store::migrate(&conn).unwrap();
        let mut empty: Vec<(String, DocAnalysis)> = Vec::new();
        assert_eq!(flush(&mut conn, &mut empty), (0, 0));
    }
}
