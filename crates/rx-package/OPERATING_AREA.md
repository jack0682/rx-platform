# One offline development operating-area judge

`external_decision` owns the existing F6 public transport DTOs, canonical signing
messages and failure vocabulary. It is a move: live receiving Request, Challenge,
Gate, Prepared and VerifiedDecision remain sealed in rx-supervisor. Wire data,
including an unsigned Approve Claim, is not permission. The receiving verification
path still requires an exact live context, author-pinned key, scope, epoch and TTL.

`operating_area::judge` evaluates one independently authored development rule:
only `support-gap-report.v1` for `development/support-area`, role
`work/support-gap-report`, Kind::WorkUse, program `rx/status-work-http`, bounded
requirements (native packages <=10000 and support profiles <=64), matching owner
context and positive TTL <=30000ms. Reporting a shortfall is permitted; operating
inadequate equipment is not the operation being authorized. A rejected rule emits
no signed approval. This is a development policy, not safety, quality, equipment
qualification or the truth of a component's self-report.

The separate one-shot tool is `tools/operating_area_judge`. It is outside the
workspace/SDK and is never started by the host adapter. It reads a dedicated
owner-only development key, checks its public key against the authored literal,
evaluates the rule, and signs through OpenSSL. This key is separate from the G2
release key. No private key belongs in Git, a host image, a build context, or host
configuration. Product authority custody/rotation and real certification remain
unestablished. Public wire types and rule evaluation cannot mint a receiving proof.

The rx-storage mailbox publishes complete immutable files under G1's cooperating
writer lock. The receiver retains that lock from the final revocation read through
F10's physical commit. A cooperating issuer publication therefore cannot interleave
inside this interval. A noncooperating publisher can: this is not a tamper-resistant
filesystem or enforcement over arbitrary external delivery. An already-published
request may await a complete response within its original lifetime; this does not
attempt use or renew its nonce/deadline. A busy mailbox at commit refuses use once.
Stale partial files are refused rather than cleared automatically.

F10's monotonic TTL continues during post-cut commit IO; HTTP data remains an
as-of self-report observation. No network judgment service, background authority
monitor, multi-area routing or physical qualification is provided here.
