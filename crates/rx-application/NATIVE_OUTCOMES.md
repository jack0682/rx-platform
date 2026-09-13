# Connecting manufacturer-specific native outcomes to core conclusions

Status: phase62 implementation. Existing single-schema `NATIVE`, `PREDICATE` and `UNOBSERVABLE` forms are preserved. New `NATIVE_OUTCOMES` declares outcome-interpretation rules in an installed/reviewed cell's StepBinding. It is not a path for HTTP requests or Host evidence to assign arbitrary conclusions during execution.

## Responsibilities and data

`rx-process-contract::native_outcome` contains only data types unaware of ROS/device APIs. `NativeOutcomeTable` has schema, profile_digest, completion_rule and cases. Each case maps an exact native status schema and set of integer codes to one of SUCCEEDED/FAILED/CANCELED. P application validates this data and derives conclusions from durable evidence. S supplies manufacturer-specific state vocabulary and code meanings.

Place `CompletionRule::NativeOutcomes { table, postconditions }` in CellConfiguration's StepBinding. The table's profile_digest and completion_rule must match the same step Intent. StepBinding belongs to the existing configuration digest/review/change context and is copied into Work at admission, preserving the interpretation rules for that historical operation. Historical Work is not reinterpreted with a new mapping from current configuration.

A table allows up to 16 cases, 64 codes per case and 128 schema/code pairs overall. Empty tables, empty code sets and duplicate pairs are rejected. Ambiguous declarations are not allowed even if the duplicate conclusions agree. There are no wildcards, range comparisons, default success, string execution or dynamic functions. Without a schema/code pair, no result is concluded.

This mapping cannot create NOT_EXECUTED or UNRESOLVED. Proving nonexecution before native emission and explicit recovery/closure of uncertain operations remain the responsibility of separate existing procedures. In particular, native API goal rejection is distinct from a pre-emission tombstone.

## Durable application and state

Existing T2 Host identity/cell authority, operation/invocation/profile correlation checks, and evidence ID/stream-sequence conflict checks are preserved. Original evidence, continuous cursor, Work result and control events are written together in the existing transaction. Replaying the same batch recovers the same result without creating another conclusion.

Even if the mapping indicates success, postconditions require current condition evidence and continuity. Expired/false/unknown conditions or cell block/epoch mismatch prevent promotion to success. As with existing rules, Intents other than finite actions cannot omit postconditions required for success. Failure/cancellation facts do not require success postconditions.

No terminal result automatically releases resources. Control/material-support handover requires separate evidence and procedures. If a different terminal result is later established for the same operation, the original outcome is preserved and integrity=DISPUTED, disposition=QUARANTINED and cell blocking are recorded. Repeated reads alone do not create a conclusion while the result is unknown.

## S declarations for ROS JTC

S's `ros_jtc::Profile::outcome_table()` builds a table bound to the verified Profile digest and completion rule. The core does not include the schema strings below or ROS library dependencies.

| Native capture schema | code | Conclusion |
|---|---|---|
| rx.ros-jtc.succeeded.v1 | 0 | SUCCEEDED |
| rx.ros-jtc.canceled.v1 | -5, -4, -3, -2, -1, 0 | CANCELED |
| rx.ros-jtc.aborted.v1 | -5, -4, -3, -2, -1, 0 | FAILED |
| rx.ros-jtc.goal-rejected.v1 | 0 | FAILED |
| Other combinations | All | No conclusion; preserve original evidence |

This is an explicit policy matching the current control_msgs5.9.0 code range and phase61 adapter capture vocabulary. Code0 alone is not classified as success. S already rejects capture creation and preserves a dispute for contradictions between ROS success and a nonzero error. Controllers using other codes require separate profiles/review; default failure or success is not guessed.

ROS admission/cancellation-request acceptance is distinct from a terminal result. ROS goal states and result-cache behavior follow the [official action design](https://design.ros2.org/articles/actions.html); error codes follow the [control_msgs5.9.0 source](https://github.com/ros-controls/control_msgs/blob/5.9.0/control_msgs/action/FollowJointTrajectory.action). Neither document establishes a machine's physical state or RX field qualification.

## Compatibility and validation scope

The existing eight normative files and gRPC/Protobuf wire formats are unchanged. The added table is an SDK data type and an internal configuration/Work JSON variant. Existing variant storage formats remain readable. Older binaries are not guaranteed to read databases/configurations containing the new variant; actual release replacement/rollback must use supported-combination checks and backup/restoration procedures. This feature does not implement that deployment procedure.

Tests cover table duplicates/limits/unknown fields/disallowed conclusions, profile/rule agreement at installation, per-schema/code results, source preservation/same batches/pre-commit failure/lost responses/late contradictions, current postconditions and Hold. S tests create native captures through the actual JTC adapter and interpret tables using the same shared types. These validate P and S boundaries separately; they are not integrated acceptance tests from an actual robot through the complete P service.

JTC device package authoring/factory and a resolver automatically connecting P configuration intake remain follow-up work. The current generator's return value must be connected to a reviewed configuration. Actual device Authority, controller-generation fencing, native cancellation reconciliation/recovery, physical calibration/support and field acceptance also remain outstanding. See [device Host documentation](https://github.com/jack0682/rx-solutions/tree/main/runtime/rx-host) for the current integration boundary and the [draft-era record](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/implementation/phase62_checks.json) for contemporaneous test evidence. Historical tests are not promoted into validation of the current vendor-neutral configuration.
