#!/usr/bin/env python3
"""Export a content-addressed protocol source bundle, without rebuilding the other repo."""
from pathlib import Path
import argparse
import hashlib
import json
import shutil

parser = argparse.ArgumentParser()
parser.add_argument("destination", type=Path)
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
source = root / "proto"
files = [*sorted(source.rglob("*.proto")), source / "semantic_fields.json"]
manifest = {"schema": "rx.protocol-source-bundle.v1", "files": {}}
for path in files:
    relative = path.relative_to(source)
    target = args.destination / relative
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(path.read_bytes())
    manifest["files"][str(relative)] = hashlib.sha256(target.read_bytes()).hexdigest()
for family in ("contracts", "cell_operations"):
    original = root / "spec" / family / "v1.0" / "protocol_manifest.json"
    target = args.destination / f"{family}_manifest.json"
    shutil.copyfile(original, target)
    manifest["files"][target.name] = hashlib.sha256(target.read_bytes()).hexdigest()
(args.destination / "bundle.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
print(f"Exported {len(manifest['files'])} pinned files to {args.destination}")
