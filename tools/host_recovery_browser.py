#!/usr/bin/env python3
"""Real registered-terminal Host recovery UI acceptance on a fresh restart fixture.

Pass ``exercise`` as ``test_host_reconnect.exercise(..., after_reconnect=exercise)``.
The standalone entry point delegates fixture lifecycle to ``test_host_reconnect.main``.
Do not run ``test_host_recovery.exercise`` first on the same fixture: this hook
requires an empty recovery list, and never deletes or repairs pre-existing records.

The browser uses the production S bundle served by P, actual release credentials
and the disposable terminal certificate. Only the proposal's HTTP reply is lost;
its real writer commit is independently read before the browser request is aborted.
No response body, business state, permission, Host ACK or source observation is faked.
No HAR, trace, cookie jar, credential body, certificate or private key is exported.
"""
from __future__ import annotations

import hashlib
import json
import re
import time
from pathlib import Path
from urllib.parse import urlsplit, urlunsplit
from uuid import UUID

from playwright.sync_api import expect, sync_playwright

import test_host_reconnect as restart
from cell_delivery.api import Api, publish_new
from test_host_recovery import _host_grants

PENDING_KEY = "rx.pending-browser-request.v1"
PROPOSE = "/api/v1/host-recoveries"
APPROVE = "/api/v1/host-recovery/approve"
PROGRESS = "/api/v1/host-recovery/progress"
ALLOWED_POSTS = {"/api/v1/session", "/api/v1/session/end", PROPOSE, APPROVE, PROGRESS}


def _path(url: str) -> str:
    return urlsplit(url).path


def _public_url(url: str) -> str:
    """Drop URI credentials, query values and fragments from browser diagnostics."""
    parsed = urlsplit(url)
    if not parsed.scheme or parsed.scheme not in {"http", "https"}:
        return parsed.scheme or url[:160]
    authority = parsed.hostname or ""
    if parsed.port:
        authority += f":{parsed.port}"
    return urlunsplit((parsed.scheme, authority, parsed.path, "", ""))


def _text(value: object, secrets: list[str]) -> str:
    text = str(value)
    for secret in secrets:
        if secret:
            text = text.replace(secret, "[REDACTED]")
    text = re.sub(r"-----BEGIN [^-]+-----.*?-----END [^-]+-----", "[REDACTED PEM]", text, flags=re.S)
    text = re.sub(r"(?i)(authorization|cookie|set-cookie)\s*[:=][^\n]+", r"\1: [REDACTED]", text)
    return text[:2000]


def _pending(page) -> dict | None:
    raw = page.evaluate("key => sessionStorage.getItem(key)", PENDING_KEY)
    return None if raw is None else json.loads(raw)


def _request(request, path: str) -> tuple[str, dict]:
    """Only allowlisted public workflow fields may enter durable request evidence."""
    assert request.method == "POST" and _path(request.url) == path
    raw = request.post_data
    assert isinstance(raw, str), "workflow request must have an exact JSON body"
    body = json.loads(raw)
    assert set(body) == {"request_key", "command"}
    UUID(body["request_key"])
    command = body["command"]
    expected = ({"host", "origin", "expected_context", "expected_cells"} if path == PROPOSE else
                {"id", "expected_revision", "proposal_digest", "expected_cells"})
    assert set(command) == expected, "unexpected workflow fields must not be persisted"
    assert all(isinstance(v, str) and v.isdecimal() for v in command["expected_cells"].values())
    return raw, body


def _persisted_before_send(page, body: dict, path: str, c: dict, host: str, cell: str) -> dict:
    pending = _pending(page)
    assert pending is not None, "request was not persisted before transmission"
    assert pending["route"] == path
    assert {"request_key": pending["request_key"], "command": pending["command"]} == body
    assert pending["principal"] == "release"
    assert pending["installation"] == c["installation"]["id"]
    assert pending["store_generation"] == c["installation"]["store_generation"]
    review = pending["recovery_review"]
    assert review["host"] == host and review["origin"] == cell
    assert review["runtime_boot"] == c["installation"]["runtime_boot"]
    return pending


def _view(value: dict, host: str, cell: str, binding_id: str | None = None) -> dict:
    assert value["view"]["operation_authorized"] is False
    binding = value["view"]["binding"]
    assert binding["schema"] == "rx.host-recovery.v1"
    assert binding["context"]["host"] == host and binding["context"]["origin"] == cell
    if binding_id is not None:
        assert binding["id"] == binding_id
    assert binding["phase"] in {"PROPOSED", "FENCING", "RECOVERY_ONLY", "ATTENTION"}
    assert re.fullmatch(r"[0-9a-f]{64}", value["proposal_digest"])
    return binding


def _fences(binding: dict) -> dict:
    return {cell: {"task": step["task"], "payload_digest": step["payload_digest"]}
            for cell, step in binding["fences"].items()}


def _assert_original_fences(binding: dict, before: dict) -> set[str]:
    original = {row["id"]: row for row in before["outbox"]}
    requests = set()
    for cell, step in binding["fences"].items():
        task = step["task"]
        request = task["request"]
        assert request in original and task["originating_message"] == request
        assert task["cell"] == cell and task["binding"] == binding["id"]
        assert original[request]["value"]["kind"] == "FENCE"
        requests.add(request)
    assert requests, "the fresh restart fixture must contain an original Fence"
    return requests


def exercise(c: dict) -> dict:
    """Use a fresh ``test_host_reconnect`` hook context; do not start or stop services here."""
    docker = c["docker"]
    fixture = c["browser"]
    origin = fixture["origin"]
    parsed_origin = urlsplit(origin)
    assert parsed_origin.scheme == "https" and parsed_origin.hostname in {"127.0.0.1", "localhost", "::1"}
    host = "host/sim"
    cell = c["delivery"]["cell"]
    output = docker.evidence / "host-recovery-browser"
    output.mkdir(parents=True, exist_ok=False)
    secrets = [str(value) for value in fixture["credentials"].values()]
    observer = Api(origin, c["final"] / fixture["ca"], c["final"] / fixture["certificate"],
                   c["final"] / fixture["private_key"], output / "api")
    observer.login("release", fixture["credentials"]["release"])
    initial_list = observer.get(PROPOSE, host=host, limit=50)
    assert initial_list["items"] == [] and initial_list["next"] is None, (
        "Use a separate fresh after_reconnect fixture; an API recovery exercise already created records"
    )
    read_context = observer.get("/api/v1/host-recovery-context", host=host, origin=cell)
    assert not read_context["context"]["blockers"], "fresh Host recovery context is blocked"
    before = c["oracle"]()
    assert before["hosts"] == c["stable"]["hosts"]
    grants = _host_grants(c, "browser-recovery-grants-before")
    assert c["evidence_count"]("browser-recovery-evidence-before") == 0

    traffic: list[dict] = []
    prohibited: list[dict] = []
    page_errors: list[str] = []
    console_errors: list[dict] = []
    expected_console_errors: list[dict] = []
    failed_requests: list[dict] = []
    security_events: list[dict] = []
    external_resources: list[str] = []
    public_requests: dict[str, dict] = {}
    screenshots: list[str] = []
    state = {"signed_in": False, "drop_committed": False, "drop_failure_pending": False,
             "accept_drop_console": False}
    policy = ""
    result = None

    with sync_playwright() as playwright:
        engine = playwright.chromium.launch(headless=True)
        context = engine.new_context(
            ignore_https_errors=True, viewport={"width": 1440, "height": 1100},
            client_certificates=[{"origin": origin, "certPath": str(c["final"] / fixture["certificate"]),
                                  "keyPath": str(c["final"] / fixture["private_key"])}],
        )
        context.add_init_script("""window.__rxRecoveryCsp = [];
          document.addEventListener('securitypolicyviolation', event => {
            window.__rxRecoveryCsp.push({directive:event.effectiveDirective,
              violated:event.violatedDirective,blocked:event.blockedURI,source:event.sourceFile,line:event.lineNumber,column:event.columnNumber,sample:event.sample});
          });""")
        page = context.new_page()
        page.set_default_timeout(20000)

        def security_snapshot() -> None:
            for item in page.evaluate("window.__rxRecoveryCsp || []"):
                security_events.append({"directive": item["directive"], "violated": item["violated"],
                                        "blocked": _public_url(item["blocked"]), "source": _public_url(item.get("source","")), "line":item.get("line"), "column":item.get("column"), "sample":_text(item.get("sample",""),secrets)})
            page.evaluate("window.__rxRecoveryCsp = []")

        def screenshot(name: str) -> None:
            path = output / name
            assert not path.exists()
            page.evaluate("document.fonts.ready")
            page.screenshot(path=str(path), full_page=True, mask=[page.locator('input[type="password"]')])
            screenshots.append(name)

        def overflow() -> dict:
            value = page.evaluate("({width:innerWidth,document:document.documentElement.scrollWidth})")
            assert value["document"] <= value["width"], f"horizontal document overflow: {value}"
            return value

        def console(message) -> None:
            if message.type != "error":
                return
            entry = {"type": message.type, "text": _text(message.text, secrets),
                     "url": _public_url(message.location.get("url", ""))}
            path = _path(message.location.get("url", ""))
            if (not state["signed_in"] and path == "/api/v1/overview" and "401" in message.text):
                expected_console_errors.append({**entry, "reason": "signed-out overview"})
            elif state["accept_drop_console"] and path == PROPOSE and "ERR_FAILED" in message.text:
                expected_console_errors.append({**entry, "reason": "intentional committed-reply loss"})
            else:
                console_errors.append(entry)

        def record_failure(request) -> None:
            failure = str(request.failure or "")
            intentional = state["drop_failure_pending"] and request.method == "POST" and _path(request.url) == PROPOSE
            if intentional:
                state["drop_failure_pending"] = False
            navigation_abort = request.method == "GET" and "ERR_ABORTED" in failure
            failed_requests.append({"method": request.method, "path": _path(request.url),
                                    "failure": _text(failure, secrets), "expected": intentional or navigation_abort,
                                    "intentional_reply_loss": intentional})

        def record_request(request) -> None:
            if not request.url.startswith(origin + "/"):
                external_resources.append(_public_url(request.url))
            if request.method not in {"GET", "HEAD"}:
                traffic.append({"method": request.method, "path": _path(request.url)})

        def guard(route) -> None:
            request = route.request
            path = _path(request.url)
            if request.method not in {"GET", "HEAD"}:
                entry = {"method": request.method, "path": path}
                if request.method != "POST" or path not in ALLOWED_POSTS:
                    prohibited.append(entry)
                    route.abort("blockedbyclient")
                    return
            route.continue_()

        page.on("pageerror", lambda error: page_errors.append(_text(error, secrets)))
        page.on("console", console)
        page.on("requestfailed", record_failure)
        page.on("request", record_request)
        context.route("**/api/v1/**", guard)
        try:
            navigation = page.goto(origin, wait_until="networkidle")
            assert navigation is not None and navigation.ok
            policy = navigation.headers.get("content-security-policy", "")
            for directive in ["script-src 'self'", "style-src 'self'", "font-src 'self'"]:
                assert directive in policy, f"missing production CSP directive: {directive}"
            assert "'unsafe-inline'" not in policy and "'unsafe-eval'" not in policy
            expect(page.get_by_role("heading", name="운영 공간에 로그인", exact=True)).to_be_visible()
            page.get_by_label("계정", exact=True).fill("release")
            page.get_by_label("비밀번호", exact=True).fill(fixture["credentials"]["release"])
            page.get_by_role("button", name="로그인", exact=True).click()
            expect(page.get_by_role("button", name="로그아웃", exact=True)).to_be_visible()
            state["signed_in"] = True
            page.evaluate("document.fonts.ready")
            assert page.evaluate('(font) => document.fonts.check(font, "복구 조회 연결")', '400 16px "IBM Plex Sans KR"')

            def open_recovery() -> object:
                page.get_by_role("button", name="구성", exact=True).click()
                page.get_by_role("tab", name=cell, exact=True).click()
                panel = page.get_by_role("region", name="Host 복구 조회 연결", exact=True)
                expect(panel).to_be_visible()
                panel.get_by_role("combobox").select_option(host)
                expect(panel.get_by_role("heading", name="현재 연결 문맥", exact=True)).to_be_visible()
                return panel

            panel = open_recovery()
            expect(panel.get_by_role("button", name="복구 연결 제안 검토", exact=True)).to_be_enabled()
            screenshot("proposal-context-desktop.png")
            panel.get_by_role("button", name="복구 연결 제안 검토", exact=True).click()
            dialog = page.get_by_role("dialog", name="이 연결의 복구를 제안할까요?", exact=True)
            expect(dialog).to_be_visible()
            screenshot("proposal-confirmation.png")

            def lose_proposal(route) -> None:
                raw, body = _request(route.request, PROPOSE)
                assert body["command"]["host"] == host and body["command"]["origin"] == cell
                pending = _persisted_before_send(page, body, PROPOSE, c, host, cell)
                response = route.fetch()
                assert response.ok, f"proposal HTTP {response.status} before simulated loss"
                committed = response.json()
                binding = _view(committed, host, cell)
                assert binding["phase"] == "PROPOSED"
                assert binding["requested_context_digest"] == body["command"]["expected_context"]
                actual = observer.get("/api/v1/host-recovery", id=binding["id"])
                assert _view(actual, host, cell)["id"] == binding["id"]
                assert actual["proposal_digest"] == committed["proposal_digest"]
                public_requests["proposal"] = {"raw_body": raw, "body": body, "pending": pending,
                                                "committed": committed, "independently_read": actual}
                publish_new(output / "proposal-committed-before-drop.json", public_requests["proposal"])
                state["drop_committed"] = True
                state["drop_failure_pending"] = True
                state["accept_drop_console"] = True
                route.abort("failed")

            page.route(f"**{PROPOSE}", lose_proposal, times=1)
            dialog.get_by_role("button", name="검토한 연결 제안", exact=True).click()
            expect(page.get_by_role("button", name="같은 요청 확인", exact=True)).to_be_enabled()
            original = public_requests["proposal"]
            binding = original["committed"]["view"]["binding"]
            binding_id = binding["id"]
            tasks = _fences(binding)
            original_fence_ids = _assert_original_fences(binding, before)
            assert c["oracle"]()["outbox"] == before["outbox"], "proposal emitted a Fence before approval"
            assert _host_grants(c, "browser-recovery-grants-proposed") == grants
            screenshot("proposal-reply-lost.png")
            security_snapshot()

            page.reload(wait_until="networkidle")
            state["accept_drop_console"] = False
            expect(page.get_by_role("button", name="같은 요청 확인", exact=True)).to_be_enabled()
            assert _pending(page) == original["pending"], "reload changed the pending request"
            assert sum(item["path"] == PROPOSE for item in traffic) == 1, "reload automatically posted a proposal"
            assert len(observer.get(PROPOSE, host=host, limit=50)["items"]) == 1
            recovered_bodies = []

            def recover_proposal(route) -> None:
                raw, body = _request(route.request, PROPOSE)
                assert raw == original["raw_body"] and body == original["body"]
                recovered_bodies.append(raw)
                route.continue_()

            page.route(f"**{PROPOSE}", recover_proposal, times=1)
            with page.expect_response(lambda response: _path(response.url) == PROPOSE and response.request.method == "POST") as response_info:
                page.get_by_role("button", name="같은 요청 확인", exact=True).click()
            response = response_info.value
            assert response.ok, f"same-key recovery HTTP {response.status}"
            recovered = response.json()
            assert _view(recovered, host, cell, binding_id)["requested_context_digest"] == binding["requested_context_digest"]
            assert recovered["proposal_digest"] == original["committed"]["proposal_digest"]
            expect(page.get_by_role("button", name="같은 요청 확인", exact=True)).to_have_count(0)
            assert _pending(page) is None and recovered_bodies == [original["raw_body"]]
            listed = observer.get(PROPOSE, host=host, limit=50)
            assert [row["view"]["binding"]["id"] for row in listed["items"]] == [binding_id]
            publish_new(output / "same-request-recovery.json", {"original_body": original["raw_body"],
                "recovered_body": recovered_bodies[0], "binding": binding_id, "receipt": recovered})

            panel = open_recovery()
            expect(panel.get_by_role("heading", name="승인 전 제안", exact=True)).to_be_visible()
            panel.get_by_role("button", name="복구 조회 연결 승인 검토", exact=True).click()
            dialog = page.get_by_role("dialog", name="이 범위의 복구 조회 연결을 승인할까요?", exact=True)
            expect(dialog).to_be_visible()
            screenshot("approval-confirmation.png")

            def inspect_approval(route) -> None:
                raw, body = _request(route.request, APPROVE)
                _persisted_before_send(page, body, APPROVE, c, host, cell)
                assert body["request_key"] != original["body"]["request_key"]
                assert body["command"]["id"] == binding_id
                assert body["command"]["proposal_digest"] == original["committed"]["proposal_digest"]
                assert body["command"]["expected_cells"] == original["body"]["command"]["expected_cells"]
                public_requests["approval"] = {"raw_body": raw, "body": body}
                route.continue_()

            page.route(f"**{APPROVE}", inspect_approval, times=1)
            with page.expect_response(lambda response: _path(response.url) == APPROVE and response.request.method == "POST") as response_info:
                dialog.get_by_role("button", name="검토한 범위 승인", exact=True).click()
            response = response_info.value
            assert response.ok, f"approval HTTP {response.status}"
            approved = response.json()
            deadline = time.monotonic() + 45
            progress_count = 0
            while True:
                current = _view(approved, host, cell, binding_id)
                assert approved["proposal_digest"] == original["committed"]["proposal_digest"] and _fences(current) == tasks
                assert current["phase"] != "ATTENTION", "actual recovery requires attention; no automatic repair is permitted"
                if current["phase"] == "RECOVERY_ONLY":
                    break
                assert current["phase"] == "FENCING" and time.monotonic() < deadline and progress_count < 5
                for process in [c["p"], c["h"]]:
                    assert docker.state(process)["State"]["Running"], "product process stopped during recovery"
                button = panel.get_by_role("button", name="승인한 차단 요청 진행·연결 확인", exact=True)
                expect(button).to_be_enabled()
                with page.expect_response(lambda r: _path(r.url) == PROGRESS and r.request.method == "POST") as response_info:
                    button.click()
                response = response_info.value
                assert response.ok, f"progress HTTP {response.status}"
                assert response.request.post_data_json == {"id": binding_id}
                approved = response.json()
                progress_count += 1
            expect(panel.get_by_role("heading", name="복구 조회 연결", exact=True)).to_be_visible()
            expect(panel.get_by_text("운전 재개 승인 필요 · 이 연결에는 작업 실행 권한이 없습니다.", exact=True)).to_be_visible()
            assert _pending(page) is None
            for step in current["fences"].values():
                assert step["phase"] == "ACKNOWLEDGED" and step["acknowledgment"]["invalidation"] == step["task"]["request"]
            screenshot("recovery-only-desktop.png")
            desktop = overflow()
            publish_new(output / "approved-recovery.json", {"request": public_requests["approval"], "response": approved,
                                                           "explicit_progress_clicks": progress_count})
            security_snapshot()

            # Reload drops the in-memory receipt. The existing record must be discoverable by its displayed row.
            page.reload(wait_until="networkidle")
            assert _pending(page) is None
            panel = open_recovery()
            row = panel.get_by_role("row").filter(has_text=binding_id[:8])
            expect(row).to_have_count(1)
            row.get_by_role("button", name="기록 열기", exact=True).click()
            expect(panel.get_by_role("heading", name="복구 조회 연결", exact=True)).to_be_visible()
            expect(panel.get_by_text(re.compile(rf"^기록 {re.escape(binding_id)} · r[0-9]+$"))).to_be_visible()
            screenshot("reloaded-record-discovery.png")
            page.set_viewport_size({"width": 390, "height": 844})
            screenshot("recovery-only-mobile.png")
            mobile = overflow()
            security_snapshot()

            after = c["oracle"]()
            assert after["hosts"] == before["hosts"] and after["baselines"] == before["baselines"]
            assert after["cells"][0]["value"]["blocks"] == before["cells"][0]["value"]["blocks"]
            assert _host_grants(c, "browser-recovery-grants-after") == grants
            assert c["evidence_count"]("browser-recovery-evidence-after") == 0
            previous_outbox = {row["id"]: row for row in before["outbox"]}
            assert {row["id"] for row in after["outbox"]} == set(previous_outbox)
            for row in after["outbox"]:
                assert row["value"] == previous_outbox[row["id"]]["value"] and row["value"]["kind"] == "FENCE"
                assert row["state"] == ("DELIVERED" if row["id"] in original_fence_ids else previous_outbox[row["id"]]["state"])
            effects = docker.run("exec", c["h"], "/bin/sh", "-c",
                                 "if [ -f /data/host/device/effects.jsonl ]; then cat /data/host/device/effects.jsonl; fi")
            assert not effects.strip(), "browser recovery caused a native effect"
            overview = observer.get("/api/v1/overview")
            scope = next(item for item in overview["cells"] if item["cell"]["value"]["id"] == cell)
            assert not scope["runs"] and scope["cell"]["value"]["qualification"] is None
            assert not prohibited and not page_errors and not console_errors and not security_events and not external_resources
            assert all(item["expected"] for item in failed_requests), "unexpected browser request failure"
            assert sum(item["intentional_reply_loss"] for item in failed_requests) == 1
            assert sum(item["reason"] == "intentional committed-reply loss" for item in expected_console_errors) <= 1
            assert sum(item["path"] == PROPOSE for item in traffic) == 2
            assert sum(item["path"] == APPROVE for item in traffic) == 1
            assert sum(item["path"] == PROGRESS for item in traffic) == progress_count
            result = {"schema": "rx.host-recovery-browser-test.v1", "status": "PASS", "simulation_only": True,
                "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                "binding": binding_id, "proposal_digest": approved["proposal_digest"], "original_fence_ids": sorted(original_fence_ids),
                "platform_image": c["p_image"]["Id"], "solutions_image": c["s_image"]["Id"],
                "same_key_and_raw_body_recovered": True, "independent_read_before_reply_drop": True,
                "reloaded_list_discovery": True, "desktop": desktop, "mobile": mobile, "screenshots": screenshots,
                "content_security_policy": policy, "native_effects": 0, "operation_authorized": False,
                "grant_records_unchanged": True, "operating_registration_unchanged": True, "restrictions_preserved": True,
                "limitations": ["FILE_SIMULATION idle restart fixture only; no physical or commissioned recovery acceptance.",
                    "The browser ignores disposable test-CA trust; the independent Python terminal client verifies the CA and server certificate.",
                    "No original native operation is present, so this harness does not claim operation-query reconciliation coverage.",
                    "This hook does not start or stop product services; the enclosing reconnect fixture owns lifecycle."]}
            publish_new(output / "result.json", result)
        except Exception as error:
            try:
                security_snapshot()
                screenshot("failure.png")
            except Exception:
                pass
            publish_new(output / "failure.json", {"status": "FAIL", "type": type(error).__name__,
                "message": _text(error, secrets), "screenshots": screenshots})
            raise
        finally:
            try:
                publish_new(output / "browser-diagnostics.json", {"page_errors": page_errors,
                    "unexpected_console_errors": console_errors, "expected_console_errors": expected_console_errors,
                    "request_failures": failed_requests, "security_policy_violations": security_events,
                    "prohibited_mutations": prohibited, "allowed_mutation_metadata": traffic,
                    "external_resources": external_resources})
            finally:
                try:
                    context.close()
                finally:
                    engine.close()

    assert result is not None
    return {"result": "host-recovery-browser/result.json",
            "sha256": hashlib.sha256((output / "result.json").read_bytes()).hexdigest()}


if __name__ == "__main__":
    restart.main(exercise)
