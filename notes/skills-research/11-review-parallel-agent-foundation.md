# Review: parallel-agent foundation

Status: review of `AGENTS.md`, `05-parallel-agent-execution-protocol.md`, the three
repository Rust/testing skills, and the canonical execution specification
(`18-production-readiness-spec-reviewed.md`), 2026-08-23.

## What is already sound

The documents consistently make `PLAN-001`/its generated manifest the scheduler, forbid launch
before `ready`, pin profile/evaluated predecessors/base/tree/config/toolchain/image identities,
assign one owned boundary in an isolated worktree, require independent review, serialize
integration, invalidate dependent evidence, and distinguish mergeability from qualification.  They
also consistently reject skipped/filtered/under-run/retry-laundered evidence and replace
calendar-duration monitoring with finite operation budgets, event floors, named cuts/barriers, and
diagnostic deadlines.  The Rust and testing skills reinforce rather than weaken those rules: their
TDD, exact-identity, real-collaborator, bound, and honest-blocker requirements are compatible with
the protocol.

## P0 — resolve before enqueuing implementation agents

### 1. `PLAN-001` cannot receive the required launch packet

`05` sections 1–2 require every launch (including the sample `PLAN-001` packet) to pin a selected
profile manifest and its blob/hash.  But the canonical specification says that
`docs/production/release-profiles.yaml` is created by `GOV-005`, which itself depends on `PLAN-001`.
`AGENTS.md` simultaneously says `PLAN-001` is the only initially merge-ready task.  Thus the one
task allowed to launch has an unproducible required packet input.  Treating a placeholder or a
future manifest as pinned would defeat the no-`TBD` rule and can falsely authorize the first fanout.

Minimal edit: add a discriminated, `PLAN-001`-only bootstrap packet profile to `05` and the
`AGENTS.md` summary.  It must pin the canonical-spec blob/tree and declare `profile_scope:
uncompiled_all`; it must not claim a release-profile hash or allow any other task to reserve.
Require `PLAN-001` to replace that bootstrap identity with the generated release-profile
manifest/blob/hash before emitting its first non-bootstrap ready set.  Make the sample packet use
that form rather than the nonexistent `GOV-005` artifact.

### 2. The packet commit topology contradicts the required one-agent red/green pair

The packet schema sets `candidate_identity.expected_head_parent` to `base.source_sha`, while `05`
section 5 requires a one-agent behavioral packet to create a red commit and then a green commit.
The green candidate's parent is necessarily the red commit, not the base.  The same field would
also reject a legitimate multi-commit task-scoped candidate.  An executor must either collapse red
and green into one commit (destroying independent red provenance) or ignore a supposedly required
packet field.

Minimal edit: replace `expected_head_parent` with explicit immutable topology fields, for example
`initial_head: <base>`, `required_merge_base: <base>`, `red_patch: <sha | not_required>`, and
`candidate_must_descend_from: <red_patch | base>`.  State that the reviewer verifies the complete
first-parent range from base through red to candidate, and that the final candidate may have the red
commit as its parent.  Do not make a red patch optional for a behavior-changing implementation
packet once that packet is handed to an implementation agent.

### 3. Evidence and branch identities collide across profiles and attempts

The protocol models state per `task/profile` pair, yet author/review branch names and the durable
handoff are only `agents/<task-id>/a<attempt>`, `review/<task-id>/a<attempt>`, and
`notes/execution/<task-id>.md`.  Tasks declared for all profiles can therefore reserve the same
paths/branch and overwrite or merge-conflict their evidence; a retry for an already integrated
profile can do the same.  That undermines worktree isolation and lets the wrong profile's note look
like a completion record.

Minimal edit: make every mutable control and handoff name include the immutable profile identity
and attempt, e.g. `agents/<task-id>/<profile-hash>/a<attempt>` and
`notes/execution/<task-id>/<profile-hash>/a<attempt>.md` (or an equivalently injective flat name).
The manifest must declare whether a profile-independent source task is a single shared producer
with multiple qualification consumers, or a separately executable task/profile pair; it may not
reserve both forms concurrently.  Update every `notes/execution/<task-id>.md` reference in `05`,
`AGENTS.md`, and the specification together.

## P1 — fix before relying on concurrent waves as an operational control

### 4. Stale invalidation has no atomic revocation mechanism for active reservations

`05` correctly says every changed base/input invalidates packets and section 8 regenerates the
ready set after each merge.  It does not say how an already running agent is atomically removed
from `reserved`/`red_proved`/`implemented`, notified, and prevented from handing a stale candidate
to review.  Section 9 tells the agent to stop when its base differs, but provides no control-plane
generation/lease to observe that difference.  With many long-running agents this becomes a race,
not an enforceable invalidation rule.

Minimal edit: add a scheduler generation and a reservation lease ID to the packet; require an
atomic compare-and-swap of `(task, profile, attempt, integration_head, generation, lease)` at
reserve, review-admission, and integration-admission.  On an accepted merge, revoke all affected
leases and notify/cancel their agents; a reviewer/integrator must reject a candidate whose lease is
not current even if its tests are green.  Record the revocation in the attempt-specific execution
note as `invalidated`, not `pass`, `fail`, or `blocked`.

### 5. The canonical specification permits rebasing evidence that the protocol forbids reusing

Specification section 13 says the integration operator reruns direct gates “after rebasing.”
`05` sections 2, 8, and 9 instead require a new attempt/packet whenever base/tree/predecessor
changes and say agents never rebase or continue stale evidence.  A literal rebase by integration
would create unreviewed candidate bytes and make red/green/raw evidence refer to the wrong SHA.

Minimal edit: revise the specification's phrase to: “after integrating a candidate whose packet is
pinned to the current integration head, rerun direct gates at the resulting integration SHA.”  If a
merge conflict or changed head requires a rebase, invalidate the old candidate and issue a fresh
packet/red-or-green replay as applicable; do not use integration-side rebasing to preserve old
evidence.

### 6. “Red” needs an explicit non-red characterization state

`05` requires every red patch to fail at the intended assertion and stops an attempt whose required
red test does not.  In contrast, `E2E-001R` and `E2E-002R` in the specification allow a scenario to
be either an intended current-product failure **or an inherited passing control**.  The latter is
valid characterization, but it is not genuine-red provenance.  Without a typed distinction, an
agent can mark a passing control `red_proved` and falsely satisfy the red/green state machine.

Minimal edit: add a packet/registry `proof_kind` (`genuine_red`, `inherited_control`,
`non_behavioral`) and a distinct scheduler state such as `characterized`.  Only `genuine_red` may
enter `red_proved` or authorize an implementation green pair.  An inherited control must record
its current-candidate pass and may be consumed only as an unchanged baseline/control under the
scenario's explicit specification rule.

## P2 — remove ambiguity before automation turns policy into accidental authorization

### 7. Commit authorization is inconsistent with the repository Git policy

The task envelope tells every author to commit the execution note, while `AGENTS.md` says not to
commit or push unless the user explicitly authorizes it.  A packet does not currently contain a
recorded authorization.  This will either stop a correctly scoped agent at handoff or encourage it
to treat any internal packet as user authority.

Minimal edit: add an immutable `git_authorization` reference to a launch packet (request/approval
identifier, permitted actions, and destination branch), and say that a task-scoped commit is allowed
only when that field is present.  Keep push and integration separately authorized.

### 8. The final task outcome vocabulary should include `invalidated`

The scheduler state diagram has `invalidated`, but the author handoff and `AGENTS.md` terminal
vocabulary only list `pass|fail|blocked`.  A stale, revoked attempt should not be forced to report
`fail` (which implies an acceptance failure) or `blocked` (which implies an external dependency).
It also must never be read as `pass` merely because its local gates completed.

Minimal edit: make the execution-note terminal field `pass|fail|blocked|invalidated`, reserve
`pass` for integrated acceptance evidence, and require an invalidated note to name the superseding
integration generation/packet.

## Decision

**REVISE.** Do not enqueue high implementation agents until P0 is fixed.  The foundation is
otherwise directionally consistent and its finite-qualification/no-calendar rules are concrete;
P1 should be completed before any concurrent wave is treated as safely self-governing.

## Re-review — 2026-08-23 (v2 protocol)

Re-reviewed the current `AGENTS.md`, canonical note 18, and v2 of `05` against every finding above,
including the bootstrap, early shared work, proof topology, identities, leases, unrelated-merge
preview, Git authority, baseline gates, and terminal outcomes.

### Resolved from the first review

- **Bootstrap deadlock:** resolved.  `PLAN-001` has a discriminator with a pinned
  `uncompiled_all` planning identity and exact future outputs; note 18 and `AGENTS.md` prohibit it
  from fabricating a `GOV-005` profile or `E2E-000S` registry.  The validator must materialize the
  planning/task-scope identities before any non-bootstrap packet, while selected-profile and
  genuine-red work remain gated on `GOV-005` and `E2E-000S` respectively.  This permits the early
  shared non-behavioral `GOV-001`/`TST-000` class without authorizing behavior work prematurely.
- **Red/green topology:** resolved.  The v2 packet pins `initial_head`, `required_merge_base`, red
  patch/evidence, first-parent range, and `candidate_must_descend_from`; it expressly permits the
  green candidate to descend from the red commit.
- **Injective task identity:** resolved.  Task/scope/attempt names now cover author/review branches,
  leases, and execution notes, and the manifest must choose either shared producer or per-profile
  execution, not both concurrently.
- **Stale work and unrelated merges:** resolved in principle.  Lease/generation CAS checks, explicit
  revocation records, observed read sets, and a no-rewrite merge-preview path prevent a stale green
  candidate from being silently merged.  The preview preserves reviewed commit SHAs, refreshes
  evidence/review, and never reuses qualification evidence.
- **Proof kinds and terminal vocabulary:** resolved.  `genuine_red`, `inherited_control`, and
  `non_behavioral` have distinct states; `invalidated` is terminal and `pass` is controller-only
  after accepted integration.  The baseline-phase rule preserves blocked qualification without
  laundering it into promotion evidence.
- **Git authority:** resolved for committed work.  `prepare_patch`, `commit_only`, and delegated
  integration are explicit; a one-agent red/green flow requires authorized commits, while push and
  integration remain separately authorized.

### Remaining P1 — protect reviewed `prepare_patch` handoffs from substitution

`prepare_patch` is deliberately the default, and section 10 permits integration after review accepts
an “authorized patch.”  Unlike the committed path, however, the v2 topology records only
`candidate: <patch-id>`: it has no required content hash, canonical patch format, base binding, or
review-to-apply byte check.  A patch can therefore change after independent review and before the
authorized integrator applies it, even while integration HEAD remains unchanged.  This bypasses the
otherwise strong immutable-candidate/first-parent guarantees for early shared non-behavioral work.

Minimal edit: make a prepared handoff an immutable, content-addressed artifact before review:
require `prepared_patch_sha256`, canonical patch format, base/tree hash, changed-file manifest, and
raw evidence hashes in the packet/note.  The reviewer signs or records that exact hash.  The
integrator must verify the bytes and clean apply against the pinned base before making the authorized
task commit; any byte/base/apply difference invalidates the attempt and requires fresh review.  Keep
the existing rule that a base advance invalidates an uncommitted patch rather than admitting it to a
merge preview.

### Remaining P1 — make gate phase assignment validator-owned, not packet-writable

The v2 packet helpfully separates `author_direct_gates`, `merge_direct_gates`, baseline exceptions,
and final qualification, and note 18 says `PLAN-001` records gate phases.  It does not yet require
the validator to derive and reject the phase assignment for every task from a versioned gate-policy
manifest.  The example permits empty arrays, and a packet author could label a required
engine/external/browser gate “inherited” or qualification-only without a machine-detectable breach.
That would allow an otherwise green direct-gate handoff to falsely look mergeable.

Minimal edit: have `PLAN-001` generate an immutable per-task gate matrix with gate ID, phase,
applicability expression, command/config identity, allowed baseline assertion (if any), and owning
task.  Packets reference that matrix by hash and may only add stricter gates.  Validation must reject
an omitted/moved gate, an inherited result without the exact `TST-000` observation hash, or a
baseline-repair exception outside its declared owner/assertion.  Review and merge admission compare
the packet's resolved matrix to the generated one.

### Remaining P2 — make controller-written terminal receipts mechanically unambiguous

The documents correctly reserve terminal `pass` for the controller after integration, but the same
scope-qualified execution note is also an author deliverable and may already be in the reviewed
task commit.  They do not specify whether the controller mutates that tracked note after merge,
writes a separately committed acceptance receipt, or stores the final state only in the scheduler.
That ambiguity conflicts with the no-rewrite rule and leaves a tooling author free to make the
terminal status look authoritative too early.

Minimal edit: define the author note as immutable handoff evidence with a nonterminal state, and
store controller-only terminal status in a distinct append-only, content-addressed scheduler
acceptance record (referenced by the note/manifest).  If a repository receipt is required, make it a
separate operator-owned acceptance commit after the task commit, with its own hash and authority;
never amend the reviewed candidate or let the author set `pass`.

### Re-review decision

**REVISE.** The prior P0/P1/P2 issues are materially addressed, and there is no remaining bootstrap
deadlock or unsafe red/control/profile fanout on the committed-candidate path.  Complete the two P1
identity/validator controls before using the default patch-handoff or concurrent waves as a
self-governing production-readiness workflow; the P2 receipt clarification should land with that
automation.
