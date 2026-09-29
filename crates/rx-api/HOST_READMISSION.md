# Restarted Host re-admission (unreleased)

`POST /api/v1/hosts/readmission` accepts the usual mutation wrapper with
`{host, previous_boot, delivery_journal, evidence_journal}`. A registered-terminal
ReleaseManager covering every cell the Host is registered for is required.

The body names the generation being replaced, never the new one. P accepts it only
when every registration of the Host has that boot and delivery journal, the bound link
of each cell has that evidence journal, no Run of those cells is executing and no work
of those cells is unresolved or disputed. Only one approval per Host is current; a
second one is refused as busy until every cell has been re-linked.

The approval does not change a registration by itself. The next authenticated Host
link may replace a registration of the named boot when the new boot keeps both
journals. Each cell consumes the approval once; the replaced registration is kept as
history. A reinstalled Host (new delivery or evidence journal), an unnamed generation
and a further restart after consumption stay `CONTINUITY_UNPROVEN`. Reusing a sequence
of the kept delivery journal with different fence content is an integrity conflict.

Re-admission restores no grant, Arm, qualification, permit or Run. The blocks raised
by the restart stay latched, so the cell remains unavailable for work until a
separate resume/requalification. Physical state of the restarted Host is not assessed.

## Binding transition

An approval may also name `binding_intent`, a P-issued Host binding intent whose change is
still staged and prepared and whose cohort is the Host's full registered cell set. Before the
commit is confirmed, the generation being replaced must keep the baseline's delivery and
evidence journals; it may be the baselined boot or a later boot that an earlier plain
re-admission registered after an unplanned restart (the commit read still has to name the
baseline installation identity). After a confirmed commit, it must be exactly the confirmed
generation (same boot and both journals). The re-admitted boot may then present the staged change's after
configuration for that intent's cell instead of the current one: evidence-cell negotiation,
link preparation, link commit and grant renewal all compare against it while the change
stays staged. If the change leaves the staged state without being applied, the current
configuration applies again and a Host carrying the after definition is no longer admitted.

The binding standing of an intent now treats a registration as current only while its
session is the Host's active producer session and its boot is the producer's peer boot, so
a later boot of the Host demotes an earlier commit match until it is re-admitted and
confirmed again.

Re-admission itself does not apply the change, dispatch Host configuration or authorize work. The live
image acceptance with `--binding-commit` approves the re-admission, stops the Host normally,
prepares and commits the Host binding with the P-issued request, restarts the Host on the
proposed startup, and requires METADATA_MATCHED, a refreshed preparation fenced on the new
boot, demotion with refused refresh and configure-hosts when the Host restarts again without a
new approval, re-admission of the committed generation, configure-hosts acknowledged by the
Host, and apply to APPLIED_UNQUALIFIED. With `--host-restart-before-commit` the Host is first
restarted without a commit: a plain re-admission registers it, the read reports
MISSING_COMMIT, refresh is refused, a binding re-admission naming the baselined boot is refused,
and the binding re-admission naming the restarted generation completes the same flow.

When the Host's producer session is replaced by a new boot, the P-side link now drops its
workers and returns to linking (which needs a re-admission) instead of failing the runtime.
