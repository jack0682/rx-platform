#!/usr/bin/env python3
"""Install repository-local signing defaults and hooks, preserving existing custom hooks."""
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def git(*args, optional=False):
    result = subprocess.run(["git", "-C", str(ROOT), *args], text=True, capture_output=True)
    if result.returncode and not (optional and result.returncode == 1):
        raise ValueError(result.stderr.strip() or "Git configuration failed")
    return result.stdout.strip()


def main():
    hooks = ROOT / ".githooks"
    configured = git("config", "--get", "core.hooksPath", optional=True)
    if configured and (ROOT / configured).resolve() != hooks.resolve():
        raise ValueError(f"Preserving custom core.hooksPath={configured}; integrate hooks manually")
    if not configured:
        previous = Path(git("rev-parse", "--git-path", "hooks"))
        if not previous.is_absolute():
            previous = ROOT / previous
        custom = [path.name for path in previous.glob("*")
                  if path.is_file() and not path.name.endswith(".sample")]
        if custom:
            raise ValueError(f"Preserving custom hooks: {', '.join(sorted(custom))}; integrate manually")
    key = git("config", "--get", "user.signingkey", optional=True)
    if not key:
        raise ValueError("Configure user.signingkey with your existing GitHub-registered OpenPGP key first")
    if git("config", "--get", "gpg.format", optional=True) not in ("", "openpgp"):
        raise ValueError("Existing signing format is not OpenPGP; select your OpenPGP key first")
    for name in ("user.name", "user.email"):
        if not git("config", "--get", name, optional=True):
            raise ValueError(f"Configure {name} before installing DCO hooks")
    for path in hooks.iterdir():
        if path.is_file():
            path.chmod(path.stat().st_mode | 0o111)
    for name, value in (("core.hooksPath", ".githooks"), ("gpg.format", "openpgp"),
                        ("user.signingkey", key), ("commit.gpgsign", "true"), ("tag.gpgsign", "true")):
        git("config", "--local", name, value)
    print("Installed local DCO hooks, OpenPGP commit/tag signing and main/develop push guard")
    print("Commits attest your DCO contribution automatically; GitHub remains the enforcement boundary")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, OSError) as exc:
        print(f"hook installation: {exc}", file=sys.stderr)
        raise SystemExit(1)
