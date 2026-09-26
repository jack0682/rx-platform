# Development release-key custody and rotation

This procedure establishes custody for the **development release key only**.
Product signing custody remains `NOT_ESTABLISHED`. Release signatures do not
authenticate the operating system or verifier binary and do not grant work,
equipment, safety, or quality authority.

## Ceremony order

The order is a gate, not guidance:

1. Review this procedure and `release_rotation_attacks.v1.json` before a key
   exists.
2. Select two owner-controlled OpenPGP recovery identities. Both identities
   must be able to decrypt each recovery envelope.
3. Select two envelope destinations outside every source checkout, build
   context, runtime tree, evidence publication directory, and agent workspace.
   The destinations must be on different filesystem devices. The primary is an
   owner-only local custody directory; the recovery copy is encrypted removable
   media.
4. Run `tools/development_release_custody.py create`. The tool generates an
   Ed25519 key in memory, independently encrypts it to both recovery identities,
   atomically creates both owner-only envelopes, and decrypts both again. It
   refuses an existing destination, symlinked parent, repository-contained
   destination, missing recovery identity, same-device pair, or failed
   round-trip.
5. Review the public ceremony record. It contains the public key, recovery
   fingerprints, envelope hashes, storage classes and separate-device result;
   it contains no private bytes or private paths.
6. Only after steps 1–5 pass may a reviewed source change replace the compiled
   root. No old root or compatibility key is retained.
7. Sign through `tools/sign_release.py --encrypted-key`. Decryption is piped to
   OpenSSL; the signer never accepts a plaintext key path and never prints the
   private key.
8. Run the complete rotation attack catalog and the custody scanner against all
   committed histories, current publishable files, and the flattened runtime
   image before merge.

## Storage and recovery contract

- Required copies: one primary encrypted envelope and one removable-media
  encrypted envelope on distinct filesystem devices.
- Required recovery identities: the two explicit full OpenPGP fingerprints in
  the ceremony record. Either identity may recover either envelope.
- Plaintext persistence: forbidden. Generation, decryption, public-key
  derivation and signing pass private PEM bytes through process pipes. The tool
  does not create a plaintext key file. OS process memory, GnuPG/OpenSSL, the
  kernel and the trusted host remain in the boundary.
- Publication: only the public ceremony record, public key, signed metadata and
  signatures may enter Git or an image. Envelope paths and private material are
  never publication inputs.
- Recovery rehearsal: `development_release_custody.py verify` must decrypt both
  envelopes independently, match their private bytes, derive the reviewed
  public key, verify both envelope hashes and confirm different devices. A
  signer invocation must then succeed once from each copy and produce the same
  deterministic Ed25519 signature for the same message.
- Single-copy loss: losing either envelope does not block signing; recover from
  the other copy and immediately run a reviewed replacement ceremony to restore
  two distinct-device copies.
- Recovery-identity loss: one of the two recovery identities is sufficient.
  Loss of both identities makes the envelopes unrecoverable and signing
  impossible. It does not justify a verifier bypass; a new root rotation and
  its complete attack suite are required.
- Suspected key disclosure: stop signing, treat the current key as compromised,
  rotate the compiled root without a dual-root window, and revoke affected
  release identities. A lost key is not automatically evidence of disclosure.

## Explicit limitations

- This is single-maintainer custody, not two-person authorization or HSM-backed
  product signing.
- The two recovery identities and primary copy currently reside under the same
  maintainer's control. The removable encrypted copy protects against loss of a
  workspace or primary filesystem, not coercion or total identity compromise.
- Secure erasure of process memory, swap, APFS snapshots, SSD wear-leveling or
  GnuPG agent state is not established.
- Offline revocation freshness remains `NOT_ESTABLISHED`: without trusted time
  or an online monotonic checkpoint, an old but correctly signed latest-known
  list can be replayed into a fresh state. Durable version/subset checks protect
  only an intact retained state.
- Whole release-state database rollback/deletion remains `NOT_DETECTED`.

## Loss response

Never copy an old root back into source, add a second accepted root, accept an
unsigned release, or inject a runtime key. If no recovery envelope and no
recovery identity remains, registered release admission becomes unavailable
until a separately reviewed rotation is completed. That outage is the intended
fail-closed behavior.
