# RX v1 IDL

Field numbers, types, optional presence and enum numbers are transcribed from the preserved specification. Semantic field aliases (Name/Id/Digest) remain in semantic_fields.json because Protobuf has no scalar aliases.

Concrete naming decisions:

- Logical services in the tables receive a Service suffix in IDL. Session and Evidence are already message names and cannot also be service symbols in the same package. All C++/Rust clients use the generated service names. Method names and data field numbers remain unchanged.
- Top-level enum constants receive the enum-name prefix required to avoid collisions in Protobuf's package namespace. RX JSON retains the unprefixed contract tokens.
- Cell RPC request wrapper names are MethodRequest; fields follow cell §4's explicit order beginning with call=1.
- CellSnapshot repeats SnapshotEntity, whose oneof is base_entity=1/cell_entity=2. The mixed wrapper is the concrete representation of cell §7's typed snapshot union. The new cell view is never decoded as the old base Snapshot.

These IDL choices are release-level implementation bindings of the document baseline. The document manifest hash is preserved; IDL and generated descriptor digests are separately tracked and tested. No prior deployed wire implementation exists.
