// file: mod_vectorstore.rs
// Merged plugin: text splitting + semantic chunking + FAISS vector storage.
//
// This plugin replaces the separate split_text plugin. It:
//  1. Splits the document text into chunks using word-count windows (from split_text logic).
//  2. Applies TextTiling-style semantic boundary detection using vocabulary overlap
//     to identify logical section breaks.
//  3. Calls the vectorize_chunks.py Python helper to generate embeddings (all-mpnet-base-v2
//     via ONNX Runtime) and store each chunk in the FAISS flat index.
//
// Config keys read from app config:
//   vectorstore_path     - Directory for FAISS index + metadata (default: data_dir/vectorstore)
//   vectorstore_script   - Path to vectorize_chunks.py (default: auto-detected)
//   vectorstore_min_chunk_words  - Min words per chunk (default: 100)
//   vectorstore_max_chunk_words  - Max words per chunk (default: 500)
//   vectorstore_window_size      - TextTiling window size in sentences (default: 5)
//   vectorstore_similarity_threshold - Boundary similarity threshold (default: 0.3)

use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use config::Config;
use log::{debug, error, info, warn};
use serde_json::{json, Value};

use crate::document::Document;
use crate::get_plugin_cfg;

pub const PLUGIN_NAME: &str = "mod_vectorstore";

// ─── Public entry point ────────────────────────────────────────────────────────

pub fn process_data(
    tx: Sender<Document>,
    rx: Receiver<Document>,
    app_config: &Config,
    _api_mutexes: &mut HashMap<String, Arc<Mutex<isize>>>,
) {
    info!("{}: Starting — loading configuration.", PLUGIN_NAME);

    let params = VectorstoreParams::from_config(app_config);
    info!(
        "{}: vectorstore_path='{}', chunk words [{}-{}], window={}, threshold={:.2}",
        PLUGIN_NAME,
        params.vectorstore_path,
        params.min_chunk_words,
        params.max_chunk_words,
        params.window_size,
        params.similarity_threshold
    );

    let mut doc_counter: usize = 0;

    for mut doc in rx {
        debug!("{}: Processing '{}'", PLUGIN_NAME, doc.title);

        if !doc.text.is_empty() {
            let chunks = split_and_chunk(&doc.text, &params);
            if !chunks.is_empty() {
                store_chunks_in_faiss(&doc, &chunks, &params);
                // Store chunk metadata in the document's text_parts
                doc.text_parts = chunks
                    .iter()
                    .enumerate()
                    .map(|(i, chunk)| {
                        HashMap::from([
                            ("id".to_string(), Value::String((i + 1).to_string())),
                            ("text".to_string(), Value::String(chunk.clone())),
                            ("insights".to_string(), json!([])),
                        ])
                    })
                    .collect();
            }
        }

        match tx.send(doc) {
            Ok(_) => doc_counter += 1,
            Err(e) => error!("{}: Channel send error: {}", PLUGIN_NAME, e),
        }
    }

    info!("{}: Completed — processed {} documents.", PLUGIN_NAME, doc_counter);
}

// ─── Configuration ─────────────────────────────────────────────────────────────

struct VectorstoreParams {
    vectorstore_path: String,
    vectorstore_script: String,
    min_chunk_words: usize,
    max_chunk_words: usize,
    window_size: usize,
    similarity_threshold: f64,
}

impl VectorstoreParams {
    fn from_config(app_config: &Config) -> Self {
        let data_dir = app_config
            .get_string("data_dir")
            .unwrap_or_else(|_| ".".to_string());

        let vectorstore_path = get_plugin_cfg!(PLUGIN_NAME, "vectorstore_path", app_config)
            .unwrap_or_else(|| format!("{}/vectorstore", data_dir));

        // Auto-detect script path relative to binary location or src tree
        let default_script = find_vectorize_script();
        let vectorstore_script = get_plugin_cfg!(PLUGIN_NAME, "vectorstore_script", app_config)
            .unwrap_or(default_script);

        let min_chunk_words = get_plugin_cfg!(PLUGIN_NAME, "vectorstore_min_chunk_words", app_config)
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(100);

        let max_chunk_words = get_plugin_cfg!(PLUGIN_NAME, "vectorstore_max_chunk_words", app_config)
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(500);

        let window_size = get_plugin_cfg!(PLUGIN_NAME, "vectorstore_window_size", app_config)
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(5);

        let similarity_threshold = get_plugin_cfg!(PLUGIN_NAME, "vectorstore_similarity_threshold", app_config)
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.3);

        VectorstoreParams {
            vectorstore_path,
            vectorstore_script,
            min_chunk_words,
            max_chunk_words,
            window_size,
            similarity_threshold,
        }
    }
}

fn find_vectorize_script() -> String {
    // Look for the script relative to the binary's directory
    let candidates = [
        "scripts/vectorize_chunks.py",
        "../scripts/vectorize_chunks.py",
        "/home/netshare/hdd/llm_storage/src/rust_projs/newslookout_rs/scripts/vectorize_chunks.py",
    ];
    for candidate in &candidates {
        if std::path::Path::new(candidate).exists() {
            return candidate.to_string();
        }
    }
    candidates[0].to_string()
}

// ─── Text splitting ─────────────────────────────────────────────────────────────

/// Split document text into semantic chunks using TextTiling-style boundary detection.
///
/// Strategy:
/// 1. Split text into sentences.
/// 2. Build sliding word-frequency windows of `window_size` sentences each.
/// 3. Compute cosine similarity between adjacent windows.
/// 4. Mark valleys in similarity below `threshold` as chunk boundaries.
/// 5. Merge very small chunks (< min_chunk_words) with neighbours.
/// 6. Split very large chunks (> max_chunk_words) by word count.
pub fn split_and_chunk(text: &str, params: &VectorstoreParams) -> Vec<String> {
    let sentences = split_into_sentences(text);
    if sentences.is_empty() {
        return vec![];
    }

    // Fallback: if text has too few sentences, return as one chunk
    if sentences.len() <= params.window_size * 2 {
        let combined = sentences.join(" ");
        let words: usize = combined.split_whitespace().count();
        if words < params.min_chunk_words {
            return vec![];
        }
        return split_by_max_words(&combined, params.max_chunk_words);
    }

    let boundaries = find_semantic_boundaries(&sentences, params.window_size, params.similarity_threshold);

    // Build initial chunks from boundaries
    let mut chunks: Vec<String> = Vec::new();
    let mut start = 0;
    for &boundary in &boundaries {
        if boundary > start {
            chunks.push(sentences[start..boundary].join(" "));
        }
        start = boundary;
    }
    if start < sentences.len() {
        chunks.push(sentences[start..].join(" "));
    }

    // Merge chunks below min_chunk_words into neighbours
    let chunks = merge_small_chunks(chunks, params.min_chunk_words);

    // Split chunks above max_chunk_words
    let mut result: Vec<String> = Vec::new();
    for chunk in chunks {
        let word_count = chunk.split_whitespace().count();
        if word_count > params.max_chunk_words {
            result.extend(split_by_max_words(&chunk, params.max_chunk_words));
        } else if word_count >= params.min_chunk_words {
            result.push(chunk);
        }
    }

    result
}

/// Split text into sentences using simple heuristics.
fn split_into_sentences(text: &str) -> Vec<String> {
    // Normalize whitespace
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");

    let mut sentences: Vec<String> = Vec::new();
    let mut current = String::new();

    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut i = 0;

    while i < n {
        current.push(chars[i]);
        if chars[i] == '.' || chars[i] == '!' || chars[i] == '?' || chars[i] == '\n' {
            // Check next char — if space or end, this is a sentence boundary
            let next = chars.get(i + 1).copied();
            if next.map_or(true, |c| c == ' ' || c == '\n' || c == '"' || c == '\'') {
                let trimmed = current.trim().to_string();
                if trimmed.split_whitespace().count() >= 3 {
                    sentences.push(trimmed);
                }
                current = String::new();
            }
        }
        i += 1;
    }
    if !current.trim().is_empty() {
        let trimmed = current.trim().to_string();
        if trimmed.split_whitespace().count() >= 3 {
            sentences.push(trimmed);
        }
    }
    sentences
}

/// Build vocabulary frequency map for a window of sentences.
fn build_freq_map(sentences: &[String]) -> HashMap<String, f64> {
    let mut freq: HashMap<String, f64> = HashMap::new();
    let stop_words: HashSet<&str> = ["the","a","an","and","or","but","in","on","at","to","for",
        "of","with","by","is","are","was","were","be","been","being","have","has","had",
        "do","does","did","will","would","could","should","may","might","shall","can",
        "it","its","this","that","these","those","he","she","we","they","i","you","not"].iter().copied().collect();
    for sentence in sentences {
        for word in sentence.split_whitespace() {
            let w: String = word
                .chars()
                .filter(|c| c.is_alphabetic())
                .collect::<String>()
                .to_lowercase();
            if w.len() >= 3 && !stop_words.contains(w.as_str()) {
                *freq.entry(w).or_insert(0.0) += 1.0;
            }
        }
    }
    freq
}

/// Cosine similarity between two frequency maps.
fn cosine_sim(a: &HashMap<String, f64>, b: &HashMap<String, f64>) -> f64 {
    let dot: f64 = a.iter().map(|(k, v)| v * b.get(k).copied().unwrap_or(0.0)).sum();
    let norm_a: f64 = a.values().map(|v| v * v).sum::<f64>().sqrt();
    let norm_b: f64 = b.values().map(|v| v * v).sum::<f64>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 { 0.0 } else { dot / (norm_a * norm_b) }
}

/// Find sentence indices where semantic coherence drops (TextTiling approach).
fn find_semantic_boundaries(sentences: &[String], window_size: usize, threshold: f64) -> Vec<usize> {
    let n = sentences.len();
    if n < window_size * 2 {
        return vec![];
    }

    // Compute similarity scores between adjacent windows at each gap
    let mut scores: Vec<f64> = Vec::new();
    for gap in window_size..n.saturating_sub(window_size) {
        let left_start = gap.saturating_sub(window_size);
        let right_end = (gap + window_size).min(n);
        let left = build_freq_map(&sentences[left_start..gap]);
        let right = build_freq_map(&sentences[gap..right_end]);
        scores.push(cosine_sim(&left, &right));
    }

    // Find valleys: positions where similarity drops below threshold and is a local minimum
    let mut boundaries: Vec<usize> = Vec::new();
    for i in 1..scores.len().saturating_sub(1) {
        let s = scores[i];
        if s < threshold && s <= scores[i - 1] && s <= scores[i + 1] {
            // Gap index: window_size + i
            boundaries.push(window_size + i);
        }
    }

    // Also add explicit double-newline boundaries from original text structure
    boundaries.sort();
    boundaries.dedup();
    boundaries
}

/// Merge consecutive chunks that are below the minimum word count.
fn merge_small_chunks(chunks: Vec<String>, min_words: usize) -> Vec<String> {
    if chunks.is_empty() {
        return chunks;
    }
    let mut result: Vec<String> = Vec::new();
    let mut buf = String::new();

    for chunk in chunks {
        let words = chunk.split_whitespace().count();
        if words < min_words {
            if !buf.is_empty() { buf.push(' '); }
            buf.push_str(&chunk);
        } else {
            if !buf.is_empty() {
                // Merge buffered small chunk into this one
                let combined = format!("{} {}", buf, chunk);
                buf = String::new();
                result.push(combined);
            } else {
                result.push(chunk);
            }
        }
    }
    // Flush remaining buffer
    if !buf.is_empty() {
        if let Some(last) = result.last_mut() {
            last.push(' ');
            last.push_str(&buf);
        } else {
            result.push(buf);
        }
    }
    result
}

/// Split a chunk that is too large by word count, with a small overlap.
fn split_by_max_words(text: &str, max_words: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() <= max_words {
        return vec![text.to_string()];
    }
    let overlap = (max_words / 10).max(20);
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < words.len() {
        let end = (start + max_words).min(words.len());
        chunks.push(words[start..end].join(" "));
        if end == words.len() { break; }
        start = end.saturating_sub(overlap);
    }
    chunks
}

// ─── FAISS storage via Python helper ───────────────────────────────────────────

fn make_doc_id(doc: &Document) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    doc.url.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn store_chunks_in_faiss(doc: &Document, chunks: &[String], params: &VectorstoreParams) {
    if chunks.is_empty() {
        return;
    }

    let index_path = format!("{}/index.faiss", params.vectorstore_path);
    let meta_path = format!("{}/meta.json", params.vectorstore_path);

    // Build JSON payload for the Python script
    let payload = serde_json::json!({
        "doc_id":    make_doc_id(doc),
        "doc_title": doc.title,
        "doc_date":  doc.publish_date,
        "doc_url":   doc.url,
        "chunks":    chunks,
    });

    let payload_str = match serde_json::to_string(&payload) {
        Ok(s) => s,
        Err(e) => {
            error!("{}: Failed to serialize chunks payload: {}", PLUGIN_NAME, e);
            return;
        }
    };

    let output = Command::new("python3")
        .arg(&params.vectorstore_script)
        .arg("--index-path").arg(&index_path)
        .arg("--meta-path").arg(&meta_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(payload_str.as_bytes());
            }
            child.wait_with_output()
        });

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stdout = stdout.trim();
            if let Ok(result) = serde_json::from_str::<serde_json::Value>(stdout) {
                let added = result.get("added").and_then(|v| v.as_u64()).unwrap_or(0);
                let total = result.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
                info!(
                    "{}: Stored {} chunks for '{}' (total in index: {})",
                    PLUGIN_NAME, added, doc.title, total
                );
            } else {
                info!("{}: Vectorstore response: {}", PLUGIN_NAME, stdout);
            }
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            warn!("{}: vectorize_chunks.py failed (status={:?}): {}", PLUGIN_NAME, out.status.code(), stderr.trim());
        }
        Err(e) => {
            warn!("{}: Could not launch vectorize_chunks.py: {}", PLUGIN_NAME, e);
        }
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_params(min: usize, max: usize, window: usize, threshold: f64) -> VectorstoreParams {
        VectorstoreParams {
            vectorstore_path: "/tmp/test_vectorstore".to_string(),
            vectorstore_script: "scripts/vectorize_chunks.py".to_string(),
            min_chunk_words: min,
            max_chunk_words: max,
            window_size: window,
            similarity_threshold: threshold,
        }
    }

    #[test]
    fn test_split_into_sentences_basic() {
        let text = "Hello world. This is a sentence. And another one here.";
        let sents = split_into_sentences(text);
        assert!(sents.len() >= 2, "Expected multiple sentences, got: {:?}", sents);
    }

    #[test]
    fn test_split_by_max_words() {
        let text = (0..200).map(|i| format!("word{}", i)).collect::<Vec<_>>().join(" ");
        let chunks = split_by_max_words(&text, 100);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].split_whitespace().count() == 100);
    }

    #[test]
    fn test_merge_small_chunks() {
        let chunks = vec![
            "a b c".to_string(),          // 3 words - small
            "hello world".to_string(),    // 2 words - small
            "the quick brown fox jumps over the lazy dog and then some more words here today yesterday".to_string(), // large
        ];
        let merged = merge_small_chunks(chunks, 10);
        // Small chunks should be merged
        assert!(merged.len() < 3, "Expected merging, got {} chunks", merged.len());
    }

    #[test]
    fn test_cosine_sim_identical() {
        let mut a = HashMap::new();
        a.insert("finance".to_string(), 2.0);
        a.insert("market".to_string(), 1.0);
        let sim = cosine_sim(&a, &a);
        assert!((sim - 1.0).abs() < 1e-6, "Identical vectors should have sim=1.0");
    }

    #[test]
    fn test_cosine_sim_orthogonal() {
        let mut a = HashMap::new();
        a.insert("finance".to_string(), 1.0);
        let mut b = HashMap::new();
        b.insert("quantum".to_string(), 1.0);
        let sim = cosine_sim(&a, &b);
        assert!(sim < 1e-6, "Orthogonal vectors should have sim≈0");
    }

    #[test]
    fn test_split_and_chunk_short_text() {
        let params = make_params(50, 200, 3, 0.3);
        let short = "Too short to chunk. Only a few words here.";
        let chunks = split_and_chunk(short, &params);
        assert!(chunks.is_empty(), "Short text should produce no chunks");
    }

    #[test]
    fn test_split_and_chunk_long_text() {
        let params = make_params(20, 100, 2, 0.4);
        // Generate two topically distinct sections
        let section1: String = (0..15).map(|_| "The central bank raised interest rates to curb inflation expectations in the economy.").collect::<Vec<_>>().join(" ");
        let section2: String = (0..15).map(|_| "Scientists discovered new quantum computing algorithms that break encryption protocols.").collect::<Vec<_>>().join(" ");
        let text = format!("{}\n\n{}", section1, section2);
        let chunks = split_and_chunk(&text, &params);
        assert!(!chunks.is_empty(), "Long text should produce chunks");
        for chunk in &chunks {
            let words = chunk.split_whitespace().count();
            assert!(words <= params.max_chunk_words + 50, // small tolerance for sentence boundaries
                "Chunk too large: {} words", words);
        }
    }

    #[test]
    fn test_build_freq_map() {
        let sents = vec!["The financial markets rose significantly today.".to_string()];
        let freq = build_freq_map(&sents);
        assert!(freq.contains_key("financial"));
        assert!(freq.contains_key("markets"));
        // Stop words should be filtered
        assert!(!freq.contains_key("the"));
    }
}
