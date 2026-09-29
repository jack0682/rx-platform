# Bounded Program input policy — candidate revision 1

Status: candidate implementation contract on the `codex/rx-program-inputs` branches. Not a published release or physical qualification.

A trusted step/action definition MAY carry `program_inputs`. Absence retains exact normalized Intent equality and MUST be omitted from canonical legacy serialization. Presence contains `schema=rx.program-input-policy.v1`, `template_digest`, and a complete `parameter_sets` list of 1–64 ArtifactRefs including the template default. Each reference has the same parameter schema, positive size no greater than 1 MiB, and a unique content digest. Artifact bytes and device semantics require separate package verification and qualification.

Only a normalized FiniteAction with Program body supports this extension. A selected concrete Intent MUST equal the approved template in every field except `Body.Program.parameter_set`, which MUST be an exact hash/schema/size member of the approved set. No program, target, profile, site, calibration, resource, timeout, completion or cancellation substitution is permitted. Caller requests cannot introduce or enlarge a policy.

The selected cell step is the authority source. A signed process binding MUST carry the same policy as its selected step. All choices MUST be in package assets and qualification dependencies. P admission and Host configuration/qualification MUST project the same concrete Intent set. Runtime frontier/read correlation MUST accept only one of those concrete digests; a digest from another template is not valid progress.

Compile inputs containing a policy use `rx.process-compile-input.v3`. Resolved processes containing a policy use `rx.resolved-process.v2`. Legacy compile v1/v2 and resolved v1 MUST reject policy-bearing data, and otherwise retain previous canonical bytes and behavior. The process-source control-flow schema remains unchanged. This extension does not change base Intent protobuf field numbers or grant recovery authority.

Policy authorizes choices; it does not choose a job's input. With no explicit selection, the template is the default. A caller must persist the actual selected Intent with its original request identity. An older reader unable to interpret the new resolved schema must refuse it, not discard restrictions. Policy rights belong in trusted deployment/capability configuration and its review, not an untrusted submission body.

The initial implementation supports a predeclared finite set only. Arbitrary runtime data admission, no-effect recovery, restarted task closure and physical compatibility are outside this extension and remain separate obligations.

Compatibility checks: no-policy canonical round trip; v1 rejection of policy-bearing bindings; v2/v3 acceptance only with valid policy; no policy expansion in review; consistent P/Host digest sets; exact selected-operation/frontier correlation; legacy API and component regression tests.
