# Platform Host client

This crate converts application DTOs into the frozen RX wire contract and communicates over mTLS. It does not modify the application store itself.

The application first commits an outbox entry, claims emission, sends the request with that stable key, and records the Host receipt in a new atomic transaction. A PREPARED receipt schedules Authorize; a captured Host result alone does not set a platform outcome. Native evidence is ingested through T2 and the configured completion policy.

Run the cross-repository, separate-process integration test with:

    ./tools/test_host_e2e.sh

The script builds a feature-gated simulation Host fixture from rx-solutions and then runs the ignored network integration test with its exact path. The regular workspace test command intentionally does not silently substitute a mock for that process.

The client includes bounded outbox dispatch, existing-invocation reconciliation and durable query-driven resource handover. `Operation.Reconcile` schedules Host reads; it never directly resubmits a production invocation. Three fresh correlated handover facts and the existing application release transaction are required. False observations remain recorded without releasing resources. See [query and handover semantics](../rx-application/RECONCILIATION.md).

Background Evidence.Publish is owned by the solutions Host. Product supervision, strict control-delivery latency, indexed large-journal scheduling and the complete recovery workflow remain pending.

The new Host process-configuration client provides Inspect/Apply/Lookup and strict payload/identity checks. Requests must be durably stored before calls and retrieved using the same request after errors. It follows the [Host contract and limitations](https://github.com/jack0682/rx-solutions/blob/codex/initial-draft/runtime/rx-host/PROCESS_CONFIGURATION.md); the [durable coordinator for P's change ledger](../rx-application/HOST_CONFIGURATION_DISPATCH.md) connects explicit Batch requests, commit before transmission, lookup by the same ID, and receipt preservation. P configuration replacement/revalidation review and [global qualification activation](../rx-application/QUALIFICATION_ACTIVATION.md) are connected.

The optional qualification client provides Inspect/Accept/Lookup for [Host qualification acceptance](https://github.com/jack0682/rx-solutions/blob/codex/initial-draft/runtime/rx-host/QUALIFICATION_ACCEPTANCE.md). This raw transport does not replace P's issuance authority or durable tasks.

`qualification_worker` executes P's durable issuance tasks. Arm block clearance is limited to IDs approved by P and is sent only after checking the current Host block. Renewal of the same lease is distinguished from qualification and Arm.

After a P-only restart, it provides administrator-approved [Host recovery communication](HOST_RECOVERY.md). It checks current transport/source and the original immutable baseline, and retrieves only existing Fence records, receipts, and native results. RECOVERY_ONLY is not permission for operational registration, qualification, Arm, or Run resumption.
