#!/usr/bin/env python3
"""Prove release-rotation defenses are load-bearing in private source copies."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
CARGO = ROOT / "tools/cargo"


def replace_once(source, before, after):
    if source.count(before) != 1:
        raise ValueError("mutation anchor count differs: " + before[:80])
    return source.replace(before, after)


def regex_once(source, pattern, replacement):
    result, count = re.subn(pattern, replacement, source, count=1, flags=re.S)
    if count != 1:
        raise ValueError("mutation regex anchor differs: " + pattern[:80])
    return result


OLD_KEY = "rx/development-release-2026-09"
OLD_PUBLIC = """[
    0xe6, 0x2d, 0xa4, 0xe2, 0x94, 0x36, 0xcd, 0xcd, 0xe5, 0xb5, 0x7a, 0x2f, 0x8e, 0xe4, 0x41, 0x70,
    0xee, 0xe3, 0xb1, 0x0f, 0xbc, 0x0c, 0xea, 0x31, 0x1d, 0x04, 0x14, 0x34, 0x1b, 0xee, 0x1e, 0x35,
]"""


def mutate(name, release_source, state_source, root_source):
    if name == "root-revert":
        root_source = replace_once(
            root_source,
            'pub const KEY_ID: &str = "rx/development-release-2026-09-r2";',
            f'pub const KEY_ID: &str = "{OLD_KEY}";',
        )
        root_source = regex_once(
            root_source,
            r"pub const PUBLIC_KEY: \[u8; 32\] = \[.*?\n\];",
            "pub const PUBLIC_KEY: [u8; 32] = " + OLD_PUBLIC + ";",
        )
    elif name == "dual-root":
        release_source = replace_once(
            release_source,
            """    if envelope.key.as_str() != root::KEY_ID {
        return Err(Error::UnknownKey);
    }
    verify_detached_message(
        &signing_message(domain, value, &envelope.key)?,
        &envelope.signature,
        &root::PUBLIC_KEY,
    )
    .map_err(|_| Error::Signature)""",
            """    let public_key = if envelope.key.as_str() == root::KEY_ID {
        root::PUBLIC_KEY
    } else if envelope.key.as_str() == root::PREVIOUS_KEY_ID {
        """
            + OLD_PUBLIC
            + """
    } else {
        return Err(Error::UnknownKey);
    };
    verify_detached_message(
        &signing_message(domain, value, &envelope.key)?,
        &envelope.signature,
        &public_key,
    )
    .map_err(|_| Error::Signature)""",
        )
    elif name == "key-id-gate":
        release_source = replace_once(
            release_source,
            "if envelope.key.as_str() != root::KEY_ID {",
            "if false && envelope.key.as_str() != root::KEY_ID {",
        )
    elif name == "signature-verification":
        release_source = regex_once(
            release_source,
            r"    verify_detached_message\(\n        &signing_message\(domain, value, &envelope.key\)\?,\n        &envelope.signature,\n        &root::PUBLIC_KEY,\n    \)\n    \.map_err\(\|_\| Error::Signature\)",
            "    Ok(())",
        )
    elif name == "domain-separation":
        release_source = replace_once(
            release_source,
            'signature("RX-RELEASE-v1", &value.manifest, value.signature.as_ref())?;',
            'signature("RX-RELEASE-REVOCATIONS-v1", &value.manifest, value.signature.as_ref())?;',
        )
    elif name == "positive-version":
        release_source = replace_once(
            release_source, "\n        || value.manifest.version.0 == 0", ""
        )
    elif name == "release-floor":
        state_source = regex_once(
            state_source,
            r"        if current\.version\(\) < prior\.manifest\.version\n            \|\| \(current\.version\(\) == prior\.manifest\.version && current\.digest != digest\)\n        \{",
            "        if false {",
        )
    elif name == "revocation-floor":
        state_source = regex_once(
            state_source,
            r"        if b\.version < a\.version\n            \|\| !a\.revoked\.is_subset\(&b\.revoked\)\n            \|\| \(b\.version == a\.version\n                && canonical::bytes\(a\)\.map_err\(state_error\)\?\n                    != canonical::bytes\(b\)\.map_err\(state_error\)\?\)\n        \{",
            "        if false {",
        )
    elif name == "revoked-membership":
        release_source = replace_once(
            release_source,
            "if revoked.revocations.revoked.contains(&digest) {",
            "if false && revoked.revocations.revoked.contains(&digest) {",
        )
    elif name == "inventory-digest":
        release_source = replace_once(
            release_source,
            "if content_digest(inventory_bytes) != signed.manifest.inventory_sha256 {",
            "if false && content_digest(inventory_bytes) != signed.manifest.inventory_sha256 {",
        )
    elif name == "file-digest":
        release_source = replace_once(
            release_source,
            "if content_digest(&acquire(path, false)?) != *expected {",
            "if false && content_digest(&acquire(path, false)?) != *expected {",
        )
    else:
        raise ValueError("unknown mutation")
    return release_source, state_source, root_source


CASES = {
    "root-revert": (
        "rotation",
        "rotated_root_refuses_old_foreign_wrong_id_and_domain_confusion",
    ),
    "dual-root": (
        "rotation",
        "rotated_root_refuses_old_foreign_wrong_id_and_domain_confusion",
    ),
    "key-id-gate": (
        "rotation",
        "rotated_root_refuses_old_foreign_wrong_id_and_domain_confusion",
    ),
    "signature-verification": (
        "rotation",
        "rotated_root_refuses_old_foreign_wrong_id_and_domain_confusion",
    ),
    "domain-separation": (
        "rotation",
        "rotated_root_refuses_old_foreign_wrong_id_and_domain_confusion",
    ),
    "positive-version": (
        "rotation",
        "zero_is_malformed_and_maximum_is_valid_without_advancing_a_floor",
    ),
    "release-floor": (
        "release",
        "durable_floor_rejects_restart_downgrade_and_same_version_equivocation",
    ),
    "revocation-floor": (
        "release",
        "revocation_survives_denied_candidate_restart_and_replayed_or_shrunk_lists",
    ),
    "revoked-membership": (
        "release",
        "revocation_survives_denied_candidate_restart_and_replayed_or_shrunk_lists",
    ),
    "inventory-digest": (
        "release",
        "compiled_root_distinguishes_unsigned_signature_key_and_content_refusals",
    ),
    "file-digest": (
        "release",
        "compiled_root_distinguishes_unsigned_signature_key_and_content_refusals",
    ),
}


def copy_source(destination):
    shutil.copytree(
        ROOT,
        destination,
        ignore=shutil.ignore_patterns(".git", "target", "node_modules", "__pycache__"),
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", required=True, type=Path)
    args = parser.parse_args()
    evidence = args.evidence.resolve()
    evidence.mkdir(parents=True, exist_ok=False)
    product_files = [
        ROOT / "crates/rx-package/src/release.rs",
        ROOT / "crates/rx-package/src/release/state.rs",
        ROOT / "crates/rx-package/src/release/root.rs",
    ]
    before = {
        str(path): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in product_files
    }
    summary = []
    for name, (test_target, test_name) in CASES.items():
        with tempfile.TemporaryDirectory(prefix="rx-release-mutation-") as temporary:
            temporary = Path(temporary)
            source = temporary / "source"
            target = temporary / "target"
            copy_source(source)
            release_path = source / "crates/rx-package/src/release.rs"
            state_path = source / "crates/rx-package/src/release/state.rs"
            root_path = source / "crates/rx-package/src/release/root.rs"
            release_source, state_source, root_source = mutate(
                name,
                release_path.read_text(),
                state_path.read_text(),
                root_path.read_text(),
            )
            release_path.write_text(release_source)
            state_path.write_text(state_source)
            root_path.write_text(root_source)
            command = [
                str(CARGO),
                "test",
                "--manifest-path",
                str(source / "Cargo.toml"),
                "-p",
                "rx-package",
                "--test",
                test_target,
                "--locked",
                "--target-dir",
                str(target),
                test_name,
                "--",
                "--exact",
                "--nocapture",
            ]
            result = subprocess.run(command, capture_output=True, text=True)
            output = result.stdout + result.stderr
            if result.returncode == 0 or "test result: FAILED" not in output:
                raise RuntimeError(
                    f"mutation did not produce a compiled red control: {name}\n{output[-2000:]}"
                )
            row = {
                "case": name,
                "defense_load_bearing": True,
                "mutant_compiled": True,
                "expected_test_became_red": test_name,
            }
            (evidence / f"{name}.log").write_text(output)
            summary.append(row)
            print(json.dumps(row), flush=True)
    after = {
        str(path): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in product_files
    }
    if before != after:
        raise RuntimeError("product source changed during private mutations")
    (evidence / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")


if __name__ == "__main__":
    main()
