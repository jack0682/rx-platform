# Preserving the normative sources

The two v1.0 directories contain baselines copied from the [RX documentation repository](https://github.com/jack0682/rx_docs). They incorporate the vendor-neutral documentation revision of 2026-09-14. Wire numbers and semantic versions are unchanged; document and manifest hashes were updated to identify the new revision. The rationale and compatibility impact follow the [revision record](contracts/v1.0/revision_2026-09-14.md).

Implementation decisions and progress follow the [current implementation documents](https://github.com/jack0682/rx_docs/tree/main/docs/implementation). Matching hashes in this copy alone do not prove implementation conformance. Subsequent changes require an explicit revision of the documentation source and synchronization.

Check: python3 tools/check_contract_baselines.py

The optional Host process configuration context extension is in [host-configuration/v1](host-configuration/v1/README.md). It does not modify the existing base/cell specifications or establish native configuration application or operating qualification.

Optional Host qualification acceptance is in [host-qualification/v1](host-qualification/v1/README.md). Host acceptance is distinct from P-wide activation and a user start.
