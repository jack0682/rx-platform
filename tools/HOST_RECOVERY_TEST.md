# Acceptance tests for Host recovery integration

All paths use self-generated FILE_SIMULATION configuration, accounts, certificates, signing material, and disposable containers/volumes/networks. Only product processes write the product DB. The oracle is a read-only SQLite transaction in a separate network-none process. No physical devices or user operational stores are connected.

## Recovery API for an idle Host

```sh
CARGO_INCREMENTAL=0 python3 tools/test_host_recovery.py --evidence-dir NEW_DIRECTORY
```

Initialize P/H from the two actual images and verify the immutable baseline of the first pinned connection. Keep H running while restarting only P on the same DB. After new producer/session negotiation and preservation of the original registration, ReleaseManager performs context → proposal → approve. Proposal does not yet send Fence. Approval uses the original Fence ID/body for the current runtime origin. Tests verify same-request retrieval, rejection of incorrect operations, recovery-only progress, and unchanged grants/registration/baseline/restrictions/effects.

## Actual operator view

```sh
CARGO_INCREMENTAL=0 ../.tools/browser-python/bin/python tools/host_recovery_browser.py --evidence-dir NEW_DIRECTORY
```

Use a separate fresh run of the same fixture. After an independent API read confirms that the actual POST committed, drop only the browser response. Following refresh, retrieve exactly one proposal using the identical raw body/key stored before sending. Check approval, rediscovery through the record list, desktop/mobile layouts, CSP, console, external resources, and native effects. Do not reuse a fixture already proposed by the API test.

The browser bypasses only trust in the disposable test CA. The separate Python terminal client validates the CA and server certificate, and the actual service retains terminal mTLS, current user, registered terminal, and Origin/CSRF checks. HAR files, cookies, private keys, and login bodies are not exported as evidence. Distinguish failures of automation accessibility selectors from product failures.

## Querying the original unresolved operation's outcome

```sh
CARGO_INCREMENTAL=0 python3 tools/test_host_recovery_known.py \
  --host-fixture ../.tools/linux-executor-build/debug/rx-host-recovery-fixture \
  --release-evidence EXACT_IMAGE_PREFLIGHT_JSON \
  --evidence-dir NEW_DIRECTORY
```

P/E are actual product executables. H is test-harness-only `rx-host-recovery-fixture run CONFIG`, using the same S service/RPC/publisher/FileDevice. Validate its Linux ELF format, architecture, and hash, and mount it read-only. Do not describe this executable as fault-injection testing of the shipped rx-hostd itself. Reuse actual rx-hostd init and signed commissioning/API; do not inject Engine rows, authority, Runs, or receipts.

- The NativeAdapter hides the returned result after recording the original FileDevice effect. It preserves the original SEND_ENTERED and invocation; SUBMIT/effects are each 1.
- Restart only P. The existing operation is UNKNOWN/NONE/QUARANTINED, and the current RuntimeRestart origin specifies the approval scope.
- Without a marker after approval, the original lookup is HIDDEN with 0 evidence. The marker is an empty file changing only when the simulated result becomes observable.
- After creating the marker, the original lookup for the same operation returns FOUND. Compare separate calls and FileDevice-effect ledgers, and verify the original capture and P's ingestion of the actual Host evidence/publisher prefix.
- This process's completion rule has postconditions. Because epoch/scope continuity between the current cell and historical permit is broken, overall work retains UNKNOWN/NONE and QUARANTINED even when native success is obtained. Distinguish outcome retrieval from process completion decisions.
- Repeated queries return the same RESULT_CAPTURED receipt/prefix. The Host reads already stored results and adds no native lookup, SUBMIT, effect, or Host evidence. There must be no new grant, qualification, Arm, part completion, or Run resumption.

`--release-evidence` requires the selected S image ID, original archive/hash, and successful logs/hashes for the 3 specified existing recovery tests. Do not arbitrarily apply old image evidence to the current image. Record fixture build and actual acceptance separately; do not report unimplemented physical-device, Host-replacement, operational-rebind, or resumption paths as passing.
