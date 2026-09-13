# Idle Host evidence communication test after P restart

`test_host_reconnect.py` stops/restarts only P in a new FILE_SIMULATION installation while keeping H in the same process. It checks that H completes a new P authentication session and Cell.Open negotiation even without creating a new device operation or evidence. This tool does not perform operational registration rebind, qualification restoration, or resumption.

```sh
CARGO_INCREMENTAL=0 python3 tools/test_host_reconnect.py \
  --platform-image rx-platform:runtime-draft \
  --solutions-image rx-solutions:runtime-draft \
  --evidence-dir ../references/implementation/idle-reconnect-new
```

A new evidence path is required. The test uses the two actual product images and existing material exporter/signed-package tools. Product CLIs perform initialization; ledger, Session, and HostRegistration rows are not written directly. The registered terminal performs an actual HTTPS login after validating the generated CA and client certificate. Certificate verification is not disabled; test certificates include an authority key identifier. Private keys, signing seeds, and login cookies remain only in temporary storage.

The assertions are:

- P installation, store generation, and shared clock remain the same across restart, while runtime boot changes.
- H instance, boot, evidence/delivery journals, and installation descriptor remain the same.
- The new producer session belongs to the new P runtime and completes cell negotiation. The test then verifies at least two actual increases of P's evidence cursor revision with through=0, together with reuse of the same session. Waiting alone does not prove repeated probes.
- The original operating HostRegistration contents/version remain intact and bound to the old session. This communication recovery is not described as completed operational reregistration.
- Existing P restrictions are preserved, and the exact RuntimeRestart is added. In this scenario, P restart alone does not add DeviceRestart.
- The unqualified cell gains neither qualification nor a Run; independent FileDevice effects and Host evidence-ledger record count remain 0.

For internal producer/session comparisons, a separate verification process opens SQLite **read-only**. Networking is disabled, and the data volume is mounted read-only. During execution it uses a read transaction over the live WAL. For stopped P, it first confirms actual container exit and uses immutable reads only when the remaining WAL is absent or empty. There is no path that modifies the product DB for reading convenience or creates authority by bypassing the operational API.

Host status `pending_evidence: null` is not interpreted as 0. Because this value is populated during shutdown, the idle test separately queries the actual Host evidence-ledger count. Public API, status, and read-only oracle results are preserved; failed runs do not produce PASS in `result.json`. Failure cleanup affects only simulation containers, volumes, and networks owned by this tool.

This test is limited to an unqualified file device and normal P shutdown/restart on the same store. Active production operations, resolution of UNKNOWN, actual sensor generation continuity, H restart, store restoration/replacement, explicit authority rebind, and physical protection/recovery require separate validation. P exit code 2 is recorded with residual attention and is not expanded into a clean shutdown of the entire site.

Current P records cursor revisions and audit events even for empty Publish calls. A 1-second interval can produce about 86,400 audit records per idle Host per day. This test validates reconnection semantics; long-term retention, capacity/load limits, and probe delivery optimization remain separate validation targets. An unchanged native evidence through value does not mean zero storage cost.
