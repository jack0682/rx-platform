# Store restore and the store generation

The installation record carries a `store_generation`, created once at installation. Every
peer that links to P (Hosts, Executors, operator API clients) presents that generation in its
session hello, and P refuses a hello whose generation differs
(`rx-api/src/grpc/session.rs`). Hosts also pin it in their platform destination and refuse
acknowledgements from another generation. The generation therefore identifies *this ledger
lineage*, and rotating it is how P tells every peer that the ledger they were linked under is
not the ledger P now serves.

`store_restore::restore_store` is the only path that rotates it. It runs offline, on a
repository no runtime has open, after the database file was replaced with a verified backup
(`rx-platformd restore`). In one transaction it:

- checks the restored installation record names the configured installation,
- replaces `store_generation` with a new id and keeps the restored `runtime_boot` as
  `previous_runtime_boot`,
- records `rx.store-restore.v1` (`storerestore/<id>`, pointer `storerestore/current`) with the
  previous and new generation and the backup file's SHA-256,
- emits `rx.event.store-restored.v1`.

It changes no cell, registration, grant, permit, qualification, Run or work. What follows:

- The next `Engine::open` starts a new runtime boot and applies the ordinary RuntimeRestart
  invalidation to every cell, with provenance under the new generation.
- Restrictions recorded before the restore keep their provenance. `read_for_cell` (runtime and
  device origins) accepts an origin written under the current generation or under any
  generation a restore recorded in this ledger replaced, i.e. the generations the restored
  cut's content was written under; any other generation is refused. So the restart and stop
  restrictions a backup carries (an offline backup is always taken from a stopped runtime) are
  released like any others: selected by an explicit requalification after the restore. An
  earlier revision refused them outright, which left every cell of a restored installation
  permanently blocked, since no review path exists for a restriction without provenance.
- Every Host's session open is refused until the operator re-pins its destination to the new
  generation and restarts it; the Host then needs an explicit re-admission of its replaced
  generation ([HOST_READMISSION.md](../rx-api/HOST_READMISSION.md)). Restoring an older cut
  can make P forget accepted work and commits the Host still knows; re-admission's
  executing-Run and unresolved-work checks apply to what P knows, and the Host's own delivery
  journal remains the record of what it did.
- `last_store_restore` returns the most recent record; restores chain through
  `previous_generation`.

Not provided: online backup while the runtime serves (the writer lock is exclusive),
restoring into a different installation id, and any judgement about materials, native effects
or physical state at the restored cut.
