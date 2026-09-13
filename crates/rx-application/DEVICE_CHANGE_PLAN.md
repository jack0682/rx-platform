# Process change plans and per-Host requirements for device candidates

phase71. Connect independent software approval of a device-candidate process to change proposal, impact review and staging. Actual Host/native settings are not changed, and the existing metadata application API is not used as confirmation of device installation.

## Change proposal

Existing process-change Create can select an approved v2 process review. The worker revalidates the process and all device sources. After calculating the candidate configuration, it creates steps for actual compiled node IDs. Candidates supply not only Intent but also conditions, completion mappings, postconditions, condition revision and handover policy. The before configuration is the existing active configuration.

Recalculate the transitive closure of shared impact from the candidate configuration, including new Hosts/resources. Creation, commit, currentness checks, independent impact review and staging compare the same impact and require access to all impacted cells. Calculating a candidate configuration alone does not change active configuration or Runs.

When device candidates exist, the Change's `host_binding_plan` preserves:

- Installation/cell, device review context and process review version digests.
- Before/after CellConfiguration references, definition/operating envelope/environment/scope.
- The canonicalized Intents and condition ID sets that each Host will actually use.
- Signed device package manifests/signatures and catalog references for the selected actions.
- Other impacted cells that share each Host.

The shared schema is `rx.host-binding-plan.v1`. Limits are 64 Hosts, 4096 Intents/128 conditions/16 device packages per Host, and 1,000,000 bytes overall. Intent digests and package identities are sorted and deduplicated. This artifact is input for S to compare, not an installation command or execution permission.

Digest calculation for existing Changes without device context is preserved. Device plans combine the existing plan digest and Host requirements in a separate domain. Stage revalidates sources and verifies that the recomputed after configuration, step origins and Host requirements match the reviewed values.

## Current blocking boundary

GET blockers display `HOST_BINDING_CHANGE_REQUIRED`. Changes with a device plan are rejected at BeginPreparation so that epoch/fence are not changed first. The shared barrier for Host configuration dispatch and P application also rejects them. An existing Host's process-context APPLIED_UNQUALIFIED receipt therefore cannot be used as evidence of a native device configuration change.

Subsequent work must reconcile actual Host installation identity and journal generations, all managed cells, stopped/support state, change effects and restoration results through a durable procedure. This phase does not claim to implement that application procedure.

## S startup configuration comparison

Follow the [Host configuration inspection specification](https://github.com/jack0682/rx-solutions/blob/codex/initial-draft/runtime/rx-host/HOST_BINDING_INSPECTION.md). The actual product `rx-hostd` compares the P artifact with current/proposed pinned configurations. Inspection results establish software agreement only; they are not an approved Host receipt, and P has no endpoint yet to intake them as application evidence.

The first integration used a fixture with two cells and two different Hosts sharing a resource. The Host whose device changes manages one cell, while access to the impact on both cells remains required. The current JTC/MELSEC package backend's 'one exact cell binding' constraint is not relaxed to pass the test. Actual backend configurations supporting multiple cells on one native Host require separate validation.
