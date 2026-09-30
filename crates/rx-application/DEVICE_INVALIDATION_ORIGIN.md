# Provenance and release of DeviceRestart restrictions

A DeviceRestart block is raised in two places: when a Host's evidence producer is replaced
without a proven P-only restart (`OpenEvidenceProducer`), and when an observation reports a
source generation that differs from the registered one or cannot be ordered against P's clock
(observation ingestion). In the same transaction each affected cell now gets an immutable
`deviceinvalidationorigin` record keyed by the block ID (schema
`rx.device-invalidation-origin.v1`). Blocks raised before this record existed have none and
are never given one. A cell whose epochs or scope vector disagree with its configured scopes
still receives the block but no origin; recording never suppresses the invalidation.

`DeviceInvalidationOrigin` holds the installation and store generation, the cell, the Host,
the block snapshot, the cause, and the cell boundary before and after the block, as the
[RuntimeRestart origin](RUNTIME_INVALIDATION_ORIGIN.md) does. The cause names the generation
being replaced: `PRODUCER_REPLACED` carries the superseded registration's boot and session and
the new ones; `SOURCE_GENERATION_CHANGED` carries the source, the registered generation (if
any) and the reported one. `registration_cell` is the cell of the Host registration that was
superseded. For a neighbouring cell reached through a shared scope or resource it differs from
`cell`, because the Host may have no registration there. `registration_epoch` is that cell's
epoch right after the invalidation; origins are recorded after every closure of the event is
invalidated so this is the post-invalidation epoch. `digest()` uses
`RX-DEVICE-INVALIDATION-ORIGIN-v1`; `read_for_cell` refuses an origin from another
installation or cell, one written under a store generation outside this ledger's restore
lineage, or one whose block no longer matches.

`GET /api/v1/device-restrictions?cell=...` returns the cell's latched DeviceRestart blocks in
ID order with their origin and digest (both null without one), under the same roles and cell
access as `/api/v1/runtime-restrictions`. It records nothing.

## Release through requalification

`Requalification.Begin.device_restrictions` selects blocks by ID and exact origin digest, at
most 128, only in the change's impacted cells and only when the origin's configuration digest
is the change's before or after configuration. A selection makes the request
`rx.requalification-request.v3`; without one it stays v2 and its digest is unchanged. Begin
binds each selected block to the change and job (`changeblockowner.device_origin`,
`devicerestrictionbinding`), and `owned_clear` at activation clears a DeviceRestart block only
when the activating job selected it with that exact binding. A block without provenance, or
not selected by this job, is refused as `FORBIDDEN` at issuance.

Provenance is not permission. Begin, report, decision, issuance and activation each require
that the Host was re-linked after the invalidation, or refuse with `CONTINUITY_UNPROVEN`:

- the Host's current bound link for `registration_cell` was committed at an epoch at or after
  `registration_epoch`, and the registration is of that link's generation (boot, session,
  delivery journal, source generations);
- for `PRODUCER_REPLACED`, that generation differs from the superseded boot or session;
- for `SOURCE_GENERATION_CHANGED`, that link read the source.

The registration's own epoch is not compared: fence acknowledgements advance it within one
generation, including the fences this requalification issues. For the same reason a fence
acknowledgement is not a re-link: a Host that only acknowledged the invalidation fence, with
an unchanged generation, still fails the first condition. Re-linking a restarted Host needs the
explicit [re-admission](../rx-api/HOST_READMISSION.md) of the replaced generation.

The usual requalification still applies: six-area report, independent decision, issuance,
confirmation by every Host and activation; issuance clears nothing. Tests cover recording for
both causes and for a neighbouring cell, refusal before re-link, after a fence acknowledgement
alone, for a stale digest, for a block without provenance and for a v2 request carrying a
selection, and the whole path from a source loss and a Host restart through re-admission,
re-link and activation to the cleared blocks. The neighbouring-cell release itself and a live
Host are not exercised. No equipment, material or physical state is assessed.
