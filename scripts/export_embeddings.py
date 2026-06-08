#!/usr/bin/env python3
"""
export_embeddings.py

Exports the sentence-embedding model used by `mod_vectorstore` to ONNX so the Rust
pipeline can load it. The Rust embedder (src/plugins/mod_vectorstore.rs) only needs two
files in the target directory:

    model.onnx     - the ONNX graph (feature-extraction / last_hidden_state)
    vocab.txt      - the WordPiece vocabulary for the bundled BertTokenizer

It auto-detects that directory at `models/sentence-transformers_all-mpnet-base-v2`
(768-dim, 512 max tokens), matching find_model_dir() / EMBED_DIM / MAX_SEQ_LEN in the code.

Why the Python API instead of `optimum-cli export onnx`:
  Recent optimum versions misdetect "sentence-transformers/..." as a sentence_transformers
  model and crash with `property 'config' ... has no setter`. ORTModelForFeatureExtraction
  loads through transformers' AutoModel, which avoids that branch. (The CLI equivalent that
  also works is: `optimum-cli export onnx --library transformers ...`.)

Usage:
  python3 scripts/export_embeddings.py                 # export if missing
  python3 scripts/export_embeddings.py --force         # re-export even if present
  python3 scripts/export_embeddings.py --model <hf_id> --out <dir>

Requirements (into the same venv):
  pip install -U "optimum[onnxruntime]" onnx onnxruntime transformers
"""

import argparse
import os
import shutil
import sys

DEFAULT_MODEL = "sentence-transformers/all-mpnet-base-v2"
DEFAULT_OUT = "models/sentence-transformers_all-mpnet-base-v2"
REQUIRED_FILES = ("model.onnx", "vocab.txt")


def already_exported(out_dir: str) -> bool:
    return all(os.path.isfile(os.path.join(out_dir, f)) for f in REQUIRED_FILES)


def ensure_model_onnx(out_dir: str) -> None:
    """optimum sometimes writes the graph under a different name or subdir; normalise it
    to `<out_dir>/model.onnx` which is the exact filename the Rust loader looks for."""
    target = os.path.join(out_dir, "model.onnx")
    if os.path.isfile(target):
        return
    candidates = []
    for root, _dirs, files in os.walk(out_dir):
        for f in files:
            if f.endswith(".onnx"):
                candidates.append(os.path.join(root, f))
    if not candidates:
        raise FileNotFoundError(f"no .onnx file produced under {out_dir}")
    # Prefer the largest .onnx (the model graph, not an external-data shard).
    src = max(candidates, key=lambda p: os.path.getsize(p))
    print(f"  normalising {src} -> {target}")
    shutil.copyfile(src, target)


def export(model_id: str, out_dir: str) -> None:
    # Imported lazily so --help works without the heavy deps installed.
    from optimum.onnxruntime import ORTModelForFeatureExtraction
    from transformers import AutoTokenizer

    os.makedirs(out_dir, exist_ok=True)
    print(f"Exporting '{model_id}' -> '{out_dir}' (ONNX, feature-extraction)...")
    ORTModelForFeatureExtraction.from_pretrained(model_id, export=True).save_pretrained(out_dir)
    AutoTokenizer.from_pretrained(model_id).save_pretrained(out_dir)
    ensure_model_onnx(out_dir)


def main() -> int:
    ap = argparse.ArgumentParser(description="Export the mod_vectorstore embedding model to ONNX.")
    ap.add_argument("--model", default=DEFAULT_MODEL, help=f"HF model id (default: {DEFAULT_MODEL})")
    ap.add_argument("--out", default=DEFAULT_OUT, help=f"output dir (default: {DEFAULT_OUT})")
    ap.add_argument("--force", action="store_true", help="re-export even if model.onnx + vocab.txt exist")
    args = ap.parse_args()

    if already_exported(args.out) and not args.force:
        print(f"Already exported: {args.out}/{{{', '.join(REQUIRED_FILES)}}} present. "
              f"Use --force to re-export.")
        return 0

    try:
        export(args.model, args.out)
    except ImportError as e:
        print(f"ERROR: missing dependency ({e}).\n"
              f"  pip install -U \"optimum[onnxruntime]\" onnx onnxruntime transformers",
              file=sys.stderr)
        return 2
    except Exception as e:  # noqa: BLE001 - surface the real cause to the user
        print(f"ERROR: export failed: {e}", file=sys.stderr)
        return 1

    missing = [f for f in REQUIRED_FILES if not os.path.isfile(os.path.join(args.out, f))]
    if missing:
        print(f"ERROR: export finished but these required files are missing: {missing}", file=sys.stderr)
        return 1

    print("Done. Verified files:")
    for f in REQUIRED_FILES:
        p = os.path.join(args.out, f)
        print(f"  {p}  ({os.path.getsize(p):,} bytes)")
    print("mod_vectorstore will load this embedder on the next pipeline run.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
