# Control-state journal and audit records

Storage schema3 separates two sequence numbers.

| Store | Role |
|---|---|
| `events` | Internal application audit/diagnostic records. The Host continues to use it as its existing evidence outbox journal |
| `control_events` | P's committed control-state changes, with an independent continuous seq |
| `control_entities` | Current control-state projection updated in the same commit |

`site-cell-control-v1` identifies the cell extension's integrated journal. Because it must provide integrated base/cell records, a new control stream is not opened under the previous `site-control-v1` name. A base stream must not be provided by merely filtering to a subset of events either.

## Storage boundary

`Engine` performs transactions through `Journaled<R>`. The wrapper tracks actual `put` operations for cells, runs, operations, start attempts, mandates, parts, permits and resources. This prevents a state change from being omitted from the journal if a particular handler forgets to call a separate event function. Overwriting a core entity key with another schema is rejected.

When the same entity changes multiple times in one transaction, one final state is recorded. Original entity changes, idempotency/outbox, control events and current projection commit/roll back together within the original repository transaction. Audit records are not reclassified and counted as control events.

Internal `ControlChange` contains the entity kind, change kind, immutable Record at that point and related evidence IDs. T2 returns the control head from the same transaction after the wrapper finishes control recording. The ack therefore does not reference an earlier head or the position of a later transaction. Duplicate recovery within the same inbox batch does not create a new control change.

Supported entities that already exist when the journal is first initialized are recorded as `SNAPSHOT_SEED`. Historical events are not disguised as new ADMITTED events. The initialization marker is written only once. SQLite schema migration preserves existing audit/evidence/outbox data and does not claim to reconstruct detailed events from before control history existed.

Journal order is DB commit/storage order. Entity order within the same transaction is stable key order, not the physical order of events on equipment. External operating-authority decisions always use the application's current state.

An [initial admission receipt](ADMISSION_RECEIPT.md) is stored together with the ADMITTED control-record location for a new Work. The admission seq is returned by the actual control INSERT and is not overwritten by a later head or native result.

## Boundary between current implementation and public contracts

The currently stored payload is a **typed application state snapshot**. Rereading continuous events and a projection at the same cut from the repository is implemented. The complete `CellJournalRecord`/`SnapshotEntity` wire representation is not yet finished.

Follow-up work needed for public mapping includes:

- Preserving explicit state/provenance such as CellContext mode/commissioning and Block.created_revision.
- Independent change/state/limitations representation for embedded Qualification.
- RunView checkpoint artifacts and activation/slot snapshots are connected to [storage at the same cut](CHECKPOINT_ARTIFACT.md). Migration of earlier journal schemas and the complete public stream remains outstanding.
- Representation that fixes related run/budget/condition information for Mandate/Permit at the cut of that event.
- Case/procedure/clearance/material/change/restart preparation models and projections beyond the currently supported entities.
- Installation-wide access, paged-snapshot lifetime, exclusive cursor/retention/gaps, bounded subscribers and SSE.

These values are not pulled from the current DB later and attached to historical events or filled with arbitrary defaults. Public Journal/Snapshot RPCs are not enabled until the complete mapping exists; R17 is PARTIAL. The current publisher uses producer through as its durable retransmission position and does not subscribe to the public journal.

## Validation

- Fail the projection update after control event INSERT and verify rollback of the core entity/control event/projection together.
- Verify that audit append and user login do not change control seq.
- Verify that lost/duplicate T2 responses still persist only one evidence-related control change.
- Verify that Hold's direct `tx.put` changes are captured in work/resource/cell projections.
- Verify continuous source seq, exclusive `after`, and agreement between snapshot head and event tail.

This is evidence for the storage/capture layer. It does not substitute for the unimplemented public wire mapping or performance validation at complete product scale.
