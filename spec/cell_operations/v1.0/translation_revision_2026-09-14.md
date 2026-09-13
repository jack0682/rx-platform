# English Translation Revision 2026-09-14-english.1

Revision date: 2026-09-14. This revision translates the current vendor-neutral baseline into English for the public repositories. It is **a translation and documentation-byte revision only**, not a change to contract behavior, a new hardware qualification, or a claim that prior validation has been rerun.

## Scope and preserved meaning

All Markdown in this version directory is translated in full, including normative documents, decision/validation guidance, README, and historical revision/review notices. Section structure, table rows, requirements, counterexamples, limits, uncertainty, and links are preserved. Normative message names, field numbers/types, enum tokens/order, operation identity, state transitions, authority/fences, permit consumption, evidence provenance, and outcome/resource-handover rules retain their meaning. Diagram labels and explanatory pseudocode are translated; protocol expressions remain equivalent. No intentional Unicode test data was present. Canonical JSON fixtures and their digest values, where present, are unchanged.

The canonical source remains this directory in `rx_docs`; copies in `rx-platform` and the exported `rx-solutions` SDK must be generated/synchronized from it. Do not maintain a divergent English normative contract in the code repositories. General project documents outside these shared version directories are outside this translation revision.

Retain semantic_version `1.0.0` and wire package `rx.cell.v1`. Existing wire field numbers, enum values, and API meanings are unchanged. Historical `revision_2026-09-14.md` hashes describe the earlier neutral revision and remain historical facts; this record identifies the new English bytes. Historical source links and the distinction between documentary review, software tests, and physical validation are preserved.

## Hash changes

Previous manifest SHA-256: `0c167639b3d048f6717819f9cc9b92bba8c3a11a4c41c7068d3a04c44e7be0ea`

English manifest SHA-256: `5e75ab4d3d72d4b22082003dbec5e2ca54562cc4302d2e306b07ce64d215d71b`

| Normative document | Previous SHA-256 | English SHA-256 |
|---|---|---|
| `01_scope_conditions_functions.md` | `ee34c7da2c426a2c775e3d84a902ac1b9ce50fc09f4a140c333c7cbdf0bd561c` | `308af9c0cec3ce8dc5a4b35d84bbaf55a9237e6b07c037a306a891812ae2248e` |
| `02_authorization_invalidation.md` | `677a05ced54bc030ac7ee830c31e09da7aa61e910656cd4864c42b7e7fd826df` | `6f8aa71c40aa27fa1bbf57d8196f4b91f27ca817856aa5aedc8e719ef094c46c` |
| `03_intervention_recovery_change.md` | `3cb1b7632e3d24c8a1e8aabd011919d8dc8bd50e0c19e1046864f478d6884e18` | `3b5ea7226d160b649051c2478cc18c06c10fe6b5d6f0af54e6d2474c6971c961` |
| `04_protocol_integration_ui.md` | `cdbca73743692e4c58f248f6dd5349f855dd38cfe1327371454691620ecc6910` | `95a4e504148fdff0497a264b28653254329ae5168f322067626ab9bed6be50f8` |

The required base manifest changes from `3499dff92023509d4daaded3bb9d89a822c948a9b2c025e954262c9bef9d5afe` to `d32519ffd33cc02f5e6f82406ad2fe253421527dc047b807c11beb11a8f7583c`. `required_base_manifest_sha256` is updated with the four cell-document hashes.

Both manifests remain canonical JCS UTF-8 without a trailing newline. Integrity records identify this translation revision and the SHA-256 of the actual manifest bytes.

## Coordinated-update impact

Normative-document bytes are included in manifest hashes. Therefore translation changes the negotiated hash even though semantic_version and wire behavior are unchanged. Under the existing exact-hash negotiation rule, old and English peers without a commonly supported hash must reject command/evidence exchange. An unchanged `1.0.0` string does not authorize mixed-hash operation.

Update both contract baselines, manifest/integrity copies, pinned software constants, hash fixtures, platform SDK exports, and SDK source locks as one coordinated repository change. Validate document integrity, copy/export consistency, and the corresponding software contract tests before publishing matching releases. This document records the required coordination; actual software-test execution evidence belongs to the implementation/CI results, not this translation claim.

Do not replace historical evidence, signatures, ledger entries, or release hashes with these new values. Existing installations require an explicit migration retaining old readers or validated dual support and preserving operation/evidence meaning. Translation alone does not commission equipment, resume operations, grant new authority, or authorize a physical deployment.

## Translation checks

Compared against the pre-translation source: headings, table-row counts, fenced-code blocks, message field-number/type sequences, requirement/scenario identifiers, and protocol-only inline expressions were checked for preservation. Explanatory labels/pseudocode were translated without changing their order or conditions. Canonical fixture payloads and expected digests were preserved byte-for-byte. A scan of the version-directory Markdown found no remaining Korean text. These are translation/structural checks, not physical, functional-safety, or distributed-conformance validation.
