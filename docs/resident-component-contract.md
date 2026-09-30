# Shared resident component identity

This additive Rust contract is the first implementation step for the resident
framework model. `rx-domain::component` owns the value types previously declared
only in rx-supervisor: CatalogReference, Declaration, RegistrationState,
Registration, VersionedRegistration and Binding. The SDK exports the same source.

The existing supervisor module re-exports these types. Registration IDs, catalog
digests, decimal-string revisions, serde field names and Accepted/Retired states
remain unchanged. No base/cell protobuf, normative manifest, persisted document
schema, admission rule or executable permission changes. Compatibility tests
decode published v1 shapes and reject authority fields in registration content.

This extraction does **not** move the authoritative writer to Platform or import
an existing registry. It creates no endpoint and grants no process ownership,
readiness or work permission. The next step must put authoritative registration
transactions in the existing Platform application path and define the migration
handshake with the supervisor; a second independent registration authority is not
an acceptable integration.

Model and completion scope are maintained in rx_docs, documents 45 and
`docs/implementation/resident_framework_completion.md`.
