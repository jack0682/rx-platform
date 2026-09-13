# Documentary Test Vectors for Canonical Representation

Inputs for subsequent Rust/C++ implementation tests of RX v1.0. SHA-256 values below were calculated from the exact UTF-8 bytes shown. They are not results of executing JCS implementation or protobuf interoperability validation. Numeric fragments are not complete Intents and cannot be admitted. CV01 profiles/timing values are explanatory, not actual device settings.

## CV01 Complete Intent

Canonical payload (no trailing newline):

```json
{"body":{"predicate":{"predicate_id":"fixture.closed","settle_ms":"0","target":{"boolean":true}}},"calibration_digests":[],"cancel_rule":"example/stop","completion_rule":"example/closed","execution_timeout_ms":"5000","kind":"ENSURE_STATE","prepare_validity_ms":"1000","profile_digest":"0000000000000000000000000000000000000000000000000000000000000000","resource_set":["controller/example"],"site_config_digest":"1111111111111111111111111111111111111111111111111111111111111111","target":"example/fixture"}
```

SHA-256 after prefixing ASCII `RX-INTENT-v1` and one LF byte:

```text
e94f6df366963718e61e68018e10b1fb0d453e36143119c1e1b59f07d53ddf92
```

## CV02 Numeric normalization fragment

Canonical payload (no trailing newline):

```json
{"real":0}
```

SHA-256 after prefixing ASCII `RX-INTENT-v1` and one LF byte:

```text
d5dc68a13b5d1ebd2b2ecea05fe2b14022fea142152458f74e0311d21b0306c8
```

## CV03 Explicit false fragment

Canonical payload (no trailing newline):

```json
{"boolean":false}
```

SHA-256 after prefixing ASCII `RX-INTENT-v1` and one LF byte:

```text
f378924571af64a661c8a74c7f675d6c9d64416276deefd8b3cd74297a806e2e
```

## Input pairs requiring separate validation

- Changing CV01 object-key order normalizes to the same bytes. Reject duplicate resources in the set.
- CV02 inputs real=-0.0 and real=0.0 yield the same bytes. Reject NaN/Infinity.
- CV03 explicitly specifies false. It differs from a missing required value arm or null.
- Changes to calibration/tool/profile produce a different intent. With the same key, return KEY_CONFLICT.
- Evidence/Receipt use separate domain prefixes defined in the specification, so even identical JSON is not expected to match an Intent digest.
- Reject unknown protobuf fields, duplicate singular/map fields, and multiple oneof arms before decoding.
- Fixture numbers identify documentary cases, not device-validation certifications.
