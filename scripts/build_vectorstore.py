#!/usr/bin/env python3
"""
build_vectorstore.py

Rebuilds the FAISS vectorstore from all saved article JSON files in the archive.
Uses sentence-transformers (all-mpnet-base-v2 via ONNX) for embeddings,
TextTiling-style semantic chunking, and a FAISS IndexFlatIP index.

Usage:
  python3 build_vectorstore.py --archive-dir /path/to/archive [--vectorstore-dir /path/to/vs]
                                [--batch-size 32] [--resume]

The --resume flag skips doc_ids already present in the existing meta.json.
"""

import argparse
import io
import json
import os
import sys
import warnings
import zipfile
from pathlib import Path

warnings.filterwarnings('ignore')

import numpy as np

MODEL_PATH = '/home/netshare/hdd/downloader/models/sentence-transformers_all-mpnet-base-v2'
DIM = 768
MIN_CHUNK_WORDS = 100
MAX_CHUNK_WORDS = 500
WINDOW_SENTENCES = 5
SIM_THRESHOLD = 0.30

STOP_WORDS = {
    'the','a','an','and','or','but','in','on','at','to','for','of','with','by',
    'is','are','was','were','be','been','being','have','has','had','do','does',
    'did','will','would','could','should','may','might','shall','can','it','its',
    'this','that','these','those','he','she','we','they','i','you','not',
}


# ─── Embedding ────────────────────────────────────────────────────────────────

def load_model():
    import onnxruntime as ort
    from transformers import AutoTokenizer
    opts = ort.SessionOptions()
    opts.log_severity_level = 3
    sess = ort.InferenceSession(
        os.path.join(MODEL_PATH, 'model.onnx'),
        sess_options=opts,
        providers=['CPUExecutionProvider'],
    )
    tokenizer = AutoTokenizer.from_pretrained(MODEL_PATH)
    return sess, tokenizer


def embed(texts: list[str], sess, tokenizer, batch_size=32) -> np.ndarray:
    out = []
    for i in range(0, len(texts), batch_size):
        batch = texts[i:i + batch_size]
        enc = tokenizer(batch, return_tensors='np', padding=True, truncation=True, max_length=512)
        hidden, _ = sess.run(None, {'input_ids': enc['input_ids'], 'attention_mask': enc['attention_mask']})
        mask = enc['attention_mask'][:, :, np.newaxis].astype(np.float32)
        pooled = (hidden * mask).sum(1) / mask.sum(1)
        norms = np.linalg.norm(pooled, axis=1, keepdims=True)
        pooled = pooled / np.where(norms == 0, 1, norms)
        out.append(pooled.astype(np.float32))
    return np.vstack(out) if out else np.zeros((0, DIM), dtype=np.float32)


# ─── Semantic chunking ─────────────────────────────────────────────────────────

def split_sentences(text: str) -> list[str]:
    sents, cur = [], ''
    for ch in ' '.join(text.split()):
        cur += ch
        if ch in '.!?\n' and len(cur.split()) >= 3:
            sents.append(cur.strip())
            cur = ''
    if cur.strip() and len(cur.split()) >= 3:
        sents.append(cur.strip())
    return sents


def freq_vec(sents: list[str]) -> dict[str, float]:
    freq: dict[str, float] = {}
    for s in sents:
        for w in s.split():
            w = ''.join(c for c in w if c.isalpha()).lower()
            if len(w) >= 3 and w not in STOP_WORDS:
                freq[w] = freq.get(w, 0) + 1
    return freq


def cosine(a: dict, b: dict) -> float:
    dot = sum(a[k] * b.get(k, 0) for k in a)
    na = sum(v*v for v in a.values()) ** 0.5
    nb = sum(v*v for v in b.values()) ** 0.5
    return dot / (na * nb) if na and nb else 0.0


def semantic_chunks(text: str) -> list[str]:
    sents = split_sentences(text)
    n = len(sents)
    w = WINDOW_SENTENCES

    if n <= w * 2:
        words = text.split()
        if len(words) < MIN_CHUNK_WORDS:
            return []
        return _word_chunks(text)

    # Compute similarity at each inter-sentence gap
    scores = [cosine(freq_vec(sents[max(0, g-w):g]), freq_vec(sents[g:min(n, g+w)]))
              for g in range(w, n - w)]

    # Boundaries: local minima below threshold
    bounds = [0]
    for i in range(1, len(scores)-1):
        if scores[i] < SIM_THRESHOLD and scores[i] <= scores[i-1] and scores[i] <= scores[i+1]:
            bounds.append(w + i)
    bounds.append(n)

    chunks = [' '.join(sents[bounds[i]:bounds[i+1]]) for i in range(len(bounds)-1)]

    # Merge tiny chunks
    merged, buf = [], ''
    for c in chunks:
        if len(c.split()) < MIN_CHUNK_WORDS:
            buf = (buf + ' ' + c).strip()
        else:
            if buf:
                c = (buf + ' ' + c).strip()
                buf = ''
            merged.append(c)
    if buf:
        if merged:
            merged[-1] = (merged[-1] + ' ' + buf).strip()
        else:
            merged.append(buf)

    result = []
    for c in merged:
        result.extend(_word_chunks(c))
    return [c for c in result if len(c.split()) >= MIN_CHUNK_WORDS]


def _word_chunks(text: str) -> list[str]:
    words = text.split()
    if len(words) <= MAX_CHUNK_WORDS:
        return [text]
    overlap = max(20, MAX_CHUNK_WORDS // 10)
    chunks, start = [], 0
    while start < len(words):
        end = min(start + MAX_CHUNK_WORDS, len(words))
        chunks.append(' '.join(words[start:end]))
        if end == len(words): break
        start = end - overlap
    return chunks


# ─── Archive reading ───────────────────────────────────────────────────────────

def read_docs_from_archive(archive_dir: Path):
    """Yield (doc_id, title, date, url, text) from all JSON entries in all zips."""
    import hashlib
    for zip_path in sorted(archive_dir.rglob('*.zip')):
        try:
            with zipfile.ZipFile(zip_path, 'r') as zf:
                for name in zf.namelist():
                    if not name.endswith('.json'):
                        continue
                    try:
                        data = json.loads(zf.read(name))
                        text = data.get('text', '')
                        url = data.get('url', '')
                        if not text or not url:
                            continue
                        doc_id = hashlib.sha1(url.encode()).hexdigest()[:16]
                        yield doc_id, data.get('title', ''), data.get('publish_date', ''), url, text
                    except Exception:
                        pass
        except Exception:
            pass


# ─── Main ─────────────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--archive-dir', default='/home/netshare/hdd/downloader/web_articles_archive')
    parser.add_argument('--vectorstore-dir', default='/home/netshare/hdd/downloader/vectorstore')
    parser.add_argument('--batch-size', type=int, default=32)
    parser.add_argument('--resume', action='store_true', help='Skip already-indexed doc_ids')
    args = parser.parse_args()

    archive_dir = Path(args.archive_dir)
    vs_dir = Path(args.vectorstore_dir)
    vs_dir.mkdir(parents=True, exist_ok=True)

    index_path = str(vs_dir / 'index.faiss')
    meta_path = str(vs_dir / 'meta.json')

    import faiss

    # Load or create index
    if os.path.exists(index_path) and args.resume:
        index = faiss.read_index(index_path)
        print(f'Resumed existing index with {index.ntotal} vectors')
    else:
        index = faiss.IndexFlatIP(DIM)
        print('Created new FAISS IndexFlatIP index')

    # Load existing metadata
    existing_ids: set[str] = set()
    meta: list[dict] = []
    if os.path.exists(meta_path) and args.resume:
        with open(meta_path) as f:
            meta = json.load(f)
        existing_ids = {m['doc_id'] for m in meta}
        print(f'Loaded {len(meta)} existing metadata entries, {len(existing_ids)} unique doc_ids')

    print('Loading embedding model...')
    sess, tokenizer = load_model()
    print('Model loaded.')

    total_docs = 0
    total_chunks = 0
    chunk_buf: list[str] = []
    meta_buf: list[dict] = []

    def flush(force=False):
        nonlocal total_chunks
        if not chunk_buf:
            return
        if not force and len(chunk_buf) < args.batch_size:
            return
        emb = embed(chunk_buf, sess, tokenizer, batch_size=args.batch_size)
        start = index.ntotal
        index.add(emb)
        for i, m in enumerate(meta_buf):
            m['id'] = start + i
        meta.extend(meta_buf)
        total_chunks += len(chunk_buf)
        chunk_buf.clear()
        meta_buf.clear()
        # Checkpoint
        faiss.write_index(index, index_path)
        with open(meta_path, 'w') as f:
            json.dump(meta, f, ensure_ascii=False)

    for doc_id, title, date, url, text in read_docs_from_archive(archive_dir):
        if doc_id in existing_ids:
            continue
        existing_ids.add(doc_id)
        total_docs += 1

        chunks = semantic_chunks(text)
        for i, chunk in enumerate(chunks):
            chunk_buf.append(chunk)
            meta_buf.append({
                'id': 0,
                'doc_id': doc_id,
                'doc_title': title,
                'doc_date': date,
                'doc_url': url,
                'chunk_index': i,
                'chunk_text': chunk[:500],
            })
        flush()

        if total_docs % 100 == 0:
            print(f'Processed {total_docs} docs, {total_chunks} chunks so far...')

    flush(force=True)

    print(f'\nDone. Docs: {total_docs}, Chunks: {total_chunks}, Index total: {index.ntotal}')


if __name__ == '__main__':
    main()
