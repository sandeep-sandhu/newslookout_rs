// file: web_api.rs
// Purpose: Lightweight HTTP status API server for the NewsLookout pipeline.
// Endpoints: GET /  GET /health  GET /status  GET /status/summary  GET /dashboard.html

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use chrono::{DateTime, Utc};
use log::{error, info, warn};

/// Shared pipeline stats updated by start_data_pipeline and read by the HTTP handler.
#[derive(Clone)]
pub struct PipelineStatus {
    pub is_running: bool,
    pub retrievers_total: usize,
    pub retrievers_enabled: usize,
    pub data_processors_total: usize,
    pub data_processors_enabled: usize,
    pub docs_retrieved: usize,
    pub docs_processed: usize,
    pub start_time: DateTime<Utc>,
    /// Names of all enabled retriever plugins.
    pub retriever_plugin_names: Vec<String>,
    /// Names of all enabled data processor plugins.
    pub data_processor_plugin_names: Vec<String>,
    /// Plugins that have completed (name → doc count string).
    pub completed_plugins: Vec<(String, usize)>,
    /// Approximate queue depths for the dashboard (0 when not tracked).
    pub db_queue_size: usize,
    pub fetch_queue_size: usize,
    pub process_queue_size: usize,
}

impl PipelineStatus {
    pub fn new() -> Self {
        PipelineStatus {
            is_running: false,
            retrievers_total: 0,
            retrievers_enabled: 0,
            data_processors_total: 0,
            data_processors_enabled: 0,
            docs_retrieved: 0,
            docs_processed: 0,
            start_time: Utc::now(),
            retriever_plugin_names: Vec::new(),
            data_processor_plugin_names: Vec::new(),
            completed_plugins: Vec::new(),
            db_queue_size: 0,
            fetch_queue_size: 0,
            process_queue_size: 0,
        }
    }
}

/// Application version reported to the dashboard.
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub type SharedStatus = Arc<Mutex<PipelineStatus>>;

pub fn create_status_tracker() -> SharedStatus {
    Arc::new(Mutex::new(PipelineStatus::new()))
}

/// Start the HTTP status API in a background daemon thread.
/// Returns immediately; the server runs until the process exits.
pub fn start_web_api(host: &str, port: u16, status: SharedStatus) {
    let addr = format!("{}:{}", host, port);
    match TcpListener::bind(&addr) {
        Ok(listener) => {
            info!("Status API listening on http://{}/dashboard.html", addr);
            thread::Builder::new()
                .name("web_api".into())
                .spawn(move || {
                    for stream in listener.incoming() {
                        match stream {
                            Ok(s) => {
                                let st = Arc::clone(&status);
                                thread::spawn(move || handle_connection(s, st));
                            }
                            Err(e) => warn!("Status API accept error: {}", e),
                        }
                    }
                })
                .expect("Could not spawn web_api thread");
        }
        Err(e) => error!("Could not bind status API to {}: {}", addr, e),
    }
}

fn handle_connection(mut stream: std::net::TcpStream, status: SharedStatus) {
    let mut buf = [0u8; 2048];
    let n = stream.read(&mut buf).unwrap_or(0);
    if n == 0 { return; }

    let request = String::from_utf8_lossy(&buf[..n]);
    let path = extract_request_path(&request);

    let (status_line, content_type, body) = match path.trim_end_matches('/') {
        "" | "/index.html" => root_response(),
        "/health" => health_response(),
        "/status" => status_response(&status),
        "/status/summary" => summary_response(&status),
        "/metrics" => metrics_response(),
        "/dashboard.html" => dashboard_response(),
        _ => (
            "HTTP/1.1 404 Not Found",
            "text/plain",
            "404 Not Found".to_string(),
        ),
    };

    let response = format!(
        "{}\r\nContent-Type: {}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{}",
        status_line, content_type, body.len(), body
    );
    let _ = stream.write_all(response.as_bytes());
}

fn extract_request_path(request: &str) -> String {
    // First line: "GET /path HTTP/1.1"
    let first_line = request.lines().next().unwrap_or("");
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() >= 2 { parts[1].to_string() } else { "/".to_string() }
}

fn root_response() -> (&'static str, &'static str, String) {
    let body = r#"{"service":"NewsLookout Status API","version":"0.5.0","endpoints":{"/status":"Full pipeline status","/status/summary":"Summary statistics","/health":"Health check","/dashboard.html":"Dashboard UI"}}"#;
    ("HTTP/1.1 200 OK", "application/json", body.to_string())
}

fn health_response() -> (&'static str, &'static str, String) {
    let ts = Utc::now().to_rfc3339();
    let body = format!(r#"{{"status":"healthy","timestamp":"{}"}}"#, ts);
    ("HTTP/1.1 200 OK", "application/json", body)
}

fn status_response(status: &SharedStatus) -> (&'static str, &'static str, String) {
    let st = status.lock().unwrap();
    let body = build_status_json(&st);
    ("HTTP/1.1 200 OK", "application/json", body)
}

/// Build the dashboard `/status` JSON. The schema matches the fields the bundled
/// dashboard's JS reads: application / performance / queues / workers / database /
/// plugins. Per-plugin URL-level counts are populated as far as the pipeline currently
/// tracks them (completed-plugin doc counts); finer telemetry can be layered on later.
fn build_status_json(st: &PipelineStatus) -> String {
    use std::collections::HashMap;
    let elapsed = Utc::now().signed_duration_since(st.start_time).num_seconds();

    // name -> processed doc count, from completed plugins.
    let processed_by: HashMap<&str, usize> =
        st.completed_plugins.iter().map(|(n, c)| (n.as_str(), *c)).collect();
    let is_completed = |name: &str| processed_by.contains_key(name);

    // Performance panels.
    let retr_total = st.retrievers_enabled.max(1);
    let retr_done = st.retriever_plugin_names.iter().filter(|n| is_completed(n)).count();
    let url_disc_pct = (retr_done as f64) / (retr_total as f64) * 100.0;
    let dp_total = st.docs_retrieved.max(st.docs_processed);
    let dp_pct = if dp_total > 0 { (st.docs_processed as f64) / (dp_total as f64) * 100.0 } else { 0.0 };

    // Worker views.
    let worker_pairs: Vec<serde_json::Value> = st.retriever_plugin_names.iter().map(|name| {
        serde_json::json!({
            "plugin": name,
            "url_worker_alive": st.is_running && !is_completed(name),
            "content_worker_alive": st.is_running && !is_completed(name),
            "url_discovery_complete": is_completed(name),
        })
    }).collect();
    let data_workers: Vec<serde_json::Value> = st.data_processor_plugin_names.iter().map(|name| {
        serde_json::json!({ "name": name, "is_alive": st.is_running })
    }).collect();

    // Plugin tables.
    let content_plugins: Vec<serde_json::Value> = st.retriever_plugin_names.iter().map(|name| {
        let processed = *processed_by.get(name.as_str()).unwrap_or(&0);
        serde_json::json!({
            "name": name,
            "total_urls": processed,
            "processed_urls": processed,
        })
    }).collect();
    let data_processing: Vec<serde_json::Value> = st.data_processor_plugin_names.iter().map(|name| {
        let processed = *processed_by.get(name.as_str()).unwrap_or(&0);
        serde_json::json!({
            "name": name,
            "urls_input": st.docs_retrieved,
            "urls_processed": processed,
        })
    }).collect();

    let m = crate::metrics::snapshot();

    let v = serde_json::json!({
        "timestamp": Utc::now().to_rfc3339(),
        "application": {
            "name": "NewsLookout",
            "version": APP_VERSION,
            "is_running": st.is_running,
            "start_time": st.start_time.to_rfc3339(),
            "elapsed_seconds": elapsed,
        },
        "performance": {
            "url_discovery": {
                "progress_percent": url_disc_pct,
                "plugins_completed": retr_done,
                "plugins_total": st.retrievers_enabled,
            },
            "content_fetching": {
                "progress_percent": if st.docs_retrieved > 0 { 100.0 } else { 0.0 },
                "completed": st.docs_retrieved,
                "total": st.docs_retrieved,
            },
            "data_processing": {
                "progress_percent": dp_pct,
                "completed": st.docs_processed,
                "total": dp_total,
            },
        },
        "queues": {
            "database_operations": { "size": st.db_queue_size },
            "fetch_completed": { "size": st.fetch_queue_size },
            "data_processing_input": { "size": st.process_queue_size },
        },
        "workers": {
            "worker_pairs": worker_pairs,
            "data_processing_workers": data_workers,
        },
        "database": {
            "connection_status": "connected",
            "http_errors": {
                "HTTP_403": m.http_403,
                "HTTP_404": m.http_404,
                "HTTP_5xx": m.http_5xx,
                "HTTP_timeouts": m.http_timeouts,
                "HTTP_transport_errors": m.http_transport_errors,
            },
        },
        "plugins": {
            "content_plugins": content_plugins,
            "data_processing": data_processing,
        },
    });
    v.to_string()
}

fn summary_response(status: &SharedStatus) -> (&'static str, &'static str, String) {
    let st = status.lock().unwrap();
    let body = format!(
        r#"{{"timestamp":"{ts}","summary":{{"is_running":{running},"docs_retrieved":{dr},"docs_processed":{dp},"retrievers_enabled":{re},"data_processors_enabled":{de}}}}}"#,
        ts      = Utc::now().to_rfc3339(),
        running = st.is_running,
        dr = st.docs_retrieved,
        dp = st.docs_processed,
        re = st.retrievers_enabled,
        de = st.data_processors_enabled,
    );
    ("HTTP/1.1 200 OK", "application/json", body)
}

fn metrics_response() -> (&'static str, &'static str, String) {
    let body = serde_json::to_string(&crate::metrics::snapshot())
        .unwrap_or_else(|_| "{}".to_string());
    ("HTTP/1.1 200 OK", "application/json", body)
}

/// Serve the bundled rich dashboard (compiled into the binary).
fn dashboard_response() -> (&'static str, &'static str, String) {
    let html = include_str!("web_dashboard.html");
    ("HTTP/1.1 200 OK", "text/html; charset=utf-8", html.to_string())
}

#[allow(dead_code)]
fn dashboard_response_legacy() -> (&'static str, &'static str, String) {
    let html = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta http-equiv="refresh" content="10">
<title>NewsLookout Dashboard</title>
<style>
  body{font-family:sans-serif;background:#1a1a2e;color:#eee;margin:0;padding:20px}
  h1{color:#e94560;margin-bottom:4px}
  .sub{color:#aaa;font-size:.85em;margin-bottom:24px}
  .grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(200px,1fr));gap:16px;margin-bottom:24px}
  .card{background:#16213e;border-radius:8px;padding:20px;border:1px solid #0f3460}
  .card h3{margin:0 0 8px;color:#e94560;font-size:.85em;text-transform:uppercase;letter-spacing:.05em}
  .card .val{font-size:2.2em;font-weight:700;color:#fff}
  .card .label{font-size:.75em;color:#aaa;margin-top:4px}
  .status-dot{display:inline-block;width:10px;height:10px;border-radius:50%;margin-right:6px}
  .running{background:#00d26a}.stopped{background:#e94560}
  table{width:100%;border-collapse:collapse;background:#16213e;border-radius:8px;overflow:hidden;margin-bottom:20px}
  th{background:#0f3460;padding:10px 14px;text-align:left;font-size:.8em;text-transform:uppercase;color:#aaa}
  td{padding:8px 14px;border-bottom:1px solid #0f3460;font-size:.85em}
  tr:last-child td{border-bottom:none}
  .section-title{color:#e94560;font-size:1em;font-weight:600;margin:20px 0 8px}
  .plugin-tag{display:inline-block;background:#0f3460;border-radius:4px;padding:2px 8px;margin:2px;font-size:.75em}
  .footer{color:#555;font-size:.75em;margin-top:20px}
</style>
</head>
<body>
<h1>NewsLookout</h1>
<div class="sub">Pipeline Status Dashboard &mdash; auto-refreshes every 10s</div>

<div class="grid" id="cards">
  <div class="card"><h3>Status</h3><div class="val" id="status">…</div></div>
  <div class="card"><h3>Docs Retrieved</h3><div class="val" id="retrieved">…</div></div>
  <div class="card"><h3>Docs Processed</h3><div class="val" id="processed">…</div></div>
  <div class="card"><h3>Elapsed</h3><div class="val" id="elapsed">…</div></div>
  <div class="card"><h3>Retrievers</h3><div class="val" id="ret-count">…</div><div class="label" id="ret-label">enabled</div></div>
  <div class="card"><h3>Processors</h3><div class="val" id="proc-count">…</div><div class="label" id="proc-label">enabled</div></div>
</div>

<p class="section-title">Plugin Overview</p>
<table>
<thead><tr><th>Plugin Type</th><th>Total</th><th>Enabled</th></tr></thead>
<tbody id="ptable"><tr><td colspan="3">Loading…</td></tr></tbody>
</table>

<p class="section-title">Retriever Plugins</p>
<div id="retriever-tags">…</div>

<p class="section-title">Data Processor Plugins</p>
<div id="processor-tags">…</div>

<p class="section-title">Completed Plugins</p>
<table>
<thead><tr><th>Plugin</th><th>Docs</th></tr></thead>
<tbody id="completed-table"><tr><td colspan="2">—</td></tr></tbody>
</table>

<div class="footer">Powered by NewsLookout &mdash; <span id="ts"></span></div>
<script>
async function refresh(){
  try{
    const r=await fetch('/status');
    const d=await r.json();
    const run=d.application.is_running;
    document.getElementById('status').innerHTML=
      `<span class="status-dot ${run?'running':'stopped'}"></span>${run?'Running':'Stopped'}`;
    document.getElementById('retrieved').textContent=d.documents.retrieved;
    document.getElementById('processed').textContent=d.documents.processed;
    const sec=d.application.elapsed_seconds;
    const h=Math.floor(sec/3600),m=Math.floor((sec%3600)/60),s=sec%60;
    document.getElementById('elapsed').textContent=
      `${String(h).padStart(2,'0')}:${String(m).padStart(2,'0')}:${String(s).padStart(2,'0')}`;
    const p=d.plugins;
    document.getElementById('ret-count').textContent=p.retrievers.enabled;
    document.getElementById('ret-label').textContent=`of ${p.retrievers.total} total`;
    document.getElementById('proc-count').textContent=p.data_processors.enabled;
    document.getElementById('proc-label').textContent=`of ${p.data_processors.total} total`;
    document.getElementById('ptable').innerHTML=
      `<tr><td>Retrievers</td><td>${p.retrievers.total}</td><td>${p.retrievers.enabled}</td></tr>`+
      `<tr><td>Data Processors</td><td>${p.data_processors.total}</td><td>${p.data_processors.enabled}</td></tr>`;

    // Retriever plugin tags
    const rnames = p.retrievers.names || [];
    document.getElementById('retriever-tags').innerHTML =
      rnames.length ? rnames.map(n=>`<span class="plugin-tag">${n}</span>`).join('') : '—';

    // Data processor plugin tags
    const pnames = p.data_processors.names || [];
    document.getElementById('processor-tags').innerHTML =
      pnames.length ? pnames.map(n=>`<span class="plugin-tag">${n}</span>`).join('') : '—';

    // Completed plugins table
    const comp = p.completed || [];
    document.getElementById('completed-table').innerHTML =
      comp.length
        ? comp.map(c=>`<tr><td>${c.name}</td><td>${c.docs}</td></tr>`).join('')
        : '<tr><td colspan="2">None yet</td></tr>';

    document.getElementById('ts').textContent=d.timestamp;
  }catch(e){
    document.getElementById('status').textContent='API unavailable';
  }
}
refresh();
</script>
</body>
</html>"#;
    ("HTTP/1.1 200 OK", "text/html; charset=utf-8", html.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_status_json_has_dashboard_contract() {
        let mut st = PipelineStatus::new();
        st.is_running = true;
        st.retrievers_enabled = 2;
        st.retriever_plugin_names = vec!["mod_en_in_rbi".into(), "mod_en_in_sebi".into()];
        st.data_processor_plugin_names = vec!["mod_vectorstore".into()];
        st.completed_plugins = vec![("mod_en_in_rbi".into(), 12)];
        st.docs_retrieved = 30;
        st.docs_processed = 18;

        let body = build_status_json(&st);
        let v: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");

        // Top-level contract keys the dashboard JS reads.
        assert!(v["application"]["is_running"].as_bool().unwrap());
        assert!(v["application"]["version"].is_string());
        assert!(v["performance"]["url_discovery"]["progress_percent"].is_number());
        assert!(v["queues"]["database_operations"]["size"].is_number());
        assert!(v["workers"]["worker_pairs"].is_array());
        assert!(v["workers"]["data_processing_workers"].is_array());
        assert_eq!(v["database"]["connection_status"], "connected");
        assert!(v["database"]["http_errors"]["HTTP_403"].is_number());

        // content_plugins reflects completed counts.
        let cps = v["plugins"]["content_plugins"].as_array().unwrap();
        let rbi = cps.iter().find(|p| p["name"] == "mod_en_in_rbi").unwrap();
        assert_eq!(rbi["processed_urls"], 12);

        // one worker pair flagged complete (rbi), one still alive (sebi)
        let pairs = v["workers"]["worker_pairs"].as_array().unwrap();
        assert_eq!(pairs.len(), 2);
        let rbi_pair = pairs.iter().find(|p| p["plugin"] == "mod_en_in_rbi").unwrap();
        assert_eq!(rbi_pair["url_discovery_complete"], true);
    }

    #[test]
    fn test_dashboard_html_is_bundled() {
        let (_s, ct, body) = dashboard_response();
        assert!(ct.contains("text/html"));
        assert!(body.contains("NewsLookout"));
        assert!(body.contains("/status"), "dashboard JS should call /status");
    }
}
