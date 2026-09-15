#!/usr/bin/env python3
"""Regression cases for the branch gate and repository artifact checks."""

import hashlib
import io
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from contextlib import redirect_stdout
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "check_repository", Path(__file__).resolve().parents[1] / "tools/check_repository.py"
)
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)


CONFIG_SPEC = importlib.util.spec_from_file_location(
    "configure_github", Path(__file__).resolve().parents[1] / "tools/configure_github.py"
)
CONFIGURE = importlib.util.module_from_spec(CONFIG_SPEC)
CONFIG_SPEC.loader.exec_module(CONFIGURE)


class GovernanceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        root = Path(__file__).resolve().parents[1]
        cls.config = json.loads((root / "repository-settings.json").read_text())
        cls.rulesets = {rule["name"]: rule for rule in cls.config["rulesets"]}

    def test_configuration_audit_rejects_bypass_and_check_provider_drift(self):
        expected = {"bypass_actors": [], "checks": [
            {"context": "CI", "integration_id": 15368},
            {"context": "DCO", "integration_id": 1861},
        ]}
        self.assertTrue(CONFIGURE.contains(expected, expected))
        self.assertFalse(CONFIGURE.contains({**expected, "bypass_actors": [{"actor_id": 5}]}, expected))
        self.assertFalse(CONFIGURE.contains({**expected, "checks": [
            {"context": "CI", "integration_id": 1},
            {"context": "DCO", "integration_id": 1861},
        ]}, expected))
        self.assertFalse(CONFIGURE.contains([{"context": "CI"}, {"context": "DCO"}],
                                            [{"context": "CI"}, {"context": "CI"}]))

    def test_update_rule_omits_only_its_false_default(self):
        expected = {"type": "update", "parameters": {"update_allows_fetch_and_merge": False}}
        self.assertTrue(CONFIGURE.contains({"type": "update"}, expected))
        self.assertFalse(CONFIGURE.contains(
            {"type": "update", "parameters": {"update_allows_fetch_and_merge": True}}, expected))

    def test_pr_only_exception_cannot_bypass_quality_checks(self):
        quality = self.rulesets["RX protected branches"]
        self.assertEqual(quality["bypass_actors"], [])
        rules = {rule["type"]: rule for rule in quality["rules"]}
        self.assertTrue({"deletion", "non_fast_forward", "required_signatures", "pull_request"} <= rules.keys())
        checks = rules["required_status_checks"]["parameters"]
        self.assertTrue(checks["strict_required_status_checks_policy"])
        self.assertEqual(checks["required_status_checks"], [
            {"context": "CI", "integration_id": 15368},
            {"context": "DCO", "integration_id": 1861},
        ])
        self.assertEqual(rules["pull_request"]["parameters"]["allowed_merge_methods"], ["merge"])
        update = self.rulesets["RX PR-only branch updates"]
        self.assertEqual(update["bypass_actors"], [
            {"actor_id": 5, "actor_type": "RepositoryRole", "bypass_mode": "pull_request"}
        ])
        self.assertEqual(update["rules"], [
            {"type": "update", "parameters": {"update_allows_fetch_and_merge": False}}
        ])
        self.assertEqual(update["conditions"], quality["conditions"])

    def test_every_branch_requires_verified_signatures_without_bypass(self):
        contributions = self.rulesets["RX signed contributions"]
        self.assertEqual(contributions["bypass_actors"], [])
        self.assertEqual(contributions["conditions"]["ref_name"], {"include": ["~ALL"], "exclude": []})
        self.assertEqual(contributions["rules"], [{"type": "required_signatures"}])

    def test_unknown_branch_creation_has_no_bypass(self):
        names = self.rulesets["RX GitFlow branch names"]
        self.assertEqual(names["bypass_actors"], [])
        self.assertEqual(names["rules"], [{"type": "creation"}])
        self.assertEqual(names["conditions"]["ref_name"], {
            "include": ["~ALL"],
            "exclude": ["refs/heads/main", "refs/heads/develop"] + [
                "refs/heads/" + prefix + "/**/*" for prefix in
                ("feature", "fix", "docs", "chore", "codex", "release", "hotfix")],
        })

    def test_tags_and_merge_settings_do_not_offer_unsigned_shortcuts(self):
        tags = self.rulesets["RX immutable release tags"]
        self.assertEqual(tags["bypass_actors"], [])
        self.assertEqual(tags["conditions"]["ref_name"], {"include": ["~ALL"], "exclude": []})
        self.assertEqual({rule["type"] for rule in tags["rules"]}, {"deletion", "update"})
        settings = self.config["settings"]
        self.assertTrue(settings["allow_merge_commit"])
        self.assertTrue(settings["web_commit_signoff_required"])
        for field in ("allow_squash_merge", "allow_rebase_merge", "allow_auto_merge", "allow_update_branch"):
            self.assertFalse(settings[field])


class SecurityAutomationTests(unittest.TestCase):
    def setUp(self):
        root = Path(__file__).resolve().parents[1]
        self.config = json.loads((root / "repository-settings.json").read_text())
        self.prefix = "repos/" + self.config["repository"]
        self.enabled = True
        self.alerts = True
        self.ignore_change = False
        self.calls = []

    def api(self, method, path, payload=None):
        self.calls.append((method, path))
        if path == self.prefix + "/automated-security-fixes":
            if method == "GET":
                return {"enabled": self.enabled, "paused": False}
            if not self.ignore_change:
                self.enabled = method == "PUT"
            return None
        if path == self.prefix + "/vulnerability-alerts":
            if method == "GET" and not self.alerts:
                raise RuntimeError("HTTP 404: vulnerability alerts disabled")
            if method != "GET":
                self.alerts = method == "PUT"
            return None
        if method != "GET":
            return {"id": int(path.rsplit("/", 1)[1])} if "/rulesets/" in path else None
        if path == self.prefix:
            return {**self.config["settings"], "security_and_analysis": self.config["security"]}
        if "/branches/" in path:
            return {}
        if path.endswith("/rulesets?per_page=100"):
            return [{"name": rule["name"], "id": index + 1}
                    for index, rule in enumerate(self.config["rulesets"])]
        if "/rulesets/" in path:
            return self.config["rulesets"][int(path.rsplit("/", 1)[1]) - 1]
        for suffix, key in (("/topics", "topics"), ("/actions/permissions", "actions"),
                            ("/actions/permissions/workflow", "workflow_permissions")):
            if path == self.prefix + suffix:
                value = self.config[key]
                return {"names": value} if key == "topics" else value
        self.fail(f"Unexpected request: {method} {path}")

    def invoke(self, apply=False):
        self.calls.clear()
        output = io.StringIO()
        argv = ["configure_github.py"] + (["--apply"] if apply else [])
        with patch.object(CONFIGURE, "api", self.api), patch("sys.argv", argv), redirect_stdout(output):
            result = CONFIGURE.main()
        return result, output.getvalue()

    def test_apply_and_reapply_keep_fixes_disabled_and_alerts_enabled(self):
        self.assertIs(self.config["dependency_updates"]["automated_security_fixes"], False)
        for _ in range(2):
            self.assertEqual(self.invoke(apply=True)[0], 0)
            self.assertFalse(self.enabled)
            self.assertTrue(self.alerts)
            self.assertNotIn(("PUT", self.prefix + "/automated-security-fixes"), self.calls)
            self.assertIn(("GET", self.prefix + "/automated-security-fixes"), self.calls)
            self.assertIn(("GET", self.prefix + "/vulnerability-alerts"), self.calls)

    def test_read_only_audit_detects_reactivation_without_changing_it(self):
        result, output = self.invoke()
        self.assertEqual(result, 1)
        self.assertIn("automatic security fixes", output)
        self.assertTrue(self.enabled)
        self.assertTrue(all(method == "GET" for method, _ in self.calls))
        self.enabled = False
        self.assertEqual(self.invoke()[0], 0)
        self.alerts = False
        with self.assertRaisesRegex(RuntimeError, "alerts disabled"):
            self.invoke()

    def test_apply_detects_a_security_change_that_did_not_take_effect(self):
        self.ignore_change = True
        result, output = self.invoke(apply=True)
        self.assertEqual(result, 1)
        self.assertIn("automatic security fixes", output)
        self.assertTrue(self.enabled)


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
        for source, target in (("develop", "main"), ("release/0.2", "main"), ("hotfix/issue", "main"), ("main", "develop"), ("release/0.2", "develop"), ("hotfix/issue", "develop")):
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

    def test_shell_syntax_is_only_data(self):
        self.assertIsNone(self.route("feature/$(exit 99)", "develop"))


class ArtifactTests(unittest.TestCase):
    def test_ai_instructions_are_excluded_without_rejecting_test_harnesses(self):
        for name in ("AGENTS.md", "claude.md", "nested/CLAUDE.local.md",
                     ".claude/settings.json", ".github/agents/reviewer.md",
                     "skills/reviewer/SKILL.md", ".cursor/rules/project.mdc",
                     ".claude", ".codex", ".cursor", ".agents", ".continue"):
            with self.subTest(name=name):
                self.assertTrue(CHECK.is_ai_artifact(Path(name)))
        for name in ("runtime/rx-host/tests/support/crash_harness.rs",
                     ".github/workflows/ci.yml", "docs/agent_architecture.md"):
            self.assertFalse(CHECK.is_ai_artifact(Path(name)))

    def test_english_policy_rejects_residual_text_but_preserves_unicode_test_coverage(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / "source.rs"
            policy = {"schema": "rx.repository-content-policy.v1", "language": "en",
                      "exclude_ai_artifacts": True}
            path.write_text("English comment\n" + chr(0xac00))
            self.assertIn("source.rs:2", CHECK.content_policy_errors(root, [path], policy)[0])
            policy["language"] = "mixed"
            self.assertEqual(CHECK.content_policy_errors(root, [path], policy), [])
            policy["language"] = "en"
            path.write_text('let unicode_fixture = "\\u{ac00}";')
            self.assertEqual(CHECK.content_policy_errors(root, [path], policy), [])

    def test_content_policy_checks_ai_files_even_in_mixed_language_repositories(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / "AGENTS.md"
            path.write_text("Local assistant instructions")
            policy = {"schema": "rx.repository-content-policy.v1", "language": "mixed",
                      "exclude_ai_artifacts": True}
            self.assertIn("must remain local", CHECK.content_policy_errors(root, [path], policy)[0])

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

    def test_ai_symlink_and_non_english_filename_cannot_escape_the_policy(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            assistant = root / "CLAUDE.md"
            assistant.symlink_to("missing-local-instructions")
            settings = root / ".claude"
            settings.symlink_to(root, target_is_directory=True)
            document = root / (chr(0xac00) + ".md")
            document.write_text("English content")
            policy = {"schema": "rx.repository-content-policy.v1", "language": "en",
                      "exclude_ai_artifacts": True}
            errors = CHECK.content_policy_errors(root, [assistant, settings, document], policy)
            self.assertEqual(len(errors), 3)
            self.assertTrue(any("must remain local" in item for item in errors))
            self.assertTrue(any("English filename" in item for item in errors))

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
