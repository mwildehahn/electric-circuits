# Foundation red-team review

Status: **REVISE**. Reviewed the uncommitted foundation set on 2026-08-23 against the repository
at `474577a088b95c746bd9ab2c8e4b6552a72f151f`, current CI, Compose files, and the new skills. This
is a review of the foundation's claims and execution rules, not an implementation plan.

The good news is that the documents clearly distinguish the current PG16/direct-engine development
system from the intended PG18/gateway profile in many places. The defects below are the few places
where that discipline is not yet executable, or where a rule would prevent the foundation from ever
getting past its own bootstrap.

## P0

### P0-1 — The scheduler bootstrap is circular

`PLAN-001` is the only initially mergeable task, but its packet example requires
`docs/production/release-profiles.yaml`, a selected profile hash, and scenario hashes. That path does
not exist in this tree. Note 18 assigns the release-profile file to later `GOV-005` (which itself
depends on `PLAN-001`), while `E2E-000S` is later responsible for the scenario registry and contract
hash validation. A rule that makes a packet with a missing profile or hash unlaunchable therefore
makes `PLAN-001` unlaunchable. The same contradiction appears in AGENTS.md's claim that PLAN-001
already generates and validates the checked-in manifest.

**Precise foundation fix:** define an explicit bootstrap mode. `PLAN-001` must own and validate a
checked-in *planning input* (graph schema, canonicalization/version rules, and a profile-independent
bootstrap identity) without claiming it is a selected release profile. Its packet must use that mode,
not a nonexistent `release-profiles.yaml`, and may contain an explicit typed
`scenario_registry: bootstrap` only for the registry/planning tasks. `GOV-005` then creates selected
release profiles, and `E2E-000S` creates the populated scenario registry; only packets after both
may require a selected profile hash and registered semantic hash. State the allowed bootstrap
exceptions in note 18, note 05, and AGENTS.md so “no TBD” remains strict everywhere else.

### P0-2 — Exact-base invalidation makes the advertised parallelism self-cancelling

The protocol pins every packet to the exact integration tree and says any source-tree change
invalidates it. It also serializes every integration and instructs the operator to invalidate every
downstream record whose source tree changed. Since any accepted task changes the integration tree,
the first merge invalidates every concurrent, disjoint candidate. Those authors must re-create their
worktrees and evidence, so the proposed parallel ready set cannot actually make progress together.
This is more restrictive than required for semantic safety and will create churn rather than prevent
it.

**Precise foundation fix:** split invalidation into (a) hard invalidation for a changed declared
predecessor, allowed-path/read-set, semantic resource, contract/schema, profile/config, image, or
toolchain; and (b) integration refresh for an unrelated ancestor change. For (b), preserve the
candidate and red evidence, require an automated intersection check plus rebase/replay of the direct
gates on the new integration head, and have the reviewer reject if that refresh changes the contract
or observed result. Keep one-at-a-time integration, but do not discard independent work merely
because its base is now an ancestor rather than HEAD.

### P0-3 — Required per-engine-task gates deadlock the baseline-repair path

AGENTS.md and note 18 require every engine-touching task to finish the full current suite, including
the external Electric oracle. The checked-in baseline says Vitest deterministically fails until
`ENG-014`, and that `mix` is unavailable for the external oracle. `ENG-014` is engine-touching, so it
cannot be marked pass while the baseline it is meant to repair is red and the missing external lane
is blocked. `TST-001`, which is meant to establish the inherited baseline, depends on `ENG-014`; and
the hermetic external-lane task `TST-005` depends on `TST-001`. This is a real dependency/gate
cycle, not a useful zero-waiver rule.

**Precise foundation fix:** distinguish author merge evidence from release qualification evidence.
For a named baseline-repair task, permit the recorded base/red failure only for the exact assertion it
owns, require its green candidate to clear that assertion and all runnable direct gates, and record
the unavailable external lane as blocked. Move installation/hermetic availability of the external
oracle before any rule that makes it an author-level prerequisite (or make it a final
`COMMON_SERVER_QUALIFICATION` gate owned by `TST-005`). Do not permit promotion while it is blocked;
this changes sequencing, not the release bar. Apply the same phase distinction to browser evidence
that cannot exist before the gateway/product lane exists.

### P0-4 — `SourceCommitID` is not an executable causal fence yet

The E2E rules correctly require the sentinel to be last in the mutation transaction, but never state
how `server.drainedThrough(SourceCommitID)` observes that sentinel in the target PG18 topology. An
explicit production publication can omit the sentinel table; querying it after commit only proves
Postgres committed, not that the replication pipeline processed prior rows or all deferred work. This
leaves the central receipt both implementation-dependent and vulnerable to false green tests.

**Precise foundation fix:** make `E2E-000A`'s contract say that the test-only marker is carried by a
defined, publication-admitted harness relation (or by a separately specified transaction-metadata
channel with an ordering proof). Specify its lifecycle: it is included in the immutable test
publication, hidden from public templates, decoded after the transaction's terminal envelope, and
the server receipt is emitted only after all work causally preceding that marker—including deferred
work—has completed. Add negative harness mutations for an unpublished marker, a marker observed
before transaction end, and a receipt that skips deferred work. The public client still need not see
the marker.

## P1

### P1-1 — The parallel protocol grants commits despite the repository Git policy

AGENTS.md says not to commit or push without explicit user authorization. In contrast, note 05 says
the agent “commits its durable handoff,” describes task-scoped author commits as mandatory, and tells
the assignment envelope to commit the execution note. A task packet or branch name is not user
authorization, so the lower-level protocol accidentally broadens authority.

**Precise foundation fix:** replace imperative author-commit language with “prepare a handoff patch
and execution note.” State that an author may commit only when the user or delegated integration
operator has explicitly granted that authority; otherwise the integration operator performs the
authorized commit after review. Retain the no-self-integration rule and make the protocol explicitly
defer to AGENTS.md's Git Policy.

### P1-2 — AGENTS.md's new target policy is still labelled as as-built in its own terms

The opening rule says AGENTS.md describes as-built behavior unless a paragraph explicitly says
“target.” Yet the new Contract-first, parallel-execution, and PG18 sections use present-tense
requirements such as “Production acceptance crosses real PostgreSQL 18” and “Launch only task/profile
pairs emitted ready,” without marking the sections as target policy. The repository instead has
PG16 Compose, host-selected CI PostgreSQL, direct routes, `NoTls` SQL connections, and a current
engine that creates a `FOR ALL TABLES` publication and slot.

**Precise foundation fix:** add one unambiguous scope sentence at the start of each new policy
section: it is a target implementation/qualification rule and does not describe current support.
Keep a short current-state exception list beside it. This is clearer than relying on an earlier global
paragraph and prevents a reader from treating requirements as completed capabilities.

### P1-3 — “Content-addressed PG18 image” does not identify the bytes actually tested

The specified `postgres@sha256:06cad…` is a multi-architecture OCI index, not one platform image.
The packet identity records that index but no OS/architecture or resolved platform-manifest digest.
An amd64 and an arm64 runner can therefore qualify different image bytes under the same claimed
identity. That violates the immutable-candidate rule even if both report PostgreSQL 18.6.

**Precise foundation fix:** add `platform` and resolved platform-manifest digest to the candidate
tuple and raw evidence, and make the harness assert them before startup. Either qualify one declared
platform or run an explicit platform matrix; do not let an OCI index alone stand for tested bytes.

### P1-4 — TLS wording can be mistaken for support by the present connectors

The target requires `verify-full`, but the current SQL path uses `tokio_postgres::NoTls`, replication
explicitly says TLS is disabled, and the current URL parser only demonstrates tolerating
`sslmode=disable`. The documents generally call this future work, but “verify SCRAM plus
verify-full TLS” is repeated without a fail-closed connector-policy rule. An implementer could pass a
libpq-looking URL to a connector that does not implement libpq TLS semantics and believe it verified
the peer.

**Precise foundation fix:** state that production-mode preflight rejects every TLS/conninfo mode
until each named connector has a documented TLS backend, CA/SAN/hostname verification behavior,
channel-binding disposition, and `pg_stat_ssl` proof. Treat `verify-full` as the required outcome,
not evidence that a connection-string token is honored. Keep `sslmode=disable` explicitly limited to
the current development profile.

### P1-5 — The asserted local PG18 smoke lacks reproducible evidence in the foundation

Note 24 and AGENTS.md describe a particular 18.6 smoke as a current fact, but the repository contains
no checked-in replay command, schema/journal, raw output, engine/DS revision tuple, or artifact hash
for it. The image index does resolve, and the PG18 generated-column facts are sound, but this result
does not meet the foundation's own evidence standard and should not be used as inherited proof.

**Precise foundation fix:** label it “unverified research observation” until its minimal replay
bundle and redacted raw result are checked in, or add a stable evidence reference containing the
exact command, platform digest, fixture SQL, source/engine/DS SHAs, and observed snapshot/live
payloads. It may remain a blocker hypothesis either way; it must not be reported as a qualified
current regression.

## P2

### P2-1 — The skills leave the required validation phase ambiguous

The testing skill says every behavior change obtains a genuine red run “against the exact candidate,”
while the parallel protocol correctly requires red on the base/red patch and green on a later candidate.
The Rust-code skill also says a focused failure is sufficient “when feasible,” whereas AGENTS.md
requires a recorded semantic red for every behavior task. These differences invite a post-hoc green
test or an environmental red to be described as compliant.

**Precise foundation fix:** put one two-commit terminology table in AGENTS.md and link all three
skills to it: `base`, `red patch`, `green candidate`, and `qualification candidate`; define which
identity each command runs against and which result is admissible. Preserve the explicit exceptions
for non-behavioral scaffolding and pure focused-law work.

### P2-2 — Some exact current-baseline counts are not tied to a retained artifact

The new strict-Clippy paragraph gives 44 library and 71 test findings, but unlike the validation
baseline it links no captured output or candidate identity. The command does currently fail, but the
counts are toolchain/source sensitive and will become stale quickly.

**Precise foundation fix:** either move the counts to a dated evidence record with the command,
toolchain, SHA and raw-log hash, or say only that strict Clippy is currently non-green and point to
the future baseline task. Do not make mutable counts part of an as-built policy claim.

## Required disposition

**REVISE.** Resolve all P0 findings before using the documents to dispatch production-readiness
implementation packets. The P1 items should be corrected in the same foundation pass because they
govern authority, current-versus-target claims, and candidate identity. P2 items can be folded into
that edit without expanding production implementation scope.

## Re-review — 2026-08-23

Status: **REVISE**. Re-reviewed the hardened AGENTS.md, all three repository skills, parallel-agent
protocol v2, canonical note 18, and PG18 note 24 at the same repository base. The prior findings on
typed `PLAN-001` bootstrap outputs and scenario timing, public marker publication plus deferred
completion, Git authority, target-versus-current wording, OCI platform identity, fail-closed TLS,
exploratory PG18 evidence, Clippy-count mutability, and red/green terminology have been addressed
substantively. The remaining findings are below.

### P0-R1 — Protocol v2 still classifies an unavailable external oracle as every engine task's direct gate

Note 18 and AGENTS.md now distinguish author/merge gates from final qualification, specifically to
let `ENG-014` repair the inherited Vitest failure before the hermetic external lane exists. Protocol
v2 §7 nevertheless says that **every** engine-touching packet includes the external Electric oracle
command, and that missing Mix is a `blocked` outcome. The packet schema makes that command an
`author_direct_gate`/`merge_direct_gate` unless the packet author makes an unstated exception.
Under §10, a blocked direct gate prevents mergeability. This reintroduces the exact
`ENG-014 → TST-001 → TST-005` deadlock that the phase split was meant to remove.

**Precise foundation fix:** replace the blanket §7 sentence with a generated-gate classification.
For ordinary engine implementation packets, list local Rust/TypeScript/conformance evidence in
author/merge gates and list external Electric/browser lanes in `final_release_qualification` unless
the packet owns or modifies that lane. A packet that installs/repairs an unavailable lane may have a
narrow, validator-declared baseline exception; it must preserve the block and cannot promote. Require
the packet to print the classification, so a missing external tool can never be silently dropped or
accidentally block the task that makes it runnable.

### P1-R1 — The declared PLAN source allowlist excludes one of PLAN's required inputs

Canonical note 18 says PLAN-001's source allowlist contains only the canonical specification and the
generated task manifest. Its bootstrap identity, packet schema, and acceptance also require the v2
execution protocol's bytes/tree and packet-schema semantics. Taken literally, the validator cannot
legally read the protocol that defines its packet/output rules; interpreted loosely, the allowlist
ceases to be a falsifiable protection against a second task authority.

**Precise foundation fix:** define the allowlist as an exact, versioned set of *authoritative planning
inputs*: canonical note 18 plus protocol v2 (with blob/tree IDs), and the generated manifest only
after PLAN writes it. State that notes 23–25 remain scenario/review inputs and cannot define task
metadata. Make the PLAN validator reject any other task-definition source rather than using “only”
in a way that conflicts with its own bootstrap packet.

### P1-R2 — Active lease expiry has no renewal/heartbeat contract

V2 correctly refreshes unaffected leases after an integration generation changes, but a packet also
contains `lease_expires_at` and must stop when expired. It never says who renews a lease during a
long clean test run, how liveness is authenticated, what maximum lease/renewal interval applies, or
whether a controller can renew a lease without an agent's observed acknowledgement. An ordinary
long gate can therefore expire a valid attempt arbitrarily, while an overly long lease recreates the
stale-owner problem leases are intended to prevent.

**Precise foundation fix:** add a lease lifecycle: bounded TTL, authenticated controller-owned
heartbeat/renew request containing packet hash and current generation, renewal only after the
controller rechecks that no relevant input/reservation changed, and an agent-visible acknowledgement
before the next phase/evidence publication. Specify that loss of the control plane/heartbeat expires
the lease and that the controller may not extend an agent's lease silently. Record each renewal in
the immutable attempt note.

### P2-R1 — PG18 note 24 retains two misleading authority/order phrases

Its opening says “the task packets in this note” are incorporated into note 18, while its later
authority section correctly says this note is scenario/rationale only and repeating a task definition
is invalid. Its non-normative “recommended execution fronts” also puts `PG18-000` alongside
`PLAN-001`, although the canonical dependencies require `GOV-001` and `GOV-002` first. The nearby
disclaimer limits harm, but these phrases invite prose-driven assignment—the failure the protocol is
designed to prevent.

**Precise foundation fix:** say “scenario rationale and proposed contract content are reflected in
the canonical tasks,” never “task packets in this note.” Replace the first execution-front item with
the canonical bootstrap sequence (PLAN-001, then validator-emitted GOV-001/TST-000, then
GOV-002, then PG18-000) or remove the list and link to the generated ready-set report.

**Re-review disposition: REVISE.** P0-R1 must be fixed before dispatching engine behavior work; the
two P1 fixes should accompany it to keep the scheduler executable and bounded. The earlier P0/P1/P2
items otherwise appear resolved at the documentation-foundation level.
