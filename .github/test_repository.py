#!/usr/bin/env python3
"""Regression cases for the branch gate and repository artifact checks."""

import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location(
    "check_repository", Path(__file__).resolve().parents[1] / "tools/check_repository.py"
)
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)


class BranchPolicyTests(unittest.TestCase):
    def route(self, source, target, fork=False):
        return CHECK.branch_error({"pull_request": {
            "head": {"ref": source, "repo": {"full_name": "contributor/fork" if fork else "owner/repo"}},
            "base": {"ref": target, "repo": {"full_name": "owner/repo"}},
        }})

    def test_allowed_integration_routes(self):
        for source in ("feature/dispatch", "fix/state", "docs/contracts", "chore/ci", "codex/work"):
            for fork in (False, True):
                with self.subTest(source=source, fork=fork):
                    self.assertIsNone(self.route(source, "develop", fork))

    def test_release_and_backmerge_routes(self):
        for source, target in (("develop", "main"), ("release/0.2", "main"), ("hotfix/issue", "main"), ("main", "develop"), ("release/0.2", "develop"), ("hotfix/issue", "develop"), ("dependabot/npm/security", "main")):
            with self.subTest(source=source, target=target):
                self.assertIsNone(self.route(source, target))

    def test_invalid_routes_are_rejected(self):
        for source, target in (("feature/work", "main"), ("codex/work", "main"), ("main", "main"), ("develop", "develop"), ("feature/", "develop"), ("random", "develop"), ("feature/work", "other")):
            with self.subTest(source=source, target=target):
                self.assertIsNotNone(self.route(source, target))

    def test_fork_cannot_impersonate_release_branch(self):
        for source, target in (("main", "develop"), ("develop", "main"), ("release/0.2", "main"), ("hotfix/issue", "main")):
            with self.subTest(source=source):
                self.assertIsNotNone(self.route(source, target, fork=True))

    def test_missing_payload_fails_closed(self):
        self.assertIsNotNone(CHECK.branch_error({}))
        self.assertIsNotNone(CHECK.branch_error({"pull_request": {}}))

    def test_dependabot_exception_requires_repository_owned_branch(self):
        for target in ("main", "develop"):
            self.assertIsNone(self.route("dependabot/cargo/update", target))
            self.assertIsNotNone(self.route("dependabot/cargo/update", target, fork=True))

    def test_shell_syntax_is_only_data(self):
        self.assertIsNone(self.route("feature/$(exit 99)", "develop"))


class ArtifactTests(unittest.TestCase):
    def test_local_links_and_code_examples(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            (root / "exists.md").write_text("ok")
            source = root / "README.md"
            source.write_text("[ok](exists.md#section)\n[bad](missing.md)\n"
                              "[remote](https://example.invalid/page)\n"
                              "`[example](absent.md)`\n```md\n[example](absent.md)\n```\n"
                              "[^note]: prose [ok](exists.md)\n[ref]: exists.md\n")
            errors, siblings = CHECK.local_link_errors(root, source)
            self.assertEqual(len(errors), 1)
            self.assertIn("missing.md", errors[0])
            self.assertEqual(siblings, 0)

    def test_machine_specific_link_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            source = root / "README.md"
            source.write_text("[local](/Users/example/private/notes.md)\n[file](file:///tmp/private.md)\n")
            errors, _ = CHECK.local_link_errors(root, source)
            self.assertEqual(len(errors), 2)
            self.assertIn("machine-specific", errors[0])

    def test_invalid_json_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / "invalid.json"
            source.write_text('{"missing":}')
            with self.assertRaises(ValueError):
                CHECK.read_json(source)

    def test_normative_content_and_manifest_mutations_fail(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            hashes = {}
            for index in range(4):
                name = f"{index}.md"
                (root / name).write_text("contract\n")
                hashes[name] = hashlib.sha256((root / name).read_bytes()).hexdigest()
            manifest = root / "protocol_manifest.json"
            manifest.write_text(json.dumps({"normative_document_sha256": hashes}))
            (root / "manifest_integrity.json").write_text(json.dumps({
                "schema_hash_sha256": hashlib.sha256(manifest.read_bytes()).hexdigest()
            }))
            self.assertEqual(CHECK.contract_errors(root), [])
            (root / "0.md").write_text("changed")
            self.assertIn("normative hash differs", CHECK.contract_errors(root)[0])
            manifest.write_text(manifest.read_text() + "\n")
            self.assertTrue(any("manifest integrity" in e for e in CHECK.contract_errors(root)))

    def test_sdk_unrecorded_file_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            sdk = root / "sdk"
            sdk.mkdir()
            lock = sdk / "source-lock.json"
            lock.write_text(json.dumps({"schema": "rx.host-sdk.v1", "files": {}}))
            self.assertEqual(CHECK.sdk_errors(root, [lock]), [])
            extra = sdk / "unexpected.rs"
            extra.write_text("changed")
            self.assertTrue(CHECK.sdk_errors(root, [lock, extra]))


if __name__ == "__main__":
    unittest.main()
