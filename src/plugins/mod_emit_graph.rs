// file: mod_emit_graph.rs
// Purpose:
//   Phase-2/3 graph exporter (roadmap Stage 6/7 / F2). Reads the co-occurrence edges written by
//   `mod_entity_graph`, aggregates them across the corpus (summing edge weights per entity pair),
//   and writes a GEXF 1.3 file that Gephi / networkx can open directly. Dependency-free XML
//   generation (no graph library). As a data_processor it forwards documents unchanged and
//   performs the one-shot export when the input stream ends, so the graph reflects every edge
//   committed upstream in this run. Disabled by default in config (it writes a file); enable it
//   to materialise the network artifact.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use config::Config;
use log::{error, info, warn};

use crate::document::Document;

pub const PLUGIN_NAME: &str = "mod_emit_graph";

/// Config key (string) for the output path; defaults to `reports/entity_graph.gexf`.
const CFG_KEY: &str = "entity_graph_gexf_path";
const DEFAULT_PATH: &str = "reports/entity_graph.gexf";

pub fn process_data(
    tx: Sender<Document>,
    rx: Receiver<Document>,
    config: &Config,
    _api_mutexes: &mut HashMap<String, Arc<Mutex<isize>>>,
) {
    info!("{}: Starting (graph export deferred to stream end).", PLUGIN_NAME);
    // Forward everything unchanged.
    for doc in rx {
        if let Err(e) = tx.send(doc) {
            error!("{}: when forwarding doc: {}", PLUGIN_NAME, e);
        }
    }

    let out_path = config.get_string(CFG_KEY).unwrap_or_else(|_| DEFAULT_PATH.to_string());
    let db_path = crate::cfg::get_database_filename(config);
    match crate::store::open(&db_path) {
        Ok(conn) => match export_gexf(&conn, &out_path) {
            Ok((nodes, edges)) => info!(
                "{}: Wrote GEXF '{}' ({} nodes, {} edges).",
                PLUGIN_NAME, out_path, nodes, edges
            ),
            Err(e) => warn!("{}: GEXF export skipped: {}", PLUGIN_NAME, e),
        },
        Err(e) => warn!("{}: cannot open store '{}': {}", PLUGIN_NAME, db_path, e),
    }
}

/// Query aggregated edges + their nodes from the store and write a GEXF file. Returns
/// (node_count, edge_count). Creates the parent directory if needed.
pub fn export_gexf(conn: &rusqlite::Connection, out_path: &str) -> Result<(usize, usize), String> {
    // Aggregate weight per undirected pair (edges are already canonicalised src<=dst upstream).
    let mut stmt = conn
        .prepare(
            "SELECT src_entity_id, dst_entity_id, SUM(weight)
             FROM entity_edges
             GROUP BY src_entity_id, dst_entity_id",
        )
        .map_err(|e| format!("prepare edge query: {}", e))?;
    let edges: Vec<(String, String, f64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(|e| format!("edge query: {}", e))?
        .filter_map(|r| r.ok())
        .collect();

    // Node labels: canonical_name from entities where available, else the id.
    let mut labels: HashMap<String, String> = HashMap::new();
    {
        let mut nstmt = conn
            .prepare("SELECT entity_id, COALESCE(canonical_name, entity_id) FROM entities")
            .map_err(|e| format!("prepare node query: {}", e))?;
        let rows = nstmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| format!("node query: {}", e))?;
        for row in rows.flatten() {
            labels.insert(row.0, row.1);
        }
    }

    // Collect distinct node ids actually referenced by edges.
    let mut node_ids: Vec<String> = Vec::new();
    for (s, d, _) in &edges {
        for id in [s, d] {
            if !node_ids.contains(id) {
                node_ids.push(id.clone());
            }
        }
    }
    let nodes: Vec<(String, String)> = node_ids
        .iter()
        .map(|id| (id.clone(), labels.get(id).cloned().unwrap_or_else(|| id.clone())))
        .collect();

    let xml = build_gexf(&nodes, &edges);

    if let Some(parent) = std::path::Path::new(out_path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create dir '{}': {}", parent.display(), e))?;
        }
    }
    std::fs::write(out_path, xml).map_err(|e| format!("write '{}': {}", out_path, e))?;
    Ok((nodes.len(), edges.len()))
}

/// Build a GEXF 1.3 document from node (id, label) and edge (src, dst, weight) lists.
pub fn build_gexf(nodes: &[(String, String)], edges: &[(String, String, f64)]) -> String {
    let mut s = String::with_capacity(256 + nodes.len() * 48 + edges.len() * 64);
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    s.push_str("<gexf xmlns=\"http://gexf.net/1.3\" version=\"1.3\">\n");
    s.push_str("  <graph mode=\"static\" defaultedgetype=\"undirected\">\n");
    s.push_str("    <nodes>\n");
    for (id, label) in nodes {
        s.push_str(&format!(
            "      <node id=\"{}\" label=\"{}\"/>\n",
            xml_escape(id),
            xml_escape(label)
        ));
    }
    s.push_str("    </nodes>\n");
    s.push_str("    <edges>\n");
    for (i, (src, dst, weight)) in edges.iter().enumerate() {
        s.push_str(&format!(
            "      <edge id=\"{}\" source=\"{}\" target=\"{}\" weight=\"{}\"/>\n",
            i,
            xml_escape(src),
            xml_escape(dst),
            weight
        ));
    }
    s.push_str("    </edges>\n");
    s.push_str("  </graph>\n");
    s.push_str("</gexf>\n");
    s
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;
    use crate::store::records::insert_edge;

    #[test]
    fn test_build_gexf_well_formed() {
        let nodes = vec![
            ("name:a".to_string(), "A & Co".to_string()),
            ("name:b".to_string(), "B Ltd".to_string()),
        ];
        let edges = vec![("name:a".to_string(), "name:b".to_string(), 3.0)];
        let xml = build_gexf(&nodes, &edges);
        assert!(xml.contains("<gexf"));
        assert!(xml.contains("label=\"A &amp; Co\""), "ampersand must be escaped");
        assert!(xml.contains("source=\"name:a\" target=\"name:b\" weight=\"3\""));
        assert_eq!(xml.matches("<node ").count(), 2);
        assert_eq!(xml.matches("<edge ").count(), 1);
    }

    #[test]
    fn test_export_gexf_aggregates_weights() {
        let dir = std::env::temp_dir();
        let out = dir.join(format!("nl_test_graph_{}.gexf", std::process::id()));
        let conn = {
            let c = rusqlite::Connection::open_in_memory().unwrap();
            store::migrate(&c).unwrap();
            c
        };
        // Two co-mentions of the same pair across two documents -> weight 2.
        insert_edge(&conn, "name:a", "name:b", "cooccur", "D1", 1, 0.0, 1.0, "t").unwrap();
        insert_edge(&conn, "name:a", "name:b", "cooccur", "D2", 1, 0.0, 1.0, "t").unwrap();
        insert_edge(&conn, "name:a", "name:c", "cooccur", "D1", 1, 0.0, 1.0, "t").unwrap();

        let (nodes, edges) = export_gexf(&conn, out.to_str().unwrap()).unwrap();
        assert_eq!(edges, 2, "two distinct pairs after aggregation");
        assert_eq!(nodes, 3, "a, b, c");
        let written = std::fs::read_to_string(&out).unwrap();
        assert!(written.contains("weight=\"2\""), "a-b weight should aggregate to 2");
        let _ = std::fs::remove_file(&out);
    }
}
