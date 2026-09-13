# P's initial admission receipt

An `ADMITTED` response confirms that P has durably admitted a specific operation. It does not mean Host readiness, native acceptance, completion or resource handover.

## Evidence location

Use the **actual seq** returned when `Journaled` appends a new Work's ADMITTED/revision1 state to the control journal. In the same transaction, record the operation ID, intent digest, operation revision at that point, journal ID and seq in `admissionreceipt/<operation key>`.

This seq is not a later journal head, the internal audit event count, the current Work record revision or an arbitrary constant. The control record referenced by the admission receipt contains the operation's original ADMITTED snapshot. A receipt write failure rolls back together with T1's Work/slot/permit/outbox/request result.

P's control journal ID has UUIDv8 form and binds these values: `RX-CONTROL-JOURNAL-ID-v1`, installation ID, store generation and `site-cell-control-v1`. Hashing uses the existing canonical domain rules. The ID remains the same across a restart of the same P process and changes when the store generation or journal view changes. It is not interchangeable with the Host delivery/evidence journal ID.

## Reads and subsequent state

`Engine::admission_receipt` checks the current session and cell access, then verifies that the Work ID/intent matches the admission receipt. Later Host receipts or completion results do not overwrite this record. The initial admission record remains unchanged after a Hold, authority revocation or subsequent result.

The protocol adapter converts this value into the ADMITTED stage of the frozen base Receipt. It includes the P operation revision and omits invocation/Host state/cancel ID, which do not yet belong to P's admission receipt. The original receipt's journal ID is not replaced with the current process boot ID.

If a Work created before this feature lacks its original admission location, a receipt is not synthesized from a historical head or current snapshot. It remains `CONTINUITY_UNPROVEN`, which is not grounds to resubmit the existing operation under a new ID. New T1 paths persist this receipt together with the operation.

## Tests and limitations

After injecting a failure immediately before persistence and a lost response after persistence, the same request was recovered and checked to leave exactly one admission receipt. Reading the control record at that seq verifies the operation ID/revision/ADMITTED state and the absence of a native invocation. Tests also verify that receipt bytes survive a later Hold and that journal identity follows the rules for P restart/store generation changes.

The public [Cell.SubmitOperation finite-run envelope/Receipt response and Operation.Get](EXECUTOR_SUBMISSION.md) are connected. Base Operation.Lookup and the complete Journal API remain follow-up work. Establishing the admission ledger location is not presented as native result evidence or a complete public journal implementation. Preserving and querying historical journal namespaces/records during store restoration also requires a separate implementation.
