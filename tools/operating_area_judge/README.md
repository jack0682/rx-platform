# Offline development judge

This one-shot issuer is deliberately outside the host binary and exported SDK.
Build with the repository's Rust toolchain:

```sh
cargo build --locked --manifest-path tools/operating_area_judge/Cargo.toml
tools/operating_area_judge/target/debug/rx-operating-area-judge decide MAILBOX CHALLENGE_ID OWNER_ONLY_PRIVATE_KEY 30000
tools/operating_area_judge/target/debug/rx-operating-area-judge revoke MAILBOX CHALLENGE_ID OWNER_ONLY_PRIVATE_KEY development/condition-changed
```

The host first publishes `request-CHALLENGE_ID.json`. Decide evaluates the compiled
bounded support-gap rule before signing. Policy denial publishes a negative report
rather than approval; the host labels a negative report as unverified, not as a
verified issuer identity. After verification the host publishes an inert reference;
revoke signs that reference and publishes a revocation. Publication and the host's
actual commit use the same cooperating G1 lock. An immutable conflicting response,
stale partial file, unreadable/busy mailbox or wrong key fails explicitly. Do not
clear files or blindly reissue after an unknown publication outcome.

Only the issuer receives the private-key mount. The host receives a directory,
never key bytes or a signer command. The tool does not authenticate physical facts,
issue equipment safety/quality approvals or establish production key custody.
