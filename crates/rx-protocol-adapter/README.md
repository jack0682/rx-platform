# Application ↔ protocol conversion

`rx-protocol-adapter` owns conversion between Protobuf and application models. Generated wire types do not belong in `rx-domain`/`rx-application`. `Evidence.Publish` received by P and responses to P's `Host.Reconcile` requests use the same `evidence_batch` conversion.

All required and optional fields of the current native-result source are preserved. Device-specific native_id and native_data ArtifactRef are also retained in `NativeEvidence.native_details`. These fields participate in immutable evidence comparison, so changing only an attachment at the same journal/seq is still a conflict.

Historical internal projections with `native_details=None` do not claim to preserve the complete source. Attempts to reconstruct them as complete original wire data for `Evidence.Get` are rejected. Missing information is not invented, and existing records are not overwritten. Reading the new internal fields with older binaries and rollback require separate compatibility review.

The currently supported external evidence type is `rx.native-result.v1`. Other EvidenceBody kinds are rejected rather than reduced to NativeResult or discarded. Formats and authority integration for observation/attestation/cancel evidence remain future scope.

Two unit tests verify lossless round trips of native_id/native_data and uint64 values beyond JavaScript integer precision, and rejection of legacy projections masquerading as complete originals.
