# English Translation Revision 2026-09-14-english.1

Revision date: 2026-09-14. This revision translates the current vendor-neutral baseline into English for the public repositories. It is **a translation and documentation-byte revision only**, not a change to contract behavior, a new hardware qualification, or a claim that prior validation has been rerun.

## Scope and preserved meaning

All Markdown in this version directory is translated in full, including normative documents, decision/validation guidance, README, and historical revision/review notices. Section structure, table rows, requirements, counterexamples, limits, uncertainty, and links are preserved. Normative message names, field numbers/types, enum tokens/order, operation identity, state transitions, authority/fences, permit consumption, evidence provenance, and outcome/resource-handover rules retain their meaning. Diagram labels and explanatory pseudocode are translated; protocol expressions remain equivalent. No intentional Unicode test data was present. Canonical JSON fixtures and their digest values, where present, are unchanged.

The canonical source remains this directory in `rx_docs`; copies in `rx-platform` and the exported `rx-solutions` SDK must be generated/synchronized from it. Do not maintain a divergent English normative contract in the code repositories. General project documents outside these shared version directories are outside this translation revision.

Retain semantic_version `1.0.0` and wire package `rx.contract.v1`. Existing wire field numbers, enum values, and API meanings are unchanged. Historical `revision_2026-09-14.md` hashes describe the earlier neutral revision and remain historical facts; this record identifies the new English bytes. Historical source links and the distinction between documentary review, software tests, and physical validation are preserved.

## Hash changes

Previous manifest SHA-256: `3499dff92023509d4daaded3bb9d89a822c948a9b2c025e954262c9bef9d5afe`

English manifest SHA-256: `d32519ffd33cc02f5e6f82406ad2fe253421527dc047b807c11beb11a8f7583c`

| Normative document | Previous SHA-256 | English SHA-256 |
|---|---|---|
| `01_responsibility_and_semantics.md` | `66fb36f5087c19fc7fec273a9c7510ce067e905a7dad39015a96434a6485e798` | `03af71059d44919e3c3b8a68b0b765fd12934cb4026b8c53f47aa6b4db1f359b` |
| `02_identity_durability_recovery.md` | `cf857007c6ff6247b62958af0cc09f93b383fd9d816e480afeada2e0f9631869` | `c8ec0784ed3c301370f2fc8399dd1d2214bb0b4bc8c9ee3c4ba6a25df9754fa5` |
| `03_data_and_protocol.md` | `4999f4dc7ba06dfe73a1ed0bb44d865923abfd3d18ad7edf3ec1cdeb6f8533e5` | `6d0ecc949c08dae3689e6912253e1473e1c617d36fc5b92d9fd209a389a4b13f` |
| `04_binding_and_admission.md` | `97ddd4bb3e3224841dfe549ba50ceba087624eb36ebc3abbdb91cc8f834fb97a` | `6351e8e075d593f2c7a2d696610f9a9ab08cd284c7245d745751f80c1923b6cb` |

Both manifests remain canonical JCS UTF-8 without a trailing newline. Integrity records identify this translation revision and the SHA-256 of the actual manifest bytes.

## Coordinated-update impact

Normative-document bytes are included in manifest hashes. Therefore translation changes the negotiated hash even though semantic_version and wire behavior are unchanged. Under the existing exact-hash negotiation rule, old and English peers without a commonly supported hash must reject command/evidence exchange. An unchanged `1.0.0` string does not authorize mixed-hash operation.

Update both contract baselines, manifest/integrity copies, pinned software constants, hash fixtures, platform SDK exports, and SDK source locks as one coordinated repository change. Validate document integrity, copy/export consistency, and the corresponding software contract tests before publishing matching releases. This document records the required coordination; actual software-test execution evidence belongs to the implementation/CI results, not this translation claim.

Do not replace historical evidence, signatures, ledger entries, or release hashes with these new values. Existing installations require an explicit migration retaining old readers or validated dual support and preserving operation/evidence meaning. Translation alone does not commission equipment, resume operations, grant new authority, or authorize a physical deployment.

## Translation checks

Compared against the pre-translation source: headings, table-row counts, fenced-code blocks, message field-number/type sequences, requirement/scenario identifiers, and protocol-only inline expressions were checked for preservation. Explanatory labels/pseudocode were translated without changing their order or conditions. Canonical fixture payloads and expected digests were preserved byte-for-byte. A scan of the version-directory Markdown found no remaining Korean text. These are translation/structural checks, not physical, functional-safety, or distributed-conformance validation.
