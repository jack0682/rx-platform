# Offline development-release checkpoints

This verifier authenticates release content against the public-key literal in
`root.rs`. Runtime policy files, environment, arguments and metadata cannot add
keys. The root is **development-only**. Product release-key custody, rotation and
ceremony are **NOT_ESTABLISHED**. Replacing the trusted verifier can replace its
root: neither the verifier binary nor the OS authenticates itself here.

A `rx.release.v1` manifest signs channel, positive monotonic version and SHA256 of
the exact inventory bytes. Its identity uses `RX-RELEASE-IDENTITY-v1` and canonical
JSON. The inventory indexes every relative artifact and the one supported external
interpreter `/usr/bin/python3`. Verification hashes all indexed bytes. Existing
source-content pins remain a separate check in the consumer. Signature messages
bind the distinct release/revocation domain, signer ID and canonical payload.
The same-version/different-content case is refused as rollback.

`verify` yields opaque captured metadata with checked content, **not current
admission or work permission**. `update_revocations` authenticates and persists a
monotonic signed revocation checkpoint independently, including when a candidate
release is denied. Lists cannot shrink. `admit` then atomically checks the retained
revocation checkpoint and release floor before recording a newer release. Reusing
an old proof does not bypass current stored revocations. Invalid content cannot
raise the release floor. Storage failure cannot create a successful admission.
Repository transactions never retry a callback.

Refusal conditions are `release/unsigned`, `release/invalid-signature`,
`release/unknown-key`, `release/revoked`, `release/rollback` and
`release/content-mismatch`. Malformed input and invalid/unavailable state have
separate conditions. Verification uses strict JSON decoding and Ed25519 strict
verification; it never promotes a parse failure or panic to a refusal receipt.

The floor survives **ordinary restart with intact retained local state**. Whole
state rollback/deletion is **NOT_DETECTED**. G1's cooperative single-writer lock
and SQL transaction are not tamper-resistant monotonic storage. This is the same
kind of retained trusted-metadata assumption used by the
[TUF version checks](https://theupdateframework.github.io/specification/v1.0.27/);
this implementation is not TUF and does not implement its roles, expiry or key
rotation. An offline revocation list has freshness **NOT_ESTABLISHED**.

Checkpoints are `EXPLICIT_CALLER_DRIVEN_CHECKPOINTS` with
`NO_TIMER_OR_BACKGROUND_MONITOR`. Authentication records observed bytes, not
immutable paths. The consumer's hash recheck narrows the interval before use;
trusted installation stability and OS remain required between checking and exec.
No physical qualification, operating-area approval or continuous permission is
conferred by signatures or persistent history.

`tools/sign_release.py` is an offline authoring tool requiring an owner-only key
file and OpenSSL. Its output contains public metadata only. The key must never
enter Git, a Docker build context, runtime image or log. The image signing stage
must occur after inventory generation and add only `release.json` and
`revocations.json`; never regenerate the inventory to include its own signature.
`tools/check_release_key_custody.py` checks the actual committed trees and optional
exported runtime filesystem for the specific private key, without printing it.
Pre-signed inert test fixtures require no secret in CI. They are development
content, not qualified executables or a product signing authority.

The reviewed development-key ceremony and recovery contract is
[`DEVELOPMENT_KEY_CUSTODY.md`](DEVELOPMENT_KEY_CUSTODY.md). Its attack catalog is
[`release_rotation_attacks.v1.json`](release_rotation_attacks.v1.json). The
procedure must exist and be reviewed before a replacement key is generated;
creating a key does not by itself establish custody.
