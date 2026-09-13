# Device Bindings and Execution Admission Conditions

Normative: RX Contract v1.0 · Inputs: [Device support policy](https://github.com/jack0682/rx_docs/blob/main/docs/13_device_support_matrix.md), [Image specification](https://github.com/jack0682/rx_docs/blob/main/docs/14_image_support_spec.md)

## 1. Role of a profile

The common contract defines completion, uncertainty, and authority semantics. A BindingProfile connects those semantics to **a specific model, mode, controller, firmware, calibration, and native API**. Do not combine different controller configurations under one product-family name.

A profile is an immutable artifact referenced by its content digest. Profiles missing required fields remain in their validation backlog and cannot be used for operational admission. A matching model name with different controller/firmware/revision requires a separate profile.

| Required item | Details |
|---|---|
| identity | support_id, manufacturer/model/hardware_revision, firmware range, driver/controller versions and source commits |
| native interface | Endpoint roles, native API/action/message types, units and joint/channel ordering, return-value semantics, native-ID/result-lookup availability |
| dependencies | Required packages, transitive dependencies, image variants, kernel/device/GPU constraints |
| capability | Provided kinds/bodies, observation schemas, completion/failure/cancel evidence, distinction between current-state and historical-result lookup |
| resource_set | Conflict units such as controller, mode, bus, shared gripper, and space |
| startup/shutdown | Effects of prepare/activate/destructor on torque, position, and mode; actual init flags |
| calibration | Base/tool/fixture/gripper/zero calibration IDs, scope, and validity conditions |
| mode/authority | Required modes, confirmation of remote/auto/manual transitions, exclusion conditions for direct native clients |
| completion/cancel | Predicates, correlation, tolerances, settling, timeouts, stop results, and release conditions |
| timing | Basis for observation age; grant TTL, ticket/deadman, prepare validity, execution, and shutdown values with supporting evidence |
| restart | Boot detection, native-history retention, residual commands, resynchronization, and unobservable-reboot limitations |
| evidence | Required evidence schemas, permitted human-attestation scope, contradiction decisions |
| validation | Literature/laboratory/site findings, actual test artifacts, dates, versions, conditions, and limitations |

Do not promote code defaults for time/speed/torque into safety criteria. A source-code update rate is not a real-time performance measurement. Recheck the same timing conditions when CPU/GPU variants change.

## 2. Application by device capability and control configuration

The following are **configuration types** to review when authoring a binding. They are not a support matrix claiming any particular device is implemented/validated, nor a requirement to include every type in the default image. Declare actual support per profile under the [device support policy](https://github.com/jack0682/rx_docs/blob/main/docs/13_device_support_matrix.md).

| Configuration type | Contract mapping | Required checks/limits |
|---|---|---|
| Stationary arm with independent gripper | FINITE_ACTION and separate gripper action | Actual joint/channel ordering, tool/calibration/completion/cancel/tolerance, shared bus and resources |
| Arm/gripper sharing a controller | FINITE_ACTION or CONTROL_SESSION | Do not assume logical functions have independent controllers. SC14 serialization or validated compound trajectory |
| Leader/follower control | CONTROL_SESSION and LIFECYCLE | Source identity, deadman, authority handover, activation torque effects, and support |
| Multiple arms/hands/head/lift | FINITE_ACTION and MODE_TRANSITION | Controller/resource sharing; distinguish command entries from independent actuator counts |
| Mobile robot | CONTROL_SESSION and separate navigation/docking operations | Distinguish velocity acceptance from location arrival, braking, and docking completion; coupled constraints on travel and arm motion |
| Actively balanced robot | MODE_TRANSITION, CONTROL_SESSION, LIFECYCLE | Pose, stability, contact, support, energy; distinguish mode acceptance from physical postconditions |
| Door/elevator/charger/facility controller | ENSURE_STATE or FINITE_ACTION | Actual-state feedback, access authority, occupancy, handover, and races with stopping/reboots/direct manipulation |

Review each driver's activate/start/stop/destructor effects on equipment. Where shutdown may disable torque, stopping the driver process cannot implement a generic hold. Deployed profiles include source commits and **expanded values** of launch conditions and init flags.

## 3. Representative native mappings

**JTC**: Map ROS `FollowJointTrajectory` goal UUIDs to RX invocations. Preserve action acceptance, action results, and joint observations separately. Pin the controller's success scope and tolerances in the profile. Joint target attainment alone does not establish a successful material grasp. Cancel by specific goal ID and confirm action results plus actual stop/support conditions. Do not present topic publishing as though an action result exists.

**Mode services**: SetMode success means the acceptance/change result documented by that API. Check a fresh active mode in ModeStatus and additional postconditions. After commanding ReadyPose, do not infer completed robot posture from the mode name alone. Processes requiring an established pose use a separate ENSURE_STATE or an explicit mode completion rule.

**Velocity/leader/policy control**: Fix CONTROL_SESSION source identity, sample schema, mode, deadman, and expiry reaction. Navigation completion requires an additional FINITE_ACTION profile for location/docking operations; it does not mean the same thing as `cmd_vel=0`.

**Driver startup/shutdown**: Image health=software ready does not authorize device activation. If prepare also writes torque/initial pose, authorize it first as a LIFECYCLE operation. If upstream initialization consists of multiple native effects, provide a binding that separately records the sequence's steps, outcomes, and recovery. If that is impossible, declare an explicit compound native action where “1 native call=multiple internal effects,” allow UNKNOWN partial outcomes, and do not claim per-step exactly-once execution.

## 4. Facilities, PLCs, and existing operational systems

| Target | Scope the binding must verify | Assumptions that must not be declared |
|---|---|---|
| PLC/facility controller | Command/result identity, actual feedback, communication settings, I/O semantics, boot, residual commands | Treating communication availability or a successful memory write as operation completion |
| Door/elevator/handover device | Access authority, occupancy/current state/material support, direct human intervention, and recovery | Inferring safety interlocks or actual sensor coverage from function names alone |
| Robot or existing fleet manager | Acceptance, execution, result lookup, cancel, ownership, and material-handover semantics | Treating an upper system's accepted response as final service completion |
| Auxiliary controller | C08 device capabilities, observations, boot, and command boundaries | Claiming an independent RX Runtime or distributed authority is validated merely because it is connected |

One possible PLC handshake is `request_id/command/parameters → accepted_id/busy/result_id/result_code + actual-state feedback`. Profiles must verify what the actual device provides. Without numeric IDs, document edge/level handshakes, exclusive execution, and ready/busy/done reset order. Historical-result correlation after restart may then be limited.

`door_closed` or `chuck_clamped` must be actual-state feedback, not command bits. Successful memory writes, healthy communication, and ON output contacts are insufficient for physical completion. Confirm the actual sensor scope, mechanical meaning, and diagnostic conditions from supplier materials and physical validation. RX DB approval alone does not replace facility/door/gripper safety interlocks.

## 5. Admission and operational configuration transitions

Configuration lifecycle is `DRAFT → VALIDATED → RELEASED → INSTALLED → READY_FOR_OPERATION`. READY_FOR_OPERATION is a decision about current device conditions, not a permanent state. Release signatures/technical validation are review-role evidence; they do not replace site calibration, modes, or observation freshness.

1. Validate profile/site-configuration digests, required sources/artifacts, and image variants.
2. Compare actual Host device/controller/firmware/mode with the profile.
3. Check calibration, tool/fixture, material state, human-intervention policy, and operation kind.
4. Check required evidence sources and native-result correlation/restart/cancel capabilities.
5. Reserve conflicting resources and acquire a Host grant. Immediately before dispatch, the Host rechecks conditions that may change.

Failures structurally identify support_id and missing conditions; do not collapse everything into “unsupported.” ProfileFinding FindingCode is fixed to 0=UNSPECIFIED, 1=SATISFIED, 2=PACKAGE_MISSING, 3=MODEL_MISMATCH, 4=MODE_MISMATCH, 5=CALIBRATION_INVALID, 6=EVIDENCE_UNAVAILABLE, 7=TIMING_UNVALIDATED, 8=AUTHORITY_UNAVAILABLE. Do not parse it from RPC Reason detail strings.

## 6. Additional requirements for the two image specifications

| Image | Additional obligations |
|---|---|
| rx-platform | Durable state volume, backup/restore generation, trusted release/profile cache, certificate mounts, journal cursor API, STORE_FAULT diagnostics |
| rx-solutions | Durable inbox/evidence-outbox volume per Host; immutable binding/profile and calibration mounts; separate robot-source/stream processes; least privilege for native devices |
| Both | Software restart restores diagnostic readiness. Do not tie automatic operation resumption, torque activation, or new-grant issuance to a successful healthcheck |

Keep Host volumes separate from the platform DB, and record journal IDs/peer identities in backup manifests. Unconditional `privileged` or full `/dev` mounts are not default contract requirements. Actual device/network/RT permissions are established in model/variant-specific tests.
