# RX strict-wire-v1 profile (revision 1)

This is the normative binary-message profile named by the existing Session.Open
`strict-wire-v1` requirement. It specifies additional validation over protobuf,
not a replacement wire encoding. Standard generated messages can conform.
Existing base/cell field numbers and negotiation manifest hashes are unchanged.
The source of this profile and its authored conformance inputs is this directory;
`export_protocol.py` includes its exact bytes in the content-addressed bundle.
Generated language message definitions MUST come from that same bundle's IDL.

## Scope and terminology

A message means one uncompressed, unframed protobuf message, with a known message
descriptor. gRPC compression/framing, mTLS, service negotiation and application
admission are separate. Apply the rules below to received raw bytes BEFORE a
normal parser can discard unknowns or merge singular fields. Apply the same rules
to serialized bytes BEFORE transmitting. A successful decode is not authority,
readiness, a complete business request, or evidence of physical completion.

`OK`, `INVALID_ARGUMENT` and `RESOURCE_EXHAUSTED` below are normative categories.
Diagnostic wording and the first error when several rules fail are not normative.
Test cases isolate the intended rule. A local encoding failure must not transmit
bytes; a decoding failure must not return a usable typed response.

## Rules

- **SW01 Size:** accept at most 1,048,576 bytes, inclusive. Reject larger messages
  with RESOURCE_EXHAUSTED before parsing.
- **SW02 Depth:** root depth is zero; a nested message increments depth by one.
  Reject entry at depth 64 or greater with INVALID_ARGUMENT. Thus the maximum is
  64 message levels INCLUDING the root.
- **SW03 Work budget:** start with 65,536 units shared by all nested messages.
  Each field occurrence consumes one unit; each scalar within a packed segment
  consumes an additional unit. An empty packed segment still costs its field
  occurrence. Reject a required unit after exhaustion with RESOURCE_EXHAUSTED.
  The budget measures encoded parsing work, not semantic element count. Equivalent
  packed/unpacked encodings can therefore have different costs; decoding does not
  promise successful re-encoding under a different representation at the limit.
- **SW04 Varints and bounds:** a varint terminates within ten octets; its tenth
  octet, if present, is at most 1. Reject truncation/overflow. Nonminimal encodings
  are allowed; this is NOT a canonical protobuf byte-string format. Declared
  lengths must fit and be wholly inside their containing message/packed segment.
  Fixed-width fields contain exactly the required four/eight octets.
- **SW05 Fields:** tag zero, unrepresentable field numbers, unknown descriptor
  fields and unsupported/wrong wire types are INVALID_ARGUMENT. Unknown fields
  are not retained or ignored, even when their wire encoding is otherwise valid.
- **SW06 Maps:** reject any encoded occurrence of a map field. The published RX
  RPC schemas define no maps. A descriptor containing an absent map field is not
  by itself rejected; future map support requires a reviewed profile change.
- **SW07 Singular:** a singular field occurs at most once in each message,
  including messages and proto3 optional fields. Equal duplicate values are
  still invalid. Repeated fields may have multiple occurrences.
- **SW08 Oneof:** at most one arm occurrence per oneof; repeating the same arm
  is also invalid. Each NONSYNTHETIC oneof of every encountered message has exactly
  one arm. Synthetic oneofs implementing proto3 optional presence may be absent.
- **SW09 Representation:** double/fixed64/sfixed64 use wire1;
  string/bytes/message use wire2; float/fixed32/sfixed32 use wire5; integer, bool
  and enum varints use wire0. Repeated packable scalars accept both packed and
  unpacked forms, including mixed forms and multiple packed segments. Groups are
  unsupported. Validate every packed scalar using its element type.
- **SW10 Scalars:** encoded bool is 0 or 1. Encoded uint32/sint32 varints fit
  unsigned32 before zigzag conversion. uint64/sint64 follow SW04; fixed integers
  use protobuf little-endian representation. Frozen RPC schemas do not use int32,
  int64, fixed/sfixed integer or float32 fields; guard tests for additional scalar
  kinds do not add supported RPC fields or a new negotiation identity.
- **SW11 Enums:** every ENCODED enum occurrence is a defined positive value at
  most INT32_MAX. Explicit zero and unknown/negative values are invalid. Absence
  of an ordinary proto3 enum is allowed structurally and yields its default zero
  on parsing. Business validation MUST still enforce required enum semantics.
- **SW12 Reals:** encoded float/double values are finite. Reject NaN and either
  infinity, including packed elements. Signed finite zero is allowed.
- **SW13 Text:** strings contain well-formed UTF-8 (no overlong encodings,
  surrogate scalars, incomplete sequences or codepoints above U+10FFFF). Bytes
  fields have no UTF-8 restriction.
- **SW14 Presence:** other than SW08, this profile does not require ordinary
  fields/submessages to occur. Absence, explicit scalar zero and optional presence
  are distinct where protobuf preserves them. Do not add application-required
  fields to structural validation.
- **SW15 Typed parse:** after raw validation, the native protobuf parser must
  accept the complete input. Parser failure is INVALID_ARGUMENT. Preserve typed
  values and presence rather than promising exact serialized-byte roundtrips.
- **SW16 Separation:** wire acceptance does not validate RX Name/Id/Digest alias
  formats, authenticate callers, issue permissions, acquire resources, admit
  operations or infer results. Those remain the resident runtime's responsibility.

## Authored conformance corpus

`vectors.json` contains fixed inputs and expectations authored from SW01-SW16 and
published field numbers. No codec is invoked to generate expected values or bytes.
Hex literals use ordinary protobuf tag/varint/fixed-width arithmetic. Bounded
recipes keep large boundary cases readable: `concat`, `repeat`, `delimited` and
`nest` compose bytes, never call a protobuf encoder or inspect its behavior.

The TCK-only `probe.proto` supplies recursive/map/additional-scalar descriptors
absent from frozen public RPC schemas. It declares no services. Reports MUST
separate public-message checks from synthetic guard checks. Do not advertise
those synthetic messages as runtime methods or expanded contract support.
`roundtrip_status` is the expected result of revalidating the ordinary generated
encoder's output ONLY when explicitly present; otherwise no byte representation
or scalar-default preservation beyond protobuf is demanded.

Keep vectors fixed while changing an implementation in a PRIVATE source/target
copy. A mutant accepting duplicate ordinary singular fields must fail the
corresponding negative vector. Regenerating expectations from the mutant is not
conformance evidence. The source suite and mutation output remain separate.
