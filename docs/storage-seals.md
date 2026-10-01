# Opt-in persistent namespace seals

`SqliteRepository::seal_prefixes` commits a caller's marker with monotonic SQL
prefix fences. The callback runs once; existing fences remain active, and new
fences take effect in the same commit. A callback, DDL or commit failure rolls
back the operation. Prefixes are validated internal Names, not SQL patterns.

Sealed keys cannot be inserted, updated, renamed out of the prefix, or deleted
in `entities` or `control_entities`. Seal rows cannot be updated/deleted by DML;
there is no application unseal API. Sibling prefixes remain writable.

Compatibility: ordinary stores still initialize/upgrade to schema 6. A store
with explicit seals becomes schema 7, and schema-6 readers refuse it. New readers
check the guard SQL and nonempty seal list before using schema 7. Merely setting
user_version to 7 does not create a valid sealed store. Target registration intake
explicitly promotes only its target to schema 8, preserving any existing seals.
This prevents older readers from using partially staged registrations. Schema 8
may have no source seals; when present, their definitions are still verified.
Supervisor enrollment/operational records opt into schema 9. Intake promotion
and later source seals preserve that higher floor; schema 10+ is refused. `open_sealed_existing` additionally requires an existing
sealed source before initialization and never creates/upgrades an unsealed source.
Frozen protocol manifests are unchanged; the generated SDK includes this adapter.

This is a local persistence capability, not remote attestation. It cannot defend
against an OS-privileged replacement of the entire database or its schema. A
source-side registration freeze does not establish target acceptance or process
ownership; the receiving application must verify its configured source and its
own request/authority context. Whole-store restore remains a separate framework
obligation.

Tests cover callback rollback, failure after partially staged fence DDL, raw DML
refusals, sibling/observation writes, reopen verification, and malformed guards.
The coordinated Supervisor scene additionally executes a prior released reader
before and after freezing copies of a real completed registry.
