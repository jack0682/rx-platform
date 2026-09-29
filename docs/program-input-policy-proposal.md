# Proposed: bounded Program input selection

Status: **PROPOSED, pure policy primitive implemented; runtime/schema integration is not implemented.**

## Evidence and problem

A public next-job test changed only the Program parameter artifact after completing the first job. Admission refused it because `dispatch.rs` binds the complete configured Intent digest. A second experiment assembled, signed and compiled a replacement process package, selected the current step, and used the correct current binding catalog. Platform review still refused it: `resolved binding differs from signed input or selected cell step`.

This is expected under the current exact-action model. It is not a reason to remove the comparison. The device-plan route can represent other reviewed actions, but it requires Host binding work and is not a ready generic per-job input facility.

## Minimal implemented primitive

`rx-process-contract::program_inputs::Policy` binds one normalized finite Program Intent and a bounded, explicit set of immutable parameter ArtifactRefs. The set includes the default parameter artifact. Its schema, content identity, size and cardinality are checked.

`accepts(template, None, submitted)` preserves exact-intent matching. With a valid policy, only `Body::Program.parameter_set` can differ. Program, target, profile, site, calibration, resource set, timeout, prepare validity, completion and cancellation rules remain bound. A malformed policy is an error, never a permissive fallback.

This primitive validates declared choices. It neither validates artifact bytes nor authorizes a caller. It does not create an operation, issue a permit, mutate configuration, or make the public next-job test pass on its own.

## Required authority and version integration

Before this may be used by admission:

1. The policy must come from the trusted deployment/capability/step definition and be covered by actual configuration qualification. A submitted command cannot bring its own policy.
2. A process package cannot enlarge a selected step's input rights. Its policy/template must match the reviewed step capability, or a separate authorized capability change must introduce that right.
3. Compiler input and resolved-process formats must explicitly opt into the extension. Legacy input without a policy must retain its canonical bytes and exact behavior. An older consumer must reject unsupported new forms rather than ignore the restriction.
4. Every approved parameter artifact must be included in package verification and qualification dependency closure. Hash/schema/size membership is not proof that the bytes are available or semantically safe for a device.
5. Host configuration and qualification targets must contain the same concrete permitted Intent digests that P accepts. Do not broaden only P while the native gate still knows a different set.
6. Frontier/read-model validation must match an observed operation to one approved concrete variant. Do not change a job's source template to match whatever operation happened to execute.
7. Generate shared SDK copies from the platform source only after the version/contract change is explicit. No direct edits to vendored SDK copies.

The initial integration may support a finite predeclared set, which is useful for a reproducible two-job trial. It must be reported as bounded selection, **not arbitrary runtime data admission**. Unknown inputs still require a supported validation/approval path; repeated deployment for every new datum remains a usability problem to measure and address.

## Verification implemented

Tests cover default exact matching, approved alternative selection, changes to every non-input field, unapproved or metadata-forged ArtifactRefs, malformed/duplicate/oversized policies, and stable content-bound policy digests. The existing `rx-process-contract` package suite is run with a private target.

## Still required for the final goal

Wire the approved policy through package/capability review, P configuration and admission, Host binding, qualification dependencies, frontier checks and both external providers. Run real normal/reply-loss/new-boot/next-job acceptance again. Do not count these pure policy tests as that integration.
