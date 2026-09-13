# Contract Design Decision Record

Scope: RX Contract v1.0 · Status: v1.0 design decisions finalized · [Method research](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/contract_research_2026-09-10/research_report.md)

## 1. Decision criteria

Existing D01–D10 were applied to the same failure cases. Alternatives that do not hide limits required for durability and physical correctness were selected first, then compared for the two-container structure, Rust/ROS boundary, and operational burden. These choices are design judgments under stated conditions, not experimentally measured physical superiority.

| Decision | Alternatives compared | Selection, rationale, and cost | Counterexamples/links |
|---|---|---|---|
| D01 IDs/duplicates | Caller operation UUID only / platform ID+caller key / content hash only | **Platform UUIDv7 + scoped key + canonical intent digest**. Distinguishes client restarts from distinct repeated production. Accepts long-term small-tombstone storage cost | SC02·05. Reject same key/different content; allow different key/same content |
| D02 Operation classification | execute/stop for everything / fully model-specific APIs / common envelope+kind-specific bodies | **Five kinds and six typed bodies**, with separate reads. Preserves common reliability semantics and mode/stream/startup completion meanings | SC01·07·08·09·14. Requires profile-authoring effort |
| D03 Acceptance | In-memory acceptance / platform-only durability / durable stages on both sides | **Separate platform ADMITTED, Host PREPARED/SEND_ENTERED, and native acceptance**. Distinguishes crash windows and prevents uncertain repeat calls. Adds Host volumes and retrieval protocol | SC02·03. Does not provide physical exactly-once |
| D04 Authority | In-memory mutex / lease only / Host fence+lease+handover | **Single Runtime, resource fences, Host grants, and handover confirmation**. Blocks delayed old-owner commands at the receiver. Exclusivity against external native clients needs separate profiles | SC03·08·14. Automatic active-active is out of scope |
| D05 Races | First success/cancel wins / timeout=failure / evidence-based outcome+separate resource state | **Evidence-based decisions, immutable outcomes, and DISPUTED incidents**, with independent cancel intent. Adds complexity to UI and recovery procedures | SC04. Conflicting evidence can trigger quarantine even after success was recorded |
| D06 Events | Best-effort topic / persistent log of every sample / separate control-event journal and telemetry | **Durable seq, replay, snapshots, gaps, and slow-consumer termination**, allowing high-rate telemetry coalescing. Bounds storage and control-path blocking | SC06·11. No infinite retention/buffering |
| D07 Identity | Raw protobuf hash / deterministic protobuf / canonical CBOR / semantic JCS | **Explicit canonical intent+RFC8785 JCS+SHA256**. Readable for UI/support tooling and avoids protobuf implementation differences. Requires canonical validators/fixtures in both languages | SC12. Fixes SI units, presence, defaults, and set/array semantics |
| D08 Atomicity | Multiple appends / distributed 2PC / local atomic state+event+outbox with Host inbox | **Two local transactions; external physical boundaries represented as unknown**. SQLite adapter chosen for the first cell; domain core remains storage-independent | SC02·03·13. Storage errors block new effects; local protection is an exception |
| D09 Time | Global timeout / peer wall clock / purpose-specific timeout+receiver monotonic | **Separate response, prepare, execution, freshness, grant, and deadman timing**. Validate expiry values per operation. Do not infer clock continuity after reboot | SC03·06·08·09. Some bindings need timing tests before admission readiness |
| D10 Compatibility/authority | Ignore unknown fields / compare minor only / explicit schema hash, capability, and role negotiation | **Exact negotiated schema + strict input + mTLS identity/role**. Prevents fabricated fallback motion. Mixed versions require dual support or coordinated upgrades | SC10·12. More conservative than default protobuf forward compatibility |

## 2. Additional transport/execution choices

| ID | Decision | Exclusion/future reconsideration conditions |
|---|---|---|
| A01 | Internal gRPC/HTTP2 + proto3 optional; UI HTTP/JSON/SSE | Custom TCP/FFI only for measured bottlenecks. Protobuf Editions after confirming Rust/C++ generator compatibility |
| A02 | BT executor in solutions; checkpoint and operation mapping in platform | Do not merge BT into the reliability core. Another workflow engine can replace it if it satisfies the same C02/C03 contracts |
| A03 | Do not require a broker or Temporal/Restate/DBOS for the first cell | Reconsider above C02 or outside C04 when long-lived orders/multiple cells/multiple consumers and an operational organization require it |
| A04 | Use industrial standards as layer-specific bindings/external representations | Do not apply OPC/PackML/VDA5050 uniformly to every robot/CNC/stream |
| A05 | High-rate streams stay inside solutions; only sessions/decisions are recorded in platform | Extending Rust RPC to a 1kHz actuator control loop requires a separate real-time design |
| A06-R1 | Select profiles/dependencies per device and apply the same admission conditions | Replaces the previous inclusion policy with the personal project's vendor-neutral policy on 2026-09-14. Automatic activation of unvalidated models remains prohibited |

## 3. Conditions for changing decisions

Supersede a decision ID when new evidence changes requirements. Existing documents/operations must retain the meaning of their original profile/schema/digest. Schema changes require contract tests proving that a translator preserves meaning, or retention of old readers. DB migration is a separately approved implementation stage with prior backup, generation, and rollback procedures.

For example, if a PLC provides durable request IDs/result history and reliably deduplicates repeated submissions, a future minor version/separate capability may extend uncertain-resubmission rules for that profile. Until then, do not relax D03 based on “probably idempotent.”

## 4. Why subsequent validation remains necessary

The design baseline fixes **what evidence is required to allow execution**. Inventing unavailable PLC signals, model calibration, or stopping performance would fabricate facts rather than complete the design. Remaining binding conditions are tracked by responsible role, deliverable, and impact in the [validation list](06_scenarios_and_validation.md). These unknowns do not defer the common contract's conclusions; they limit the delivery scope of each binding.

This document is the vendor-neutral documentation revision. Earlier design-audit results in the tables apply only to the contemporaneous scope of the [original document](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/docs/contracts/v1.0/05_decisions.md), and are not execution-test results for this revision.
