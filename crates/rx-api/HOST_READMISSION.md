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
