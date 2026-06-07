// file: mod_vectorstore.rs
// Text splitting + semantic chunking + ONNX embedding + SQLite vector storage.
//
// Replaces the earlier Python subprocess. Embeddings are generated with the
// all-mpnet-base-v2 ONNX model via the `ort` crate (ONNX Runtime) and stored
// as raw f32 BLOBs in SQLite alongside chunk metadata.
//
// Config keys:
//   vectorstore_path               - Dir for the SQLite DB (default: data_dir/vectorstore)
//   vectorstore_model_dir          - Dir with model.onnx + vocab.txt (default: auto-detect)
//   vectorstore_min_chunk_words    - Min words per chunk (default: 100)
//   vectorstore_max_chunk_words    - Max words per chunk (default: 500)
//   vectorstore_window_size        - TextTiling window in sentences (default: 5)
//   vectorstore_similarity_threshold - Boundary threshold (default: 0.3)

use std::collections::{HashMap, HashSet};
use std::fs;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use config::Config;
use log::{debug, error, info, warn};
use ort::session::Session;
use ort::value::Tensor;
use rusqlite::{params, Connection};
use serde_json::{json, Value};

use crate::document::Document;
use crate::get_plugin_cfg;

pub const PLUGIN_NAME: &str = "mod_vectorstore";
const EMBED_DIM: usize = 768;
const MAX_SEQ_LEN: usize = 512;

fn load_session(model_dir: &str) -> Option<Session> {
    let model_path = format!("{}/model.onnx", model_dir);
    if !std::path::Path::new(&model_path).exists() {
        warn!(
            "{}: ONNX model not found at '{}'. Vector embedding disabled.",
            PLUGIN_NAME, model_path
        );
        return None;
    }
    match Session::builder().and_then(|mut b| b.commit_from_file(&model_path)) {
        Ok(s) => {
            info!("{}: Loaded ONNX model from '{}'.", PLUGIN_NAME, model_path);
            Some(s)
        }
        Err(e) => {
            warn!(
                "{}: Failed to load ONNX model '{}': {}. Embedding disabled.",
                PLUGIN_NAME, model_path, e
            );
            None
        }
    }
}

// ─── Public entry point ───────────────────────────────────────────────────────

pub fn process_data(
    tx: Sender<Document>,
    rx: Receiver<Document>,
    app_config: &Config,
    _api_mutexes: &mut HashMap<String, Arc<Mutex<isize>>>,
) {
    info!("{}: Starting — loading configuration.", PLUGIN_NAME);

    let params = VectorstoreParams::from_config(app_config);
    info!(
        "{}: path='{}', model='{}', chunk_words=[{}-{}], window={}, threshold={:.2}",
        PLUGIN_NAME,
        params.vectorstore_path,
        params.model_dir,
        params.min_chunk_words,
        params.max_chunk_words,
        params.window_size,
        params.similarity_threshold
    );

    if let Err(e) = fs::create_dir_all(&params.vectorstore_path) {
        error!(
            "{}: Cannot create vectorstore dir '{}': {}",
            PLUGIN_NAME, params.vectorstore_path, e
        );
    }

    let tokenizer = BertTokenizer::from_dir(&params.model_dir);
    if tokenizer.is_none() {
        warn!("{}: Tokenizer unavailable — chunks split but not embedded.", PLUGIN_NAME);
    }

    let mut session = load_session(&params.model_dir);

    let db_path = format!("{}/vectors.db", params.vectorstore_path);
    let db_conn = match open_vector_db(&db_path) {
        Ok(c) => Some(c),
        Err(e) => {
            error!(
                "{}: Failed to open vector DB '{}': {}",
                PLUGIN_NAME, db_path, e
            );
            None
        }
    };

    let mut doc_counter: usize = 0;

    for mut doc in rx {
        debug!("{}: Processing url={}", PLUGIN_NAME, doc.url);

        if !doc.text.is_empty() {
            let chunks = split_and_chunk(&doc.text, &params);
            if !chunks.is_empty() {
                if let (Some(tok), Some(ref conn), Some(ref mut sess)) =
                    (tokenizer.as_ref(), db_conn.as_ref(), session.as_mut())
                {
                    embed_and_store(&doc, &chunks, tok, conn, sess);
                }
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

// ─── Configuration ────────────────────────────────────────────────────────────

struct VectorstoreParams {
    vectorstore_path: String,
    model_dir: String,
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

        let model_dir = get_plugin_cfg!(PLUGIN_NAME, "vectorstore_model_dir", app_config)
            .unwrap_or_else(find_model_dir);

        let min_chunk_words =
            get_plugin_cfg!(PLUGIN_NAME, "vectorstore_min_chunk_words", app_config)
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(100);

        let max_chunk_words =
            get_plugin_cfg!(PLUGIN_NAME, "vectorstore_max_chunk_words", app_config)
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(500);

        let window_size = get_plugin_cfg!(PLUGIN_NAME, "vectorstore_window_size", app_config)
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(5);

        let similarity_threshold =
            get_plugin_cfg!(PLUGIN_NAME, "vectorstore_similarity_threshold", app_config)
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.3);

        VectorstoreParams {
            vectorstore_path,
            model_dir,
            min_chunk_words,
            max_chunk_words,
            window_size,
            similarity_threshold,
        }
    }
}

fn find_model_dir() -> String {
    let candidates = [
        "/home/netshare/hdd/downloader/models/sentence-transformers_all-mpnet-base-v2",
        "models/sentence-transformers_all-mpnet-base-v2",
    ];
    for c in &candidates {
        if std::path::Path::new(c).exists() {
            return c.to_string();
        }
    }
    candidates[0].to_string()
}

// ─── BERT WordPiece tokenizer ─────────────────────────────────────────────────
// Loads vocab.txt (one token per line → token ID = line index).
// Implements lowercase, whitespace/punctuation split, WordPiece subword
// tokenization, [CLS]/[SEP] framing, and truncation to max_length.

pub struct BertTokenizer {
    vocab: HashMap<String, i64>,
    cls_id: i64,
    sep_id: i64,
    unk_id: i64,
}

impl BertTokenizer {
    pub fn from_dir(model_dir: &str) -> Option<Self> {
        let vocab_path = format!("{}/vocab.txt", model_dir);
        let text = match fs::read_to_string(&vocab_path) {
            Ok(t) => t,
            Err(e) => {
                warn!(
                    "{}: vocab.txt not found at '{}': {}",
                    PLUGIN_NAME, vocab_path, e
                );
                return None;
            }
        };
        let mut vocab = HashMap::new();
        for (i, line) in text.lines().enumerate() {
            vocab.insert(line.trim().to_string(), i as i64);
        }
        let cls_id = *vocab.get("[CLS]").unwrap_or(&101);
        let sep_id = *vocab.get("[SEP]").unwrap_or(&102);
        let unk_id = *vocab.get("[UNK]").unwrap_or(&100);
        Some(BertTokenizer { vocab, cls_id, sep_id, unk_id })
    }

    /// Tokenize `text` → (input_ids, attention_mask), both len <= max_length.
    pub fn tokenize(&self, text: &str, max_length: usize) -> (Vec<i64>, Vec<i64>) {
        let lower = text.to_lowercase();
        let words = basic_tokenize(&lower);

        let mut ids: Vec<i64> = vec![self.cls_id];
        for word in &words {
            let wp = self.wordpiece(word);
            if ids.len() + wp.len() + 1 > max_length {
                break;
            }
            ids.extend_from_slice(&wp);
        }
        ids.push(self.sep_id);

        let len = ids.len();
        (ids, vec![1i64; len])
    }

    fn wordpiece(&self, token: &str) -> Vec<i64> {
        if token.is_empty() {
            return vec![];
        }
        if let Some(&id) = self.vocab.get(token) {
            return vec![id];
        }
        let chars: Vec<char> = token.chars().collect();
        let mut result = Vec::new();
        let mut start = 0;

        while start < chars.len() {
            let mut end = chars.len();
            let mut found = false;
            while start < end {
                let substr: String = chars[start..end].iter().collect();
                let candidate = if start == 0 {
                    substr
                } else {
                    format!("##{}", substr)
                };
                if let Some(&id) = self.vocab.get(&candidate) {
                    result.push(id);
                    start = end;
                    found = true;
                    break;
                }
                end -= 1;
            }
            if !found {
                return vec![self.unk_id];
            }
        }
        result
    }
}

fn basic_tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(current.clone());
                current.clear();
            }
        } else if is_punctuation(ch) {
            if !current.is_empty() {
                tokens.push(current.clone());
                current.clear();
            }
            tokens.push(ch.to_string());
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn is_punctuation(c: char) -> bool {
    let cp = c as u32;
    (cp >= 33 && cp <= 47)
        || (cp >= 58 && cp <= 64)
        || (cp >= 91 && cp <= 96)
        || (cp >= 123 && cp <= 126)
}

// ─── SQLite vector storage ────────────────────────────────────────────────────

const CREATE_TABLE_SQL: &str = "
CREATE TABLE IF NOT EXISTS chunk_vectors (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    doc_id      TEXT NOT NULL,
    doc_title   TEXT,
    doc_date    TEXT,
    doc_url     TEXT,
    chunk_index INTEGER,
    chunk_text  TEXT,
    vector      BLOB NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_cv_doc ON chunk_vectors (doc_id);
CREATE INDEX IF NOT EXISTS idx_cv_url ON chunk_vectors (doc_url);
";

fn open_vector_db(path: &str) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
    conn.execute_batch(CREATE_TABLE_SQL)?;
    Ok(conn)
}

fn make_doc_id(doc: &Document) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    doc.url.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

// ─── Embedding + storage ──────────────────────────────────────────────────────

fn embed_and_store(
    doc: &Document,
    chunks: &[String],
    tokenizer: &BertTokenizer,
    conn: &Connection,
    session: &mut Session,
) {

    // Tokenise all chunks; track original mask lengths for mean-pooling
    let mut all_ids: Vec<Vec<i64>> = Vec::new();
    let mut all_masks: Vec<Vec<i64>> = Vec::new();
    let mut max_len = 0usize;

    for chunk in chunks {
        let (ids, mask) = tokenizer.tokenize(chunk, MAX_SEQ_LEN);
        max_len = max_len.max(ids.len());
        all_ids.push(ids);
        all_masks.push(mask);
    }

    if max_len == 0 {
        return;
    }

    let batch = all_ids.len();
    let mut flat_ids = vec![0i64; batch * max_len];
    let mut flat_masks = vec![0i64; batch * max_len];

    for (i, (ids, masks)) in all_ids.iter().zip(all_masks.iter()).enumerate() {
        for (j, (&id, &mask)) in ids.iter().zip(masks.iter()).enumerate() {
            flat_ids[i * max_len + j] = id;
            flat_masks[i * max_len + j] = mask;
        }
    }

    // Build ORT tensors from (shape, flat_vec) tuples
    let ids_tensor = match Tensor::<i64>::from_array(([batch, max_len], flat_ids)) {
        Ok(t) => t,
        Err(e) => {
            error!(
                "{}: Tensor build failed for url={}: {}",
                PLUGIN_NAME, doc.url, e
            );
            return;
        }
    };
    let mask_tensor = match Tensor::<i64>::from_array(([batch, max_len], flat_masks)) {
        Ok(t) => t,
        Err(e) => {
            error!(
                "{}: Tensor build failed for url={}: {}",
                PLUGIN_NAME, doc.url, e
            );
            return;
        }
    };

    let outputs = match session.run(ort::inputs![ids_tensor, mask_tensor]) {
        Ok(o) => o,
        Err(e) => {
            error!(
                "{}: ONNX inference failed for url={}: {}",
                PLUGIN_NAME, doc.url, e
            );
            return;
        }
    };

    // Output 0: token_embeddings [batch, seq_len, 768]
    let (shape, token_emb) = match outputs[0].try_extract_tensor::<f32>() {
        Ok(pair) => pair,
        Err(e) => {
            error!(
                "{}: Tensor extract failed for url={}: {}",
                PLUGIN_NAME, doc.url, e
            );
            return;
        }
    };

    if shape.len() < 3 {
        error!(
            "{}: Unexpected output shape {:?} for url={}",
            PLUGIN_NAME, shape, doc.url
        );
        return;
    }
    let (out_batch, seq_len, emb_dim) = (shape[0] as usize, shape[1] as usize, shape[2] as usize);

    let doc_id = make_doc_id(doc);

    for chunk_idx in 0..batch.min(out_batch) {
        // Mean-pool token embeddings over non-padding positions
        let mut sum = vec![0f32; emb_dim];
        let mut count = 0f32;

        for s in 0..seq_len.min(max_len) {
            let is_real = all_masks[chunk_idx].get(s).copied().unwrap_or(0) == 1;
            if is_real {
                let offset = (chunk_idx * seq_len + s) * emb_dim;
                for d in 0..emb_dim {
                    sum[d] += token_emb[offset + d];
                }
                count += 1.0;
            }
        }

        if count == 0.0 {
            continue;
        }
        let denom = count;
        let mut emb: Vec<f32> = sum.iter().map(|v| v / denom).collect();

        // L2 normalise
        let norm: f32 = emb.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-9);
        for v in &mut emb { *v /= norm; }

        let blob: Vec<u8> = emb.iter().flat_map(|v| v.to_le_bytes()).collect();
        let preview: String = chunks[chunk_idx].chars().take(500).collect();

        if let Err(e) = conn.execute(
            "INSERT INTO chunk_vectors
                (doc_id, doc_title, doc_date, doc_url, chunk_index, chunk_text, vector)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                doc_id,
                doc.title,
                doc.publish_date,
                doc.url,
                chunk_idx as i64,
                preview,
                blob,
            ],
        ) {
            error!(
                "{}: DB insert failed for url={}: {}",
                PLUGIN_NAME, doc.url, e
            );
        }
    }

    info!(
        "{}: Stored {} chunks (url={})",
        PLUGIN_NAME,
        batch.min(out_batch),
        doc.url
    );
}

// ─── Text splitting ───────────────────────────────────────────────────────────

pub fn split_and_chunk(text: &str, params: &VectorstoreParams) -> Vec<String> {
    let sentences = split_into_sentences(text);
    if sentences.is_empty() {
        return vec![];
    }

    if sentences.len() <= params.window_size * 2 {
        let combined = sentences.join(" ");
        if combined.split_whitespace().count() < params.min_chunk_words {
            return vec![];
        }
        return split_by_max_words(&combined, params.max_chunk_words);
    }

    let boundaries =
        find_semantic_boundaries(&sentences, params.window_size, params.similarity_threshold);

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

    let chunks = merge_small_chunks(chunks, params.min_chunk_words);

    let mut result: Vec<String> = Vec::new();
    for chunk in chunks {
        let wc = chunk.split_whitespace().count();
        if wc > params.max_chunk_words {
            result.extend(split_by_max_words(&chunk, params.max_chunk_words));
        } else if wc >= params.min_chunk_words {
            result.push(chunk);
        }
    }
    result
}

fn split_into_sentences(text: &str) -> Vec<String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut sentences: Vec<String> = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut i = 0;

    while i < n {
        current.push(chars[i]);
        if matches!(chars[i], '.' | '!' | '?' | '\n') {
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

fn build_freq_map(sentences: &[String]) -> HashMap<String, f64> {
    let stop_words: HashSet<&str> = [
        "the", "a", "an", "and", "or", "but", "in", "on", "at", "to", "for", "of", "with", "by",
        "is", "are", "was", "were", "be", "been", "being", "have", "has", "had", "do", "does",
        "did", "will", "would", "could", "should", "may", "might", "shall", "can", "it", "its",
        "this", "that", "these", "those", "he", "she", "we", "they", "i", "you", "not",
    ]
    .iter()
    .copied()
    .collect();

    let mut freq: HashMap<String, f64> = HashMap::new();
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

fn cosine_sim(a: &HashMap<String, f64>, b: &HashMap<String, f64>) -> f64 {
    let dot: f64 = a.iter().map(|(k, v)| v * b.get(k).copied().unwrap_or(0.0)).sum();
    let norm_a: f64 = a.values().map(|v| v * v).sum::<f64>().sqrt();
    let norm_b: f64 = b.values().map(|v| v * v).sum::<f64>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

fn find_semantic_boundaries(
    sentences: &[String],
    window_size: usize,
    threshold: f64,
) -> Vec<usize> {
    let n = sentences.len();
    if n < window_size * 2 {
        return vec![];
    }
    let mut scores: Vec<f64> = Vec::new();
    for gap in window_size..n.saturating_sub(window_size) {
        let left = build_freq_map(&sentences[gap.saturating_sub(window_size)..gap]);
        let right = build_freq_map(&sentences[gap..(gap + window_size).min(n)]);
        scores.push(cosine_sim(&left, &right));
    }
    let mut boundaries: Vec<usize> = Vec::new();
    for i in 1..scores.len().saturating_sub(1) {
        let s = scores[i];
        if s < threshold && s <= scores[i - 1] && s <= scores[i + 1] {
            boundaries.push(window_size + i);
        }
    }
    boundaries.sort();
    boundaries.dedup();
    boundaries
}

fn merge_small_chunks(chunks: Vec<String>, min_words: usize) -> Vec<String> {
    if chunks.is_empty() {
        return chunks;
    }
    let mut result: Vec<String> = Vec::new();
    let mut buf = String::new();

    for chunk in chunks {
        if chunk.split_whitespace().count() < min_words {
            if !buf.is_empty() {
                buf.push(' ');
            }
            buf.push_str(&chunk);
        } else if !buf.is_empty() {
            result.push(format!("{} {}", buf, chunk));
            buf = String::new();
        } else {
            result.push(chunk);
        }
    }
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
        if end == words.len() {
            break;
        }
        start = end.saturating_sub(overlap);
    }
    chunks
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_params(min: usize, max: usize, window: usize, threshold: f64) -> VectorstoreParams {
        VectorstoreParams {
            vectorstore_path: "/tmp/test_vectorstore".to_string(),
            model_dir: "/tmp/nonexistent_model".to_string(),
            min_chunk_words: min,
            max_chunk_words: max,
            window_size: window,
            similarity_threshold: threshold,
        }
    }

    // ── Text splitting ────────────────────────────────────────────────────────

    #[test]
    fn test_split_into_sentences_basic() {
        let text = "Hello world today. This is a sentence. And another one here.";
        let sents = split_into_sentences(text);
        assert!(sents.len() >= 2, "Expected multiple sentences, got: {:?}", sents);
    }

    #[test]
    fn test_split_by_max_words_splits_large() {
        let text = (0..200).map(|i| format!("word{}", i)).collect::<Vec<_>>().join(" ");
        let chunks = split_by_max_words(&text, 100);
        assert!(chunks.len() >= 2, "Expected ≥2 chunks, got {}", chunks.len());
        for chunk in &chunks {
            assert!(chunk.split_whitespace().count() <= 100, "Chunk exceeds max_words");
        }
    }

    #[test]
    fn test_split_by_max_words_short_unchanged() {
        let short = "one two three";
        let chunks = split_by_max_words(short, 100);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], short);
    }

    #[test]
    fn test_merge_small_chunks() {
        let chunks = vec![
            "a b c".to_string(),
            "hello world".to_string(),
            "the quick brown fox jumps over the lazy dog and then some words here today".to_string(),
        ];
        let merged = merge_small_chunks(chunks, 10);
        assert!(merged.len() < 3, "Expected merging, got {} chunks", merged.len());
    }

    #[test]
    fn test_cosine_sim_identical() {
        let mut a = HashMap::new();
        a.insert("finance".to_string(), 2.0);
        a.insert("market".to_string(), 1.0);
        assert!((cosine_sim(&a, &a) - 1.0).abs() < 1e-6, "Identical → sim≈1.0");
    }

    #[test]
    fn test_cosine_sim_orthogonal() {
        let mut a = HashMap::new();
        a.insert("finance".to_string(), 1.0);
        let mut b = HashMap::new();
        b.insert("quantum".to_string(), 1.0);
        assert!(cosine_sim(&a, &b) < 1e-6, "Orthogonal → sim≈0");
    }

    #[test]
    fn test_split_and_chunk_short_text() {
        let params = make_params(50, 200, 3, 0.3);
        assert!(
            split_and_chunk("Too short.", &params).is_empty(),
            "Short text should produce no chunks"
        );
    }

    #[test]
    fn test_split_and_chunk_long_text() {
        let params = make_params(20, 100, 2, 0.4);
        let sec1 = std::iter::repeat("The central bank raised interest rates to curb inflation.")
            .take(15)
            .collect::<Vec<_>>()
            .join(" ");
        let sec2 =
            std::iter::repeat("Scientists discovered new quantum algorithms breaking encryption.")
                .take(15)
                .collect::<Vec<_>>()
                .join(" ");
        let chunks = split_and_chunk(&format!("{}\n\n{}", sec1, sec2), &params);
        assert!(!chunks.is_empty(), "Long text should produce chunks");
        for chunk in &chunks {
            let wc = chunk.split_whitespace().count();
            assert!(
                wc <= params.max_chunk_words + 50,
                "Chunk too large: {} words",
                wc
            );
        }
    }

    #[test]
    fn test_build_freq_map_filters_stopwords() {
        let sents = vec!["The financial markets rose significantly today.".to_string()];
        let freq = build_freq_map(&sents);
        assert!(freq.contains_key("financial"));
        assert!(freq.contains_key("markets"));
        assert!(!freq.contains_key("the"), "Stop word should be filtered");
    }

    // ── Tokenizer ─────────────────────────────────────────────────────────────

    #[test]
    fn test_basic_tokenize_splits_punctuation() {
        let tokens = basic_tokenize("hello, world!");
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
        assert!(tokens.contains(&",".to_string()));
    }

    #[test]
    fn test_basic_tokenize_empty() {
        assert!(basic_tokenize("").is_empty());
        assert!(basic_tokenize("   ").is_empty());
    }

    fn minimal_tokenizer() -> BertTokenizer {
        let mut vocab = HashMap::new();
        vocab.insert("[CLS]".to_string(), 101i64);
        vocab.insert("[SEP]".to_string(), 102i64);
        vocab.insert("[UNK]".to_string(), 100i64);
        vocab.insert("hello".to_string(), 7592i64);
        vocab.insert("world".to_string(), 2088i64);
        BertTokenizer { vocab, cls_id: 101, sep_id: 102, unk_id: 100 }
    }

    #[test]
    fn test_tokenize_adds_cls_sep() {
        let tok = minimal_tokenizer();
        let (ids, mask) = tok.tokenize("hello world", 16);
        assert_eq!(ids[0], 101, "First token should be [CLS]");
        assert_eq!(*ids.last().unwrap(), 102, "Last token should be [SEP]");
        assert_eq!(ids.len(), mask.len());
        assert!(mask.iter().all(|&m| m == 1));
    }

    #[test]
    fn test_tokenize_truncates_at_max_length() {
        let mut vocab = HashMap::new();
        vocab.insert("[CLS]".to_string(), 101i64);
        vocab.insert("[SEP]".to_string(), 102i64);
        vocab.insert("[UNK]".to_string(), 100i64);
        for i in 0u64..100 {
            vocab.insert(format!("word{}", i), (i + 1000) as i64);
        }
        let tok = BertTokenizer { vocab, cls_id: 101, sep_id: 102, unk_id: 100 };
        let long_text = (0..100).map(|i| format!("word{}", i)).collect::<Vec<_>>().join(" ");
        let (ids, mask) = tok.tokenize(&long_text, 10);
        assert!(ids.len() <= 10, "Should be truncated to 10");
        assert_eq!(ids.len(), mask.len());
    }

    #[test]
    fn test_wordpiece_unk_for_unknown_token() {
        let tok = minimal_tokenizer();
        assert_eq!(tok.wordpiece("xyzxyzxyz"), vec![100i64]);
    }

    #[test]
    fn test_wordpiece_known_token() {
        let tok = minimal_tokenizer();
        assert_eq!(tok.wordpiece("hello"), vec![7592i64]);
    }

    // ── Vector serialization ──────────────────────────────────────────────────

    #[test]
    fn test_vector_roundtrip() {
        let emb: Vec<f32> = (0..EMBED_DIM).map(|i| i as f32 * 0.001).collect();
        let blob: Vec<u8> = emb.iter().flat_map(|v| v.to_le_bytes()).collect();
        let recovered: Vec<f32> = blob
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        assert_eq!(emb.len(), recovered.len());
        for (a, b) in emb.iter().zip(recovered.iter()) {
            assert!((a - b).abs() < 1e-7, "Float roundtrip mismatch");
        }
    }

    // ── SQLite schema ─────────────────────────────────────────────────────────

    #[test]
    fn test_open_vector_db_creates_schema() {
        let path = "/tmp/test_vectorstore_schema_unit.db";
        let _ = std::fs::remove_file(path);
        let conn = open_vector_db(path).expect("Should open DB");
        let count: i64 = conn
            .query_row("SELECT count(*) FROM chunk_vectors", [], |r| r.get(0))
            .expect("Table should exist after schema creation");
        assert_eq!(count, 0);
        std::fs::remove_file(path).ok();
    }
}
