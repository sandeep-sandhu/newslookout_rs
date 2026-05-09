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
        }
    }
}

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
            info!("Status API listening on http://{}/status", addr);
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
    let elapsed = Utc::now().signed_duration_since(st.start_time);
    let body = format!(
        r#"{{"timestamp":"{ts}","application":{{"name":"NewsLookout","version":"0.5.0","is_running":{running},"start_time":"{start}","elapsed_seconds":{elapsed}}},"plugins":{{"retrievers":{{"total":{rt},"enabled":{re}}},"data_processors":{{"total":{dt},"enabled":{de}}}}},"documents":{{"retrieved":{dr},"processed":{dp}}}}}"#,
        ts      = Utc::now().to_rfc3339(),
        running = st.is_running,
        start   = st.start_time.to_rfc3339(),
        elapsed = elapsed.num_seconds(),
        rt = st.retrievers_total,
        re = st.retrievers_enabled,
        dt = st.data_processors_total,
        de = st.data_processors_enabled,
        dr = st.docs_retrieved,
        dp = st.docs_processed,
    );
    ("HTTP/1.1 200 OK", "application/json", body)
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

fn dashboard_response() -> (&'static str, &'static str, String) {
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
  .grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(220px,1fr));gap:16px;margin-bottom:24px}
  .card{background:#16213e;border-radius:8px;padding:20px;border:1px solid #0f3460}
  .card h3{margin:0 0 8px;color:#e94560;font-size:.85em;text-transform:uppercase;letter-spacing:.05em}
  .card .val{font-size:2.2em;font-weight:700;color:#fff}
  .card .label{font-size:.75em;color:#aaa;margin-top:4px}
  .status-dot{display:inline-block;width:10px;height:10px;border-radius:50%;margin-right:6px}
  .running{background:#00d26a}.stopped{background:#e94560}
  table{width:100%;border-collapse:collapse;background:#16213e;border-radius:8px;overflow:hidden}
  th{background:#0f3460;padding:10px 14px;text-align:left;font-size:.8em;text-transform:uppercase;color:#aaa}
  td{padding:10px 14px;border-bottom:1px solid #0f3460;font-size:.9em}
  tr:last-child td{border-bottom:none}
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
</div>
<table>
<thead><tr><th>Plugin Type</th><th>Total</th><th>Enabled</th></tr></thead>
<tbody id="ptable"><tr><td colspan="3">Loading…</td></tr></tbody>
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
    document.getElementById('ptable').innerHTML=
      `<tr><td>Retrievers</td><td>${p.retrievers.total}</td><td>${p.retrievers.enabled}</td></tr>`+
      `<tr><td>Data Processors</td><td>${p.data_processors.total}</td><td>${p.data_processors.enabled}</td></tr>`;
    document.getElementById('ts').textContent=d.timestamp;
  }catch(e){document.getElementById('status').textContent='API unavailable';}
}
refresh();
</script>
</body>
</html>"#;
    ("HTTP/1.1 200 OK", "text/html; charset=utf-8", html.to_string())
}
