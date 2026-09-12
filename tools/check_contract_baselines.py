#!/usr/bin/env python3
"""Verify vendored normative bytes. Does not infer implementation conformance."""
from pathlib import Path
import hashlib
import json

root = Path(__file__).resolve().parents[1] / "spec"
for family in ("contracts", "cell_operations"):
    folder = root / family / "v1.0"
    manifest = json.loads((folder / "protocol_manifest.json").read_text())
    for name, expected in manifest["normative_document_sha256"].items():
        actual = hashlib.sha256((folder / name).read_bytes()).hexdigest()
        if actual != expected:
            raise SystemExit(f"Normative content mismatch: {family}/{name}")
    print(f"{family}: {len(manifest['normative_document_sha256'])} normative files unchanged")
