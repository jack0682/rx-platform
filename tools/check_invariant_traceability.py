#!/usr/bin/env python3
"""Check local invariant IDs and named Rust test declarations, not semantic coverage."""
from collections import Counter
import json
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
MAP = ROOT / "docs/invariant-traceability.json"


def require(condition, message):
    if not condition:
        raise ValueError(message)


def definitions(spec):
    sources = (
        ("contracts/v1.0/01_responsibility_and_semantics.md", "## 8. Required invariants", r"^- (I\d{2}): ", 12),
        ("cell_operations/v1.0/05_validation_audit.md", "## 1. Invariants to verify", r"^\| (OI\d{2}) \| ", 18),
    )
    ids = []
    for relative, heading, pattern, count in sources:
        text = (spec / relative).read_text(encoding="utf-8")
        require(text.count(heading + "\n") == 1, f"ambiguous definition section: {relative}")
        section = text.split(heading + "\n", 1)[1].split("\n## ", 1)[0]
        found = re.findall(pattern, section, re.MULTILINE)
        require(len(found) == len(set(found)) == count, f"expected {count} unique definitions: {relative}")
        ids.extend(found)
    return set(ids)


def rust_code(text):
    """Mask comments and literals so a commented-out test does not count."""
    raw_literal = re.compile(r'(?:br|r)(#*)"')
    char_literal = re.compile(r"'(?:\\.|[^'\\\n])'")
    result = list(text)
    i = 0
    while i < len(text):
        start = i
        if text.startswith("//", i):
            end = text.find("\n", i)
            i = len(text) if end < 0 else end
        elif text.startswith("/*", i):
            i += 2
            depth = 1
            while i < len(text) and depth:
                if text.startswith("/*", i):
                    depth += 1
                    i += 2
                elif text.startswith("*/", i):
                    depth -= 1
                    i += 2
                else:
                    i += 1
            require(depth == 0, "unterminated Rust block comment")
        elif raw := raw_literal.match(text, i):
            closing = '"' + raw[1]
            end = text.find(closing, raw.end())
            require(end >= 0, "unterminated Rust raw string")
            i = end + len(closing)
        elif text[i] == '"':
            i += 1
            while i < len(text) and text[i] != '"':
                i += 2 if text[i] == "\\" else 1
            require(i < len(text), "unterminated Rust string")
            i += 1
        elif char := char_literal.match(text, i):
            i = char.end()
        else:
            i += 1
            continue
        result[start:i] = ["\n" if c == "\n" else " " for c in text[start:i]]
    return "".join(result)


def test_names(path):
    code = rust_code(path.read_text(encoding="utf-8"))
    return Counter(re.findall(
        r"#\[\s*(?:test|tokio\s*::\s*test)(?:\([^\]]*\))?\s*\]\s*"
        r"(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)\s*\(",
        code,
    ))


def main():
    spec = ROOT / "spec" if (ROOT / "spec").is_dir() else ROOT / "sdk/spec"
    expected = definitions(spec)
    data = json.loads(MAP.read_text(encoding="utf-8"))
    require(data["schema"] == "rx.invariant-traceability.v1", "unknown map schema")
    for field in ("scope", "verification_limit", "source_limit"):
        require(isinstance(data[field], str) and data[field].strip(), f"missing {field}")
    entries = data["invariants"]
    require(isinstance(entries, list), "invariants must be a list")
    ids = [entry["id"] for entry in entries]
    require(all(isinstance(i, str) for i in ids), "IDs must be strings")
    duplicates = sorted(i for i, n in Counter(ids).items() if n > 1)
    missing = sorted(expected - set(ids))
    unknown = sorted(set(ids) - expected)
    require(not (duplicates or missing or unknown), f"ID mismatch: missing={missing}, unknown={unknown}, duplicate={duplicates}")
    counts = Counter()
    cache = {}
    links = 0
    for entry in entries:
        status = entry["status"]
        require(status in {"declared", "uncovered"}, f"{entry['id']}: invalid status")
        require(isinstance(entry["reason"], str) and entry["reason"].strip(), f"{entry['id']}: missing reason")
        tests = entry["tests"]
        require(isinstance(tests, list), f"{entry['id']}: tests must be a list")
        require(bool(tests) == (status == "declared"), f"{entry['id']}: status/tests disagree")
        seen = set()
        for test in tests:
            relative, name = test["file"], test["name"]
            require(isinstance(relative, str) and isinstance(name, str), "test file/name must be strings")
            path = (ROOT / relative).resolve()
            require(not Path(relative).is_absolute() and path.is_relative_to(ROOT) and path.suffix == ".rs" and path.is_file(), f"invalid local Rust test path: {relative}")
            require((relative, name) not in seen, f"{entry['id']}: duplicate test reference")
            seen.add((relative, name))
            require(isinstance(test["reason"], str) and test["reason"].strip(), f"{entry['id']}: missing test rationale")
            if path not in cache:
                cache[path] = test_names(path)
            require(cache[path][name] == 1, f"{entry['id']}: expected one test declaration {relative}::{name}, found {cache[path][name]}")
            links += 1
        counts[status] += 1
    summary = {"total": len(expected), "declared": counts["declared"], "uncovered": counts["uncovered"]}
    require(isinstance(data["summary"], dict) and all(type(data["summary"].get(k)) is int for k in summary), "summary counts must be integers")
    require(data["summary"] == summary, f"summary mismatch: expected {summary}")
    print(f"Invariant traceability: I=12 OI=18; declared={summary['declared']} uncovered={summary['uncovered']}; test links={links}. Semantic coverage not checked.")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, TypeError, OSError) as error:
        print(f"invariant traceability: {error}", file=sys.stderr)
        raise SystemExit(1)
