# RX Contract and Protocol Design Baseline v1.0

Status: v1.0 document revision 2026-09-14-english.1 · Documentation baseline. Does not establish completed product implementation, interoperability testing, or physical validation.

RX contracts express **who accepted and recorded a request, what action was delivered to a device, and which observations supported the outcome decision**. These meanings must survive changes in ROS, BT, or communication mechanisms.

## Reading order

| Document | Questions answered |
|---|---|
| [01 Responsibility and semantics](01_responsibility_and_semantics.md) | Who owns and decides? What do operation kinds, acceptance, completion, and uncertainty mean? |
| [02 Identity, records, and recovery](02_identity_durability_recovery.md) | How are duplicates, restarts, authority, and storage failures handled? |
| [03 Data, messages, and transport](03_data_and_protocol.md) | Which data/RPCs are exchanged under which ordering, timing, and buffer rules? |
| [04 Device bindings and support conditions](04_binding_and_admission.md) | How does the common contract map to device models, modes, and facilities? |
| [05 Decision record](05_decisions.md) | Which alternatives were selected and why? |
| [06 Counterexamples, validation, and traceability](06_scenarios_and_validation.md) | Do rules hold under loss, races, and different models? What needs subsequent validation? |
| [Method research report](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/contract_research_2026-09-10/research_report.md) | What evidence supports industrial standards and recent methods? |

[Reader review record](review_record.md) · [Canonical representation test vectors](canonical_vectors.md) · [Protocol manifest](protocol_manifest.json)

## Scope

- Targets `rx-platform` and `rx-solutions` on the same site's primary computer, with one authoritative Runtime and one or more device Hosts. External minicomputers are C08 auxiliary equipment.
- The Rust core does not depend on ROS. Solutions owns ROS, BT, native SDKs, UI, and device-specific optional packages. Each of the two images may contain multiple processes.
- Includes finite actions, target-state establishment, mode transitions, control sessions, and startup/shutdown with physical effects. High-rate control samples remain inside solutions.
- When actual models/firmware/calibration/PLC signals are unavailable, **finalize common semantics while withholding execution admission for that binding**. Record required support profiles and open items.
- v1.0 excludes automatic failover across multiple active Runtimes, general Internet remote operation, replacement of safety PLCs, hard real-time guarantees, and exactly-once guarantees for physical actions.

## Core choices

1. Distinguish request acceptance, Host preparation, device acceptance, and operation outcome.
2. Uncertainty is a lack of knowledge. It does not mean device stopping or failure.
3. Do not deliver normal physical commands before persistent records. Once delivery becomes possible, a lost response must not trigger automatic re-execution.
4. Distinguish outcome settlement from the conditions for handing resources/material to the next operation.
5. Internal transport uses gRPC+proto3 optional; semantic identity uses JCS+SHA-256; UI uses HTTP/JSON/SSE.
6. Declare signals, modes, calibration, observations, and stopping capabilities in version-pinned binding profiles and check them at admission.

`MUST/prohibited/required` denotes implementation requirements of this baseline. An `example` is not a physical configuration value. Stated software-limit defaults are v1.0 design choices, not device stopping-performance figures. Validated profiles must provide device-specific time, force, speed, and tolerance values.

This baseline takes precedence over initial plans/examples in 06, 11, 15, and contracts for contract semantics and protocol choices. Product scope (01–04), device support policy (13), and the two-image boundary (14) remain valid. Changes require a new revision, impact cases, and migration rules without retroactively changing recorded-operation meanings.

Vendor-neutralization scope, compatibility, and hash changes are in the [documentation revision record](revision_2026-09-14.md). The [English translation revision](translation_revision_2026-09-14.md) records the subsequent translation-only byte/hash changes. Do not present the 2026-09-10 review as new execution validation of these revisions.
