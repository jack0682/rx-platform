# Compiled offline development operating areas

`external_decision` owns the existing F6 public transport DTOs, canonical signing
messages and failure vocabulary. It is a move: live receiving Request, Challenge,
Gate, Prepared and VerifiedDecision remain sealed in rx-supervisor. Wire data,
including an unsigned Approve Claim, is not permission. The receiving verification
path still requires an exact live context, author-pinned key, scope, epoch and TTL.

`operating_area::judge` selects an independently authored development rule by its compiled key identity. The original SUPPORT entry remains:
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
monitor, runtime authority enrollment or physical qualification is provided here.


G4 adds COMPACT: area `development/compact-support-area`, issuer
`development/compact-support-judge-v1`, program `rx/status-compact-work-http`, role
`work/compact-support-gap-report`, key `rx/development-compact-judge-v1`. It has its
own public key and rule, native requirement <=2000, profile requirement <=8 and
TTL <=15000ms. SUPPORT remains <=10000, <=64 and <=30000ms. Both authorize only
inert derived support-gap reports. The tighter rule is a development example,
not evidence of a real operating area's policy or physical qualification.

The compiled declaration list is validated BEFORE building authority maps: 1..8
entries, no duplicate area, issuer, key identity, public key or program. Invalid
lists fail with named `area-catalog/*` conditions. This validator cannot enroll an
area: only the private compiled list feeds product selectors. Generic F6 authored
policies retain their independent 1..8-authority limit and may legitimately bind
multiple keys to one area/issuer. G4 catalog uniqueness does not restrict that API.

The host selects key and role from the sealed owner's program, never a requested
area or response field. Each selected recipe receives only its own authority.
Ordinary catalogs remain without anchors; G3 aliases name SUPPORT for source
compatibility. Catalog programs cannot enter the legacy library fallback. A file,
environment variable or site configuration cannot add an entry or replace a key.
Adding an area requires reviewed source and rebuilt binaries; the trusted Rust/OS
boundary and G2 development release limitations remain.

Distinct keys are an additional boundary, not a claim about first failure order.
The adapter first rejects unexpected key/area/role claims as untrusted input; F6
then checks the exact live challenge/context before cryptographic verification.
An unchanged approved decision cannot authorize the other area. A foreign key
signing the target's exact challenge while claiming its key identity fails the
signature check. Neither public wire values nor historical reports are permission.
