# Operating Scope, Conditions, Hazards, and Functions

Normative: RX Cell Operations Contract v1.0 · Original source-selection rationale follows S01–S13/O-D01–11 in the [research report](https://github.com/jack0682/rx_docs/blob/6111a7d1dcf33052f38c3e67c6585aec2b44df3c/references/cell_operation_research_2026-09-10/research_and_decisions.md).

## 1. Responsibilities and cell boundaries

A `Cell` is an operational scope combining hazards, control, and material handover. It is not defined as one device, container, or manufacturer. Installation engineers register devices, command resources, hazard zones, material locations, Hosts, external protective functions, and dependencies in `CellDefinition`. The platform validates this definition and uses it in decisions and records.

| Role | Owned information/actions | Boundary |
|---|---|---|
| Manufacturing team/OEM | Actual device functions, usage limits, native commands/results, stopping/support, entry conditions, validation materials | Guarantees for the particular product/version, distinct from whole-cell validation |
| Installation/process engineer | Mechanics, process, access, combined hazards, control scope, recovery procedures, integration validation | Do not infer meanings absent from supplier documentation from signal names alone |
| Validation/release role | Match assessment/test scope to configuration; review open items, constraints, and operating scope | A document signature alone is not evidence of physical state |
| Site operations/intervention role | Start intent, performing/confirming defined site procedures, personnel/material/change records | Distinguish ordinary notification acknowledgement from safety reset/access procedures |
| platform | Authority for condition decisions, intent, permits, invalidation, intervention, recovery, and history | Do not replace real-time safety-function performance with DB state |
| Host | Predispatch condition/epoch/permit checks, native integration, observations, local reactions | Do not conflate the conclusion ledger's responsibility with physical protective functions |

## 2. Per-installation operating-scope record

This section does not assume a particular customer/product as a premise of the common contract. Finalize the following inputs for each actual installation. Existing simulated-cell and historical industrial-application records are in the [preserved materials](https://github.com/jack0682/rx_docs/blob/main/references/README.md); they do not replace new site validation.

| Item | Scope to define | Inputs required for execution admission |
|---|---|---|
| Use | Installation-specific work such as transport, handover, facility services, and manufacturing | Actual demonstration/operation/teaching scope and operational procedures |
| Facility controllers | Connections such as PLCs, access, elevators, and charging | Programs, communication settings, signal semantics, fault/reboot behavior |
| Equipment and goods | Robots, tools, fixtures, loading, and support configurations | Installed instances, models, versions, calibration, coordinates, and load combinations |
| Work | Acceptance, execution, handover, and final completion conditions | Actual sequence/parallel scope and responsibilities of each participant |
| People | Intervention, access, manual manipulation, and receipt roles | Intervention purpose/location, maintenance, handover, and restart procedures |
| Validation | Physical functions and usage limits of the configuration | Environment-specific validation results, open items, and constraints |

An installation without finalized mandatory inputs/evidence is `NOT_COMMISSIONED`. This describes site-input status, not failure of the software design. Do not fill unknown values with default true or “not applicable.” Do not automatically assign facility-startup responsibility to RX. Also follow the [project scope](https://github.com/jack0682/rx_docs/blob/main/docs/03_product_scope.md).

## 3. Validated operating scope

An `OperatingEnvelope` is an immutable artifact defining usable combinations. It must include:

- Cell/device instances, controller/firmware/driver versions, contract and profile versions.
- Recipe/step semantics, permitted operation kinds/purposes, native-continuation methods.
- Tools, fixtures, calibration, material types/shapes, loads, contact/support conditions, positions/zones.
- Permitted operational/control modes, simultaneous actions, human access/intervention/maintenance scope, required staffing conditions.
- Actually depended-on environments such as floors, slopes, space, and illumination, plus energy/pressure/battery conditions.
- Condition definitions, hazard rows, safety-function specifications, recovery procedures, validation results, and their scopes.

Do not automatically permit the Cartesian product of individual maxima. Separate maximum-load and maximum-reach tests, for example, do not validate their simultaneous combination. Represent permitted combinations through explicit variants/range constraints; reject out-of-envelope or uninterpretable combinations.

`Qualification` records validation evidence, review roles, constraints, and withdrawal information for the envelope and its dependency artifact hashes. States are `DRAFT / IN_REVIEW / QUALIFIED / SUSPENDED / RETIRED`. Transition to QUALIFIED requires mandatory evidence links and review records. If requirements remain unmet, an explicitly reduced envelope unrelated to those items may be newly defined/validated, but the original required scope must not be marked passed. Unfulfilled profiles record validation scope/open items without retroactively widening previously approved operating scope.

## 4. Condition semantics

`ConditionDefinition` fixes ID, revision, applicable purpose/step, original source, evaluation rule, units, validity, and loss impact. Runtime results are `PASS / FAIL / UNKNOWN`, recorded with reasons and evidence.

| Condition kind | Evaluation time | Meaning of loss |
|---|---|---|
| ADMISSION | When newly dispatching an operation | Defer the next dispatch; distinct from maintained conditions of already-started work |
| MAINTAINED | While a particular action/mode continues | Apply the specified local reaction, dispatch restrictions, mandate revocation/intervention policy |
| WAIT_TARGET | Target explicitly awaited by the normal recipe | Not yet reached may be normal waiting. Fault conditions/deadlines are separate |

Condition evaluation uses the version-pinned restricted grammar `ALL, ANY, EQ, RANGE, SET_CONTAINS`. Do not embed arbitrary scripts, network queries, or new native writes in evaluators. EQ compares only matching typed values; floating-point tolerance is expressed as an explicit RANGE. RANGE requires units, lower/upper bounds, and inclusion/exclusion for each bound. Meaningless unit conversion/rounding must not manufacture a pass.

ALL: any FAIL yields FAIL; otherwise any UNKNOWN yields UNKNOWN; all PASS yields PASS. ANY: any PASS yields PASS; all FAIL yields FAIL; otherwise UNKNOWN. Mandatory condition sets always use ALL. ANY is only for validated alternative evidence paths; conflicting evidence takes precedence as UNKNOWN/incident. No rule negates unknown inputs into true. Required opposite values, such as `EQ(false)`, first require source existence/validity.

Observation source/boot/seq/quality/age follow common v1.0 rules. Schema/unit/configuration/generation mismatch, unknown source time, contradictions, or missing required signals produce UNKNOWN. Preserve both FAIL and UNKNOWN reasons. Do not disguise cached GOOD as a new sample.

## 5. Signals, current conditions, and actual protective functions

Distinguish source purposes `PROCESS_STATE`, `SAFETY_STATUS_MIRROR`, `FUNCTION_PATH_SIGNAL`, and `HUMAN_RECORD`. Declaration alone does not confer a safety grade.

- PROCESS_STATE is evidence of process state. Whether “door closed” also includes locking, safe entry, or laser-start permission is a separate matter.
- SAFETY_STATUS_MIRROR is a display/record copy of an actual function. Its GOOD value does not prove that the actual function always operates.
- FUNCTION_PATH_SIGNAL participates in the relevant safety-function path. It requires complete allocation across sensors, logic, communication, outputs, and actuation, with required performance, fault responses, and validation scope.
- HUMAN_RECORD is evidence of a human action/confirmation. It does not replace sensor self-tests or stopping performance.

Do not assume ordinary RPC/DB ledgers alone deliver reactions within the time required for risk reduction. Specify participating components and latency, power, common-cause failures, and resource contention. Unvalidated functions cannot satisfy production/access conditions relying on that risk reduction.

## 6. Impact scope and material

CellDefinition contains dependencies `condition→operation/step`, `resource→related resource/zone`, and `artifact→qualification/condition/recovery`. Compute impact through the closure of these relationships. Allow parallel execution outside the closure only with validation evidence of physical-hazard and control independence. If missing/contradictory definitions make scope unknown, block all new production in that cell.

`MaterialState` records evidence identifying an object/slot, claims about current location, multiple support providers with their evidence, and the last change/intervention generation. Distinguish sensor identification, tracked slots, and human declarations; retain limitations when actual object identity is unvalidated. Combined gripper+chuck support is permitted; a single-owner lock does not replace physical support.

PASS for a support claim is valid only within its material/tool/load/pose/diagnostic/freshness scope. Invalidate it on manual movement, pressure/power anomalies, calibration changes, material replacement, or conflicting related signals. Sender-release conditions include required receiver-support evidence and failure responses; site mechanical/process specifications determine the exact release sequence.

An operation reducing support must include, in addition to controller resources, **the support resource for the same material/handover generation** in the common resource_set. P applies existing T1 global resource CAS to prevent simultaneous reservation of gripper release and chuck release across Hosts. Current receiver PASS alone does not authorize mutual release. Validate through profiles/local protective paths/mechanics that required alternative support remains throughout release; do not release the support resource before confirming outcome/new support state. If it is unknown whether object aliases/slots reference the same physical material, do not allow support reduction.

## 7. First-cell hazard/function/condition cards

These hazard-scenario cards derive from the confirmed task description. No actual risk grades/PLr/pass values have been assigned. Each `OPEN_*` is a commissioning-input requirement, not an execution default.

| Card | Hazard path/required constraints | Allocation/evidence required | Restriction until complete |
|---|---|---|---|
| H01 Access/laser | Radiation, heat, or machining remains active while access is possible | OEM entry-permitted state, guarding, and operation-inhibition functions; installation team's combined-access assessment; OPEN_LASER_SCOPE | No entry/machining integration authorization |
| H02 Automatic door | Door movement conflicts with a person/arm/tool/material in the doorway | OEM door functions, detection/guarding/stopping; installation access/zone validation; OPEN_DOOR_FUNCTION | No automatic door operations |
| H03 Material handover | Sender releases without confirmed receiver retention, causing dropping/entrapment | Mechanical/robot/OEM support/seating/retention semantics and pressure/power-fault behavior; OPEN_SUPPORT | No corresponding handover/release |
| H04 Clearance/machining | Machining starts while arm/tool/material remains | Clearance zones, observation scope, OEM machining permission, installation interference assessment; OPEN_CLEARANCE | No machining-start integration |
| H05 Intervention/restart | Old commands or restored power/communication cause motion during human presence/maintenance | Site access/isolation/personnel checks, native queues, reset/start semantics; OPEN_INTERVENTION | No production resumption/access guidance |
| H06 Loss/stability | Stopping/loss of power removes axis/material/body support | Model-specific local reaction, support, energy specifications/tests; OPEN_LOSS_RESPONSE | No physical operations for that model |

`SafetyFunctionSpec` requires function ID, hazard/operating scope, trigger, allocation of sensing/decision/communication/output/actuation, target state/maintained conditions, required reaction time/performance and sources, fault assumptions/detection/responses, reset/restart, validation methods/results/limits. Without values or supporting evidence, the specification is not QUALIFIED. Merely naming “a safety PLC” does not satisfy all fields.

Require SafetyFunctionSpec for risk-reduction items allocated to control safety functions. Where structure eliminates/reduces risk, link design/validation/residual-risk evidence and record why a control function is inapplicable. This model does not assign arbitrary PLr to every hazard or require safety certification for every ROS package. Do not automatically conclude that usage information/training alone suffices; review the suitability of adopted risk-reduction measures.

Stationary models add axis/tool/material support; wheeled models add position/docking/braking/travel coupled with arm actions; actively balanced models add whole-body support, reaction forces, stopping/recovery space, and energy conditions. Identical stop/Damping/zero-velocity names do not imply identical physical states. Link each support ID at model/mode level under the [device support policy](https://github.com/jack0682/rx_docs/blob/main/docs/13_device_support_matrix.md).
