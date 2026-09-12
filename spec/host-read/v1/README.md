# Optional Host read binding v1

`rx.host.read.v1` adds one read-only Inspect RPC after the frozen base/cell negotiations. Request keys and entity CAS are absent. The explicit binding hash covers this document, the protobuf and shared DTO. The frozen base/cell manifests are unchanged.

The response contains canonical `rx.host-snapshot.v1` bytes and an exact SHA-256/schema/size artifact reference. It identifies the authenticated Host, physical Host boot, its two journals, cell definition/envelope, current epoch/scopes, persisted resource fence maxima, pending IDs and requested native source observations. It is not a grant, Arm, qualification, native effect or handover certificate.

Each source observation carries its own generation, schema/unit, acquisition time/uncertainty/quality and immutable evidence ID. Host boot must not substitute for a device/source generation. Unsupported native reads return sources_available=false with no observations. The requester must reject missing or substituted sources rather than invent READY values. At most 128 sources/pending IDs and one million payload bytes are accepted.

Metadata is read under the Host gate; native observation callbacks are read-only. The consumer rechecks current context and freshness before registration/admission. TLS connection alone is not device readiness. Production hardware qualification and source mappings remain separate profile obligations.
