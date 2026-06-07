#!/usr/bin/env python3
"""
vectorize_chunks.py

Generates sentence embeddings for text chunks and stores them in a FAISS index.

Called by the Rust mod_vectorstore plugin with JSON on stdin:
  {
    "doc_id": "<hash>",
    "doc_title": "...",
    "doc_date": "YYYY-MM-DD",
    "doc_url": "...",
    "chunks": ["chunk text 1", "chunk text 2", ...]
  }

Appends embeddings to the FAISS index at --index-path and metadata to --meta-path.

Usage (from Rust subprocess or standalone):
  echo '<json>' | python3 vectorize_chunks.py --index-path /path/to/index.faiss --meta-path /path/to/meta.json
"""

import sys
import json
import argparse
import os
import warnings
warnings.filterwarnings('ignore')

import numpy as np

MODEL_PATH = '/home/netshare/hdd/downloader/models/sentence-transformers_all-mpnet-base-v2'
DIM = 768


def load_tokenizer_and_session():
    """Load ONNX session and tokenizer. Lazy-loaded on first call."""
    import onnxruntime as ort
    from transformers import AutoTokenizer

    ort_opts = ort.SessionOptions()
    ort_opts.log_severity_level = 3  # suppress warnings

    sess = ort.InferenceSession(
        os.path.join(MODEL_PATH, 'model.onnx'),
        sess_options=ort_opts,
        providers=['CPUExecutionProvider'],
    )
    tokenizer = AutoTokenizer.from_pretrained(MODEL_PATH)
    return sess, tokenizer


def embed_texts(texts: list[str], sess, tokenizer, batch_size: int = 32) -> np.ndarray:
    """Generate L2-normalised 768-dim embeddings for a list of texts."""
    all_embeddings = []
    for i in range(0, len(texts), batch_size):
        batch = texts[i:i + batch_size]
        enc = tokenizer(
            batch,
            return_tensors='np',
            padding=True,
            truncation=True,
            max_length=512,
        )
        hidden, _ = sess.run(
            None,
            {'input_ids': enc['input_ids'], 'attention_mask': enc['attention_mask']},
        )
        # Mean pool over sequence dimension with attention mask
        mask = enc['attention_mask'][:, :, np.newaxis].astype(np.float32)
        pooled = (hidden * mask).sum(axis=1) / mask.sum(axis=1)
        # L2 normalise
        norms = np.linalg.norm(pooled, axis=1, keepdims=True)
        norms = np.where(norms == 0, 1.0, norms)
        pooled = pooled / norms
        all_embeddings.append(pooled.astype(np.float32))
    return np.vstack(all_embeddings) if all_embeddings else np.zeros((0, DIM), dtype=np.float32)


def load_or_create_index(index_path: str):
    """Load existing FAISS flat inner-product index or create new one."""
    import faiss
    if os.path.exists(index_path):
        return faiss.read_index(index_path)
    # IndexFlatIP for cosine similarity (vectors are L2-normalised)
    return faiss.IndexFlatIP(DIM)


def load_meta(meta_path: str) -> list[dict]:
    if os.path.exists(meta_path):
        with open(meta_path, 'r', encoding='utf-8') as f:
            return json.load(f)
    return []


def save_meta(meta_path: str, meta: list[dict]):
    with open(meta_path, 'w', encoding='utf-8') as f:
        json.dump(meta, f, ensure_ascii=False)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--index-path', default='/home/netshare/hdd/downloader/vectorstore/index.faiss',
                        help='Path to FAISS index file')
    parser.add_argument('--meta-path', default='/home/netshare/hdd/downloader/vectorstore/meta.json',
                        help='Path to JSON metadata file')
    parser.add_argument('--input', '-i', help='Input JSON file (default: read stdin)')
    args = parser.parse_args()

    # Read input
    if args.input:
        with open(args.input, 'r') as f:
            payload = json.load(f)
    else:
        payload = json.load(sys.stdin)

    chunks = payload.get('chunks', [])
    if not chunks:
        print('{"status": "ok", "added": 0}')
        return 0

    # Ensure output directories exist
    os.makedirs(os.path.dirname(args.index_path), exist_ok=True)
    os.makedirs(os.path.dirname(args.meta_path), exist_ok=True)

    sess, tokenizer = load_tokenizer_and_session()
    embeddings = embed_texts(chunks, sess, tokenizer)

    import faiss
    index = load_or_create_index(args.index_path)
    start_id = index.ntotal
    index.add(embeddings)
    faiss.write_index(index, args.index_path)

    meta = load_meta(args.meta_path)
    for i, chunk_text in enumerate(chunks):
        meta.append({
            'id': start_id + i,
            'doc_id': payload.get('doc_id', ''),
            'doc_title': payload.get('doc_title', ''),
            'doc_date': payload.get('doc_date', ''),
            'doc_url': payload.get('doc_url', ''),
            'chunk_index': i,
            'chunk_text': chunk_text[:500],  # store first 500 chars for preview
        })
    save_meta(args.meta_path, meta)

    result = {'status': 'ok', 'added': len(chunks), 'total': index.ntotal}
    print(json.dumps(result))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
