"""Direct terminal-mTLS acceptance client. Request evidence excludes login secrets/cookies."""
from __future__ import annotations
import hashlib
import http.cookiejar
import json
import ssl
import urllib.error
import urllib.parse
import urllib.request
import uuid
from pathlib import Path


def encoded(value: object) -> bytes:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()


def publish_new(path: Path, value: object) -> None:
    data = encoded(value)
    if path.exists():
        if path.read_bytes() != data:
            raise ValueError(f"different evidence already exists: {path.name}")
        return
    with path.open("xb") as target:
        target.write(data)
        target.flush()
        import os
        os.fsync(target.fileno())


class Rejected(RuntimeError):
    def __init__(self, status: int, body: object):
        super().__init__(f"HTTP {status}: {body}")
        self.status, self.body = status, body


class Api:
    def __init__(self, origin: str, ca: Path, certificate: Path, key: Path, evidence: Path):
        self.origin = origin
        self.evidence = evidence
        evidence.mkdir(parents=True, exist_ok=True)
        tls = ssl.create_default_context(cafile=str(ca))
        tls.load_cert_chain(str(certificate), str(key))
        self.cookies = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(
            urllib.request.ProxyHandler({}), urllib.request.HTTPSHandler(context=tls),
            urllib.request.HTTPCookieProcessor(self.cookies),
        )
        self.observations = 0

    def _request(self, path: str, body: bytes | None = None) -> object:
        if not path.startswith("/api/v1/") or path.startswith("//"):
            raise ValueError("local API path required")
        headers = {}
        if body is not None:
            headers = {"Content-Type": "application/json", "Origin": self.origin, "X-RX-Client": "browser-v1"}
        request = urllib.request.Request(self.origin + path, data=body, headers=headers)
        try:
            with self.opener.open(request, timeout=20) as response:
                return json.load(response)
        except urllib.error.HTTPError as error:
            content = error.read(1_048_576)
            try:
                value = json.loads(content)
            except (ValueError, UnicodeError):
                value = {"unreadable_response_sha256": hashlib.sha256(content).hexdigest()}
            raise Rejected(error.code, value) from None

    def login(self, principal: str, password: str) -> object:
        # Intentionally never persist this body or the cookie jar.
        return self._request("/api/v1/session", encoded({"principal": principal, "password": password}))

    def get(self, path: str, **query: object) -> object:
        if query:
            path += "?" + urllib.parse.urlencode(query)
        value = self._request(path)
        self.observations += 1
        publish_new(self.evidence / f"read-{self.observations:05}.json", {"path": path, "response": value})
        return value

    def mutate(self, label: str, path: str, command: object) -> object:
        if not label or any(c not in "abcdefghijklmnopqrstuvwxyz0123456789-_" for c in label):
            raise ValueError("bounded evidence label required")
        request_file = self.evidence / f"{label}.request.json"
        if request_file.exists():
            saved = json.loads(request_file.read_bytes())
            if saved["path"] != path or encoded(saved["body"]["command"]) != encoded(command):
                raise ValueError("a pending request cannot be replaced by a changed command")
        else:
            saved = {"path": path, "body": {"request_key": str(uuid.uuid4()), "command": command}}
            publish_new(request_file, saved)
        return self.recover(label)

    def recover(self, label: str) -> object:
        saved = json.loads((self.evidence / f"{label}.request.json").read_bytes())
        # Every retry reconstructs exactly the saved canonical bytes and original UUID.
        try:
            value = self._request(saved["path"], encoded(saved["body"]))
        except Rejected as error:
            self.observations += 1
            publish_new(self.evidence / f"{label}.rejection-{self.observations:05}.json", {"status": error.status, "body": error.body})
            raise
        self.observations += 1
        # Read-like receipts may evolve (e.g. an ARMING attempt), so retain every response.
        publish_new(self.evidence / f"{label}.response-{self.observations:05}.json", value)
        return value
