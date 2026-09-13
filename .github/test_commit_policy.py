#!/usr/bin/env python3
"""Exercise signing hooks in isolated Git repositories with disposable test keys."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import check_commit_policy as policy
import merge_pr


class HookTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.keys = tempfile.TemporaryDirectory()
        cls.env = {key: value for key, value in os.environ.items()
                   if not key.startswith("GIT_") and key != "GNUPGHOME"}
        cls.env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull,
                       GNUPGHOME=cls.keys.name, GIT_TERMINAL_PROMPT="0")
        os.chmod(cls.keys.name, 0o700)
        cls.gpg = shutil.which("gpg")
        if not cls.gpg:
            raise RuntimeError("GnuPG is required for real signature regression tests")
        subprocess.run([cls.gpg, "--batch", "--pinentry-mode", "loopback", "--passphrase", "",
                        "--quick-generate-key", "Test Contributor <test@example.com>", "ed25519", "sign", "1d"],
                       env=cls.env, check=True, capture_output=True)

    @classmethod
    def tearDownClass(cls):
        if shutil.which("gpgconf"):
            subprocess.run(["gpgconf", "--kill", "gpg-agent"], env=cls.env, capture_output=True)
        cls.keys.cleanup()

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.repo = Path(self.temporary.name) / "repo"
        self.repo.mkdir()
        self.command("git", "init", "-b", "feature/test")
        for name, value in (("user.name", "Test Contributor"), ("user.email", "test@example.com"),
                            ("user.signingkey", "test@example.com"), ("gpg.program", self.gpg)):
            self.command("git", "config", name, value)
        (self.repo / "tools").mkdir()
        for name in ("check_commit_policy.py", "install_git_hooks.py"):
            shutil.copy2(ROOT / "tools" / name, self.repo / "tools" / name)
        shutil.copytree(ROOT / ".githooks", self.repo / ".githooks")
        self.command(sys.executable, "tools/install_git_hooks.py")

    def command(self, *args, input=None, ok=True):
        result = subprocess.run(args, cwd=self.repo, env=self.env, input=input,
                                text=True, capture_output=True)
        if ok:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        return result

    def commit(self, message="A test contribution", *args):
        self.command("git", "commit", "--allow-empty", "-m", message, *args)
        return self.command("git", "rev-parse", "HEAD").stdout.strip()

    def check(self, *args, ok=True):
        return self.command(sys.executable, "tools/check_commit_policy.py", *args, ok=ok)

    def test_hook_adds_one_signoff_and_creates_verified_signature(self):
        self.commit()
        self.check("--all")
        self.command("git", "commit", "--amend", "--no-edit", "--allow-empty")
        message = self.command("git", "show", "-s", "--format=%B", "HEAD").stdout
        self.assertEqual(message.count("Signed-off-by: Test Contributor <test@example.com>"), 1)
        self.check("--range", "HEAD")

    def test_hook_never_forges_another_authors_signoff(self):
        result = self.command("git", "commit", "--allow-empty", "--author", "Other Author <other@example.com>",
                              "-m", "An upstream contribution", ok=False)
        self.assertIn("Missing author trailer", result.stderr)
        message = (self.repo / ".git/COMMIT_EDITMSG").read_text()
        self.assertIn("Signed-off-by: Test Contributor <test@example.com>", message)
        self.assertNotIn("Signed-off-by: Other Author", message)

    def test_forged_body_line_does_not_count_as_a_trailer(self):
        self.commit("A test\n\nSigned-off-by: Test Contributor <test@example.com>\n\nMore body text")
        message = self.command("git", "show", "-s", "--format=%B", "HEAD").stdout
        self.assertEqual(message.count("Signed-off-by:"), 2)
        self.check("--all")

    def test_disabled_signing_configuration_is_rejected(self):
        self.command("git", "config", "commit.gpgsign", "false")
        result = self.command("git", "commit", "--allow-empty", "-m", "Unsigned", ok=False)
        self.assertIn("Enable commit.gpgsign", result.stderr)

    def test_unsigned_commit_and_tampered_signature_fail_audit(self):
        signed = self.commit()
        raw = self.command("git", "cat-file", "commit", signed).stdout
        bad = self.command("git", "hash-object", "-t", "commit", "-w", "--stdin",
                           input=raw.replace("A test contribution", "Altered contribution")).stdout.strip()
        self.check("--range", bad, ok=False)
        self.commit("Unsigned despite signed defaults", "--no-gpg-sign")
        self.check("--range", "HEAD^..HEAD", ok=False)

    def test_missing_signoff_and_bot_authors_are_not_exempt(self):
        self.commit()
        tree = self.command("git", "rev-parse", "HEAD^{tree}").stdout.strip()
        sha = self.command("git", "commit-tree", "-S", tree, input="Missing attestation\n").stdout.strip()
        self.assertIn("missing author Signed-off-by", self.check("--range", sha, ok=False).stderr)
        self.command("git", "config", "user.name", "dependabot[bot]")
        self.command("git", "config", "user.email", "49699333+dependabot[bot]@users.noreply.github.com")
        sha = self.command("git", "commit-tree", "-S", tree, input="Bot attestation missing\n").stdout.strip()
        self.check("--range", sha, ok=False)

    def test_actual_push_rejects_protected_updates_and_deletions(self):
        self.commit()
        bare = Path(self.temporary.name) / "remote.git"
        self.command("git", "init", "--bare", str(bare))
        self.command("git", "remote", "add", "origin", str(bare))
        self.command("git", "push", "origin", "HEAD:refs/heads/feature/test")
        for branch in ("main", "develop"):
            result = self.command("git", "push", "origin", f"HEAD:refs/heads/{branch}", ok=False)
            self.assertIn("Direct changes", result.stderr)
            # Seed the disposable remote without hooks to exercise deletion too.
            self.command("git", "-c", "core.hooksPath=/dev/null", "push", "origin", f"HEAD:refs/heads/{branch}")
            result = self.command("git", "push", "origin", f":refs/heads/{branch}", ok=False)
            self.assertIn("Direct changes", result.stderr)

    def test_tags_must_be_signed_and_cannot_be_updated_or_deleted(self):
        self.commit()
        bare = Path(self.temporary.name) / "remote.git"
        self.command("git", "init", "--bare", str(bare))
        self.command("git", "remote", "add", "origin", str(bare))
        self.command("git", "-c", "tag.gpgsign=false", "tag", "unsigned")
        self.command("git", "push", "origin", "refs/tags/unsigned", ok=False)
        self.command("git", "tag", "-s", "v0.1.0", "-m", "Test release")
        self.command("git", "push", "origin", "refs/tags/v0.1.0")
        self.command("git", "push", "origin", ":refs/tags/v0.1.0", ok=False)
        self.command("git", "tag", "-s", "-f", "v0.1.0", "-m", "Replacement")
        self.command("git", "push", "--force", "origin", "refs/tags/v0.1.0", ok=False)

    def test_installer_preserves_custom_hooks_and_global_configuration(self):
        self.command("git", "config", "--unset", "core.hooksPath")
        custom = self.repo / ".git/hooks/pre-commit"
        custom.write_text("#!/bin/sh\nexit 0\n")
        result = self.command(sys.executable, "tools/install_git_hooks.py", ok=False)
        self.assertIn("Preserving custom hooks", result.stderr)
        self.assertEqual(custom.read_text(), "#!/bin/sh\nexit 0\n")
        self.command("git", "config", "core.hooksPath", "../custom-hooks")
        result = self.command(sys.executable, "tools/install_git_hooks.py", ok=False)
        self.assertIn("Preserving custom core.hooksPath", result.stderr)
        self.assertEqual(self.command("git", "config", "--global", "--list").stdout, "")


class GitHubVerificationTests(unittest.TestCase):
    def test_merge_ready_states_keep_unsafe_states_closed(self):
        pr = {"state": "open", "draft": False, "base": {"ref": "develop"},
              "mergeable": True, "mergeable_state": "blocked"}
        for state in ("blocked", "clean"):
            pr["mergeable_state"] = state
            merge_pr.require_ready_pr(pr)
        for state in ("behind", "dirty", "unknown", "unstable", "draft", None):
            pr["mergeable_state"] = state
            with self.subTest(state=state), self.assertRaises(ValueError):
                merge_pr.require_ready_pr(pr)
        pr["mergeable_state"] = "blocked"
        for field, value in (("state", "closed"), ("draft", True), ("mergeable", False), ("mergeable", None)):
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                merge_pr.require_ready_pr({**pr, field: value})

    def test_blocked_pr_still_requires_checks_and_preserves_server_enforcement(self):
        head, base = "a" * 40, "b" * 40
        pr = {"state": "open", "draft": False, "base": {"ref": "develop", "sha": base},
              "head": {"sha": head}, "title": "A checked change", "mergeable": True,
              "mergeable_state": "blocked"}
        calls = []
        def api(path, payload=None):
            calls.append((path, payload))
            if path == "user":
                return {"id": 1, "login": "maintainer", "name": "Maintainer"}
            if path.endswith("/pulls/1"):
                return pr
            if payload:
                raise ValueError("GitHub rejected an unsatisfied repository rule")
            self.fail(f"Unexpected API request: {path}")
        with patch.object(sys, "argv", ["merge_pr.py", "1"]), patch.object(merge_pr, "api", api):
            with patch.object(merge_pr, "require_checks", side_effect=ValueError("DCO failed")):
                with self.assertRaisesRegex(ValueError, "DCO failed"):
                    merge_pr.main()
                self.assertFalse(any(payload for _, payload in calls))
            calls.clear()
            with patch.object(merge_pr, "require_checks") as checks:
                with self.assertRaisesRegex(ValueError, "GitHub rejected"):
                    merge_pr.main()
                repository = json.loads((ROOT / "repository-settings.json").read_text())["repository"]
                checks.assert_called_once_with(repository, 1, head)
                payloads = [payload for _, payload in calls if payload]
                self.assertEqual(len(payloads), 1)
                self.assertEqual(payloads[0]["sha"], head)
                self.assertEqual(payloads[0]["merge_method"], "merge")
                self.assertNotIn("admin", payloads[0])

    def test_merge_rechecks_base_and_head_after_checks(self):
        head, base = "a" * 40, "b" * 40
        pr = {"state": "open", "draft": False, "base": {"ref": "develop", "sha": base},
              "head": {"sha": head}, "title": "A checked change", "mergeable": True,
              "mergeable_state": "blocked"}
        actor = {"id": 1, "login": "maintainer", "name": "Maintainer"}
        variants = [{**pr, "head": {"sha": "c" * 40}},
                    {**pr, "base": {"ref": "develop", "sha": "c" * 40}},
                    {**pr, "base": {"ref": "main", "sha": base}}]
        for changed in variants:
            with self.subTest(changed=changed), patch.object(sys, "argv", ["merge_pr.py", "1"]), \
                    patch.object(merge_pr, "require_checks"), \
                    patch.object(merge_pr, "api", side_effect=[pr, actor, changed]) as api:
                with self.assertRaisesRegex(ValueError, "PR head or base changed"):
                    merge_pr.main()
                self.assertEqual(api.call_count, 3)

    def test_check_requires_verified_openpgp_and_exact_commit(self):
        sha = "a" * 40
        message = "Subject\n\nSigned-off-by: Author <a@example.com>\n"
        def git(*args, input=None):
            return "Author <a@example.com>\0" + message if args[0] == "show" else message.split("\n\n")[1]
        for verification, actual in (({"verified": False, "signature": "-----BEGIN PGP SIGNATURE-----"}, sha),
                                     ({"verified": True, "signature": "-----BEGIN SSH SIGNATURE-----"}, sha),
                                     ({"verified": True, "signature": "-----BEGIN PGP SIGNATURE-----"}, "b" * 40)):
            record = {"sha": actual, "commit": {"verification": verification}}
            with patch.object(policy, "git", git), patch.object(policy, "run", return_value=json.dumps(record)):
                with self.assertRaises(ValueError):
                    policy.check_commit(sha, "owner/repo")

    def test_merge_gate_rejects_wrong_app_stale_head_and_push_ci(self):
        head = "a" * 40
        checks = [{"id": index, "name": name, "app": {"id": app}, "head_sha": head,
                   "status": "completed", "conclusion": "success",
                   "details_url": "https://github.com/owner/repo/actions/runs/5/job/6"}
                  for index, (name, app) in enumerate((("CI", 15368), ("DCO", 1861)), 1)]
        workflow = {"event": "pull_request", "head_sha": head, "path": ".github/workflows/ci.yml",
                    "pull_requests": [{"number": 1}]}
        def api(path):
            return {"total_count": 2, "check_runs": checks} if "check-runs?" in path else workflow
        with patch.object(merge_pr, "api", api):
            merge_pr.require_checks("owner/repo", 1, head)
            checks[1]["app"]["id"] = 99
            with self.assertRaises(ValueError):
                merge_pr.require_checks("owner/repo", 1, head)
            checks[1]["app"]["id"] = 1861
            checks[1]["head_sha"] = "b" * 40
            with self.assertRaises(ValueError):
                merge_pr.require_checks("owner/repo", 1, head)
            checks[1]["head_sha"] = head
            workflow["event"] = "push"
            with self.assertRaises(ValueError):
                merge_pr.require_checks("owner/repo", 1, head)


if __name__ == "__main__":
    unittest.main()
