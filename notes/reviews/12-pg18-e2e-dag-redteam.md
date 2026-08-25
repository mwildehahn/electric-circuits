# PostgreSQL 18 / E2E execution-DAG red team

Date: 2026-08-23
Reviewed inputs:

- `notes/18-production-readiness-spec-reviewed.md`
- `notes/24-postgres18-and-e2e-tdd-addendum.md`

This review treats the two notes as a work plan to be executed by independent subagents. It does not
review or change product code.

## Verdict

**Do not schedule the integrated plan yet.** The canonical note's literal task graph is acyclic and
has no unknown IDs, but that is not sufficient evidence that a selected release profile is
executable. The conditional edges and profile inheritance on which compatibility-only and
native-only closure depend are prose, the addendum contains a second divergent definition of all 11
new packets, and several task acceptances require behavior or artifacts owned by tasks that cannot
have run yet. There are also two production-state stores without complete ownership: the gateway's
durable feed registry and the PostgreSQL backup/PITR source used by restore qualification.

The minimum safe start is to make one task registry authoritative, define a real profile-expression
schema, apply the P0 dependency changes below, and then run `PLAN-001` against the corrected graph.

## Reproducible machine check

The checks below parsed headings matching `### TASK-ID — ...`, collected every backticked ID between
`**Depends:**` and `**Profiles:**`, and ran Kahn topological sort plus strongly connected components.
They also enumerated profile labels and compared duplicate task sections after whitespace
normalization.

Snapshot initially fully parsed for this review:

| File | SHA-256 at full parse |
| --- | --- |
| canonical spec | `0afc417e98951f946316ec6f7ea82dea13a15883fbf7c7300e058a40ef460ad3` |
| addendum | `5477c276cbbbef7d7078b97bc16405bcfcffdae7abc59477b6f9ae55dbbb5ed0` |

The files changed in the shared workspace during the audit. The hashes above identify the live
revision on which the final graph counts and findings are based.

| Check | Canonical note | Addendum alone | Integrated reading |
| --- | ---: | ---: | ---: |
| task definitions | 141 | 11 | 152 definitions / 141 unique IDs |
| literal dependency references | 492 | 62 | not safely mergeable |
| unknown dependencies | 0 | 42 (expected external references) | 0 after resolving to canonical IDs |
| duplicate IDs | 0 | 0 | **11** |
| literal SCC cycles | 0 | 0 | 0 if canonical definitions win |
| topologically sorted | 141/141 | n/a standalone | 141/141 if canonical definitions win |
| literal roots | `PLAN-001`, `GOV-001`, `TST-000` | none | ambiguous until one registry wins |

All 11 duplicate sections differ after normalization: `PG18-000`–`003` and `E2E-000`–`005`.
Differences are not merely typography. For example, the addendum and canonical note differ in packet
titles, adapter wording, dependency details, and whether an E2E task says to implement or run the
scenario. A scheduler cannot prove which acceptance contract an assignee used.

The canonical note currently uses 17 distinct free-form profile labels, including `all`, `all
clients`, `client profiles`, `all client profiles`, `all native`, `native`, `Swift client profiles`,
`server qualification`, `all migration lanes`, and exact feature names. Seven tasks contain
conditional dependencies in prose rather than graph data:

- `CAP-005`
- `E2E-004`
- `E2E-005`
- `TST-003`
- `MIG-001`
- `MIG-002`
- `MIG-004`

A conservative profile-closure check also finds a real applicability conflict independent of those
seven prose branches: common tasks `PROTO-004`, `SEC-004`, `STO-002`, `ENG-006A`, `ENG-007A`, and
`TST-002A` all depend on `PROTO-003A`, but `PROTO-003A` is marked `all native`. Therefore a literal
`COMPAT_V1` closure either includes a disabled task or omits a required dependency. The analogous
native conflict is that common qualification tasks depend on `ENG-003`, while `ENG-003` is marked
`COMPAT_V1`.

No day/week/month promotion threshold was found. The notes explicitly reject calendar-duration
acceptance. That is good, but many corpora specify only a lower bound on *committed* operations and no
upper bound on attempts, so a broken or overloaded system can run forever instead of producing a
finite fail result.

## Severity-ranked findings

### P0 — The two-note “integrated” spec has two divergent task registries

The addendum says its packets extend or are incorporated into the canonical DAG, but it repeats all
11 task definitions. `PLAN-001` says duplicate task IDs must be rejected. An executor following that
rule must reject these inputs; an executor silently preferring note 18 violates the rule and may miss
newer addendum text.

**Exact change:** make note 18 the only task-metadata authority. Replace addendum section 6 task
packets with a generated table containing only `ID`, canonical anchor, scenario IDs, and rationale.
Add this text at the top of note 24:

> This note is a scenario and rationale addendum. Task IDs, dependencies, profiles, boundaries, and
> acceptance text are authoritative only in `18-production-readiness-spec-reviewed.md` and the
> generated task manifest. A task heading in this note is a validation error.

Alternatively generate both presentations from the same manifest and require their normalized
task-section hashes to match, but do not maintain two handwritten definitions.

### P0 — Profile closure is not machine-defined and is already contradictory

The plan promises exact profile closure, yet optional profiles are described as additions to
`COMMON_SERVER` while task metadata treats them like unrelated exact labels. It is unspecified
whether `NATIVE_SUBSET` inherits `NATIVE_CORE`; it must, because `SWF-009`, `SWF-010`, `SWF-013`, and
the tests require `SWF-006`. The prose alternatives are counted as dependencies by a simple parser
and ignored by a simpler one. Neither behavior is safe.

**Exact changes:** introduce two explicit axes in the machine graph:

```yaml
lane: COMPAT_V1 | NATIVE_CORE
features: [NATIVE_AGGREGATE, NATIVE_SUBSET, NATIVE_TXN_ATOMIC, NATIVE_REPLICA_SINK]
```

Features are legal only with `lane: NATIVE_CORE`. Store applicability and conditional dependencies
as expressions, for example:

```yaml
- id: E2E-004
  depends:
    - MIG-002
    - MIG-002B
    - MIG-003
    - { id: E2E-003C, when: "lane == COMPAT_V1" }
    - { id: E2E-003N, when: "lane == NATIVE_CORE" }
```

Delete the prose conditional clauses from `CAP-005`, `E2E-004`, `E2E-005`, `TST-003`, `MIG-001`,
`MIG-002`, and `MIG-004`; generate prose from the conditional edges instead. Reject any selected
profile in which a required dependency's applicability evaluates false.

Change `PROTO-003A` from `Profiles: all native` to `Profiles: COMMON_SERVER` (or split private common
framing from a native public capability). Common server tasks already require it. Resolve the
Electric-adapter qualification policy in the same way: because section 4 and G6 require the external
Electric lanes for every engine release, the preferred consistent fix is to mark `ENG-003`,
`TSC-001`, and `TST-005` as `COMMON_SERVER_QUALIFICATION` and state that inclusion does not expose
`/v1/shape` in a native-only public profile. The alternative is to make all three compatibility-only
and amend section 4/G6/TST-003; the current hybrid is invalid.

### P0 — Migration acceptance runs after the rehearsal it is supposed to gate

`E2E-004` depends on `MIG-005`; `MIG-005` depends on `MIG-004`, which already executes shadow,
cutover, rollback, and upgrade in a production clone. Thus public cutover/rollback acceptance is
green only after two broad integration packets have exercised that behavior. This contradicts the
TDD rule and the rule that qualification must not patch behavior. `E2E-003C` is worse: it claims
`CUT-*` and `ROLL-*` cases while depending only on `MIG-000`, not the owners `MIG-002`, `MIG-002B`,
`MIG-003`, or `MIG-005`. Adding those dependencies naively would create a semantic/cyclic release
chain through `MIG-004 -> OPS-009 -> TST-003 -> E2E-005 -> E2E-003C`.

**Exact dependency/text changes:**

- In `E2E-003C`, remove `OWN-*`, `CUT-*`, and `ROLL-*`; retain `SYNC-*`, `LIFE-*`, `AUTH-*`, and
  `CODEC-*` plus compatibility exclusions.
- Change `E2E-004 Depends` to `MIG-002`, `MIG-002B`, `MIG-003`, plus the lane-conditioned
  `E2E-003C`/`E2E-003N`. Remove `MIG-005`.
- Add `E2E-004` to `MIG-004 Depends`.
- Keep `MIG-005 -> MIG-004`, and make `MIG-005` consume the unchanged, hash-pinned `E2E-004`
  scenarios.
- State that `MIG-006` is authorized only after `MIG-005` and the exact `E2E-004` evidence hash.

The resulting order is implementation (`MIG-001`–`003`) -> black-box acceptance (`E2E-004`) ->
isolated clone (`MIG-004`) -> rollback rehearsal (`MIG-005`) -> production shadow (`MIG-006`).

### P0 — PostgreSQL 18 packet order contradicts both the recommended front and acceptance ownership

`PG18-002` is advertised as an early parallel packet, but it depends on `DSR-003`, which is behind DS
qualification, backup, restore-frontier verification, gateway lifecycle/RBAC, and catalog work. Slot
reason classification can and should be red/green early; reset integration cannot. Separately,
`PG18-003` requires the full `PG18-E2E-001`–`009` matrix, including physical-standby promotion, but
the promotion/reset implementation owner is `OPS-004`, which is not a dependency and comes later.

**Exact changes:** split the work as follows:

- `PG18-002A — Observe and fail closed on every slot invalidation reason` depends on `PG18-000`,
  `E2E-000`, and `ENG-006`. It owns the reason capture, unknown-value fail-close, readiness latch,
  and reset-off tests.
- `OPS-003B` depends on `PG18-002A`, not the combined packet.
- `PG18-002B — Integrate invalidation with authorized reset` depends on `PG18-002A` and `DSR-003`.
  It owns reset-on evidence.
- `PG18-003A — Package and preflight PG18` depends on `PG18-001`, `PG18-002A`, `SEC-006A`,
  `OPS-003A`, and `OPS-003B`; it owns `PG18-E2E-001`–`008` except any case explicitly assigned to
  another owner.
- `OPS-004` remains the owner of `PG18-E2E-009`.
- Add `PG18-003Q — PG18 profile integration qualification`, depending on `PG18-003A`,
  `PG18-002B`, and `OPS-004`, and move “complete section 4.1 matrix passes” to it.
- Replace downstream dependencies on `PG18-003` with `PG18-003Q` where the complete matrix is
  required.

Also change `PG18-001` acceptance from “passes through the public client” to “passes through the
isolated reference materializer supplied by `E2E-000`; `PG18-003Q` and `E2E-001` repeat the cases
through the public gateway.” The production gateway is intentionally unavailable at the early
`PG18-001` wave.

### P0 — The gateway's durable registry is an unqualified production database

`SEC-002B` requires a durable transactional feed/claim registry and `SEC-007` requires atomic quota
coordination across gateway replicas. The topology and tasks never choose the backing database,
single-writer/HA model, consistency level, migrations, connection/admission limits, backup/restore,
RPO/RTO, or corruption behavior. `GW-E2E-004` proves restart reconciliation but not loss, rollback,
split brain, or schema upgrade of the registry. This store can mint duplicate claims, leak another
principal's feed, or forget revocation state, so it is part of the security and durability boundary,
not an implementation detail.

**Exact new packets/dependencies:**

- Add `GWR-001 — Own and qualify the gateway registry store`, depending on `GOV-004`, `SEC-001`,
  `SEC-006A`, and `CAP-001A`. Select and pin the database/topology; define transaction/isolation and
  unique-key contracts, fencing, migrations, encryption, pools/queues, physical accounting,
  backup/restore, and failure policy.
- Add `GWR-001` to `SEC-002B Depends`.
- Add `GWR-002 — Gateway registry backup/restore and reconciliation`, depending on `GWR-001` and
  `SEC-002B`; cover ahead/behind/lost/corrupt registry versus engine/DS and require deny/reset rather
  than authority reconstruction by guess.
- Add `GWR-002` to `OPS-002`, `E2E-002`, `TST-012`, and `RLS-001`.
- Clarify the launch topology as either exactly one gateway process or one public gateway service
  with N replicas. If it is one process, move distributed quota to a future profile and qualify
  local quota now. If N > 1, the registry/ledger must be HA and fenced; `OPS-001A` must declare its
  replica count and failure behavior.

### P0 — PostgreSQL restore/PITR evidence has no producer

`DSR-002` compares a DS manifest to whole-stack, PG-only, and PITR states, and `OPS-002` exercises
empty-host restores. No task owns creation, validation, or restoration of the PostgreSQL backup/PITR
artifact, nor proves what happens to publication, slot, system identifier, TLS identities, and the
gateway registry after provider restore. “Provider assumptions” in `PG18-000` is not executable
evidence.

**Exact new packet:** add `PGR-001 — Qualify PostgreSQL 18 backup/PITR and provider restore`, depending
on `PG18-000`, `OPS-003A`, and `OPS-003B`. It must pin the provider mechanism, produce a restore
manifest/frontier, restore to an empty environment at named source cuts, record cluster/system/
timeline/slot/publication facts, and prove the reset-or-resume decision. Add `PGR-001` to `DSR-002`,
`OPS-002`, `TST-012`, and G8. With the `PG18-002A`/`002B` split above this does not form the current
reset/preflight cycle.

### P1 — Test ownership is duplicated between implementation packets and late E2E packets

Section 13 says every behavior packet first adds/selects a stable public E2E test and closes it
green. Later, `E2E-001` and `E2E-002` say to run/implement the same scenario groups only after every
implementation dependency is complete. This permits two agents to edit the same scenario or permits
an implementation agent to satisfy TDD with a focused private test while the stable public test is
not written until later.

**Exact text change:** add a scenario registry with fields `scenario_id`, `contract_hash`,
`test_owner_task`, `implementation_owner_task`, `integration_runner_task`, `profiles`, and
`expected_public_oracle`. The implementation packet owns the stacked red/green scenario patch.
`E2E-001`/`002` may add only image/network adapters and evidence; they may not change the journal,
oracle, expected outcome, or exclusions. Any contract-hash change invalidates implementation and
qualification evidence. Change “Implement `SRV-*`/`GW-*`” in the addendum to “Run the hash-pinned
`SRV-*`/`GW-*` contracts.”

### P1 — Optional native modules can promote without exact optional evidence

`E2E-003N` mentions `AGG-001` and `SUBSET-001`, while `TST-003` says “plus selected optional module
tests.” Neither statement creates machine edges. There is no named optional qualification packet for
transaction atomicity or replica-sink crash semantics. `SWF-013` does not depend on selected optional
modules. `SWF-002` also makes `CMP-002B` part of acceptance even though that compatibility-only task
is neither a dependency nor legal in a native-only closure.

**Exact changes:**

- Extract shared scalar/key golden data from `CMP-002B` into `TST-002V — Shared value/key corpus`;
  make both `CMP-002B` and `SWF-002` depend on it, and remove cross-profile acceptance prose.
- Add profile-conditioned edges from `SWF-013`, `E2E-003N`, `CAP-005`, `E2E-005`, and `TST-003` to
  `SWF-008` for aggregate; `ENG-001`, `ENG-001A`, and `SWF-009` for subset; `PROTO-003B`, `ENG-002`,
  and `SWF-005B` for transaction atomicity; and `SWF-007` plus the app adapter below for replica
  sink.
- Add stable scenarios and runner packets for `TXN-001` and sink crash/acknowledgement, rather than
  hiding them under “selected optional module tests.”
- Make every optional native feature explicitly inherit the `NATIVE_CORE` closure.

### P1 — Native app integration has no principal owner

`SWF-007` combines a reusable sink protocol with an app-owned adapter, violating the one-principal-
boundary rule. For a native migration that does not select the replica sink, `MIG-002` can still
cut over an application cache without any task that implements the application's event-to-reader
bridge. `E2E-003N` saying “actual local persistence where selected” makes the proof optional at the
point it matters.

**Exact changes:** split `SWF-007` into package-level `SWF-007` and `APP-NATIVE-SINK-001`, generated
from the frozen app inventory and depending on `SWF-007` plus `APP-OWN-001`. Add
`APP-NATIVE-CONSUMER-001` for a native-core app that owns event application/checkpoint/reader
integration without the generic sink. `NATIVE-ADR-001` must select exactly one app integration path;
`MIG-002`, `E2E-003N`, and `MIG-004` conditionally depend on it. Add an explicit app credential-
provider/token-refresh adapter owner as part of that app packet or as a separate packet; `SWF-012`
defines an injected interface but does not integrate a production issuer.

### P1 — The “immutable artifact” qualification order is underspecified

`E2E-005` says it runs immutable artifacts, but the final immutable artifact task `RLS-001` depends on
`TST-003`, which depends on `E2E-005`. Adding `RLS-001` as an E2E dependency would create a cycle.
There can be a valid two-stage model—content-addressed candidates first, final attestation later—but
the notes do not name it or require the candidate digest producers.

**Exact text/dependency change:** define `SEC-008A`/`SWF-013`/the compatibility app build as producing
content-addressed **candidate artifacts** that cannot be rebuilt in place. Add those producers as
profile-conditioned dependencies of `E2E-005`. Change its boundary to “candidate artifact set.”
Keep `RLS-001` after `TST-003`; it may only assemble/sign/attest the already-tested digests and must
fail if any byte is rebuilt. Reserve “release artifacts” for `RLS-001` output.

### P1 — Gate evidence can still be laundered through ambiguous status/import rules

The top-level state enum omits `skipped`, `under_run`, `stale`, `zero_tests`, and `wrong_digest`, yet
later tasks say they reject those states. Unless the importer has a normative mapping, an adapter can
report `pass` while its underlying runner skipped tests. `not_applicable_by_profile` is “approved”
without saying who or what may approve it. `TST-000` legitimately passes as a baseline-recording task
while containing failing/blocked lanes, so task status and nested suite status must not share one
field.

**Exact text change:** define separate `task_outcome` and `evidence_observations`. Only the validator
may emit `not_applicable_by_profile`, and only when the task applicability expression evaluates
false for the profile-manifest hash. A required dependency may never be N/A. Map skipped, filtered,
zero-test, under-run, stale, missing raw data, dirty source, wrong digest/config/profile, or missing
scenario IDs to `fail`; environmental unavailability maps to `blocked`. A meta-task such as
`TST-000` may pass its inventory obligation while each nested lane retains its own non-pass result;
downstream gate predicates inspect the nested lane results, not only the meta-task result.

### P1 — G10 is both a prerequisite for “production readiness” and something proven using production

The final decision says production readiness requires G0–G10, but G10 includes passive production
shadow, beta, 10/50/100% rollout, and decommission. That wording either forbids the exposure needed
to collect G10 evidence or allows production traffic while the system is formally not ready. There
is also no single promotion authority for each exposure boundary.

**Exact change:** split G10 into staged authorization gates:

- `G10a lab`: `E2E-004` + `MIG-004` + `MIG-005`; authorizes passive shadow only.
- `G10b shadow`: `MIG-006`; authorizes opt-in beta only.
- `G10c beta`: `MIG-007`; authorizes 10% canary only.
- `G10d canary`: separate evidence for 10%, then 50%, then 100%; each command consumes the previous
  immutable evidence hash.
- `G10e decommission`: `MIG-009`, explicitly after the rollback-support window/policy rather than as
  a prerequisite to initial GA.

Add a named promotion task/command and approver role for each boundary. Say “GA readiness” rather
than “production traffic” where the full chain is intended.

### P1 — Operation-count acceptance is not necessarily finite or reproducible

“10,000,000 committed operations” is a lower bound on a downstream outcome. If admission rejects all
work or a fence never completes, the harness can make unlimited attempts or wait forever. Production
shadow/canary counts have the same issue. “At least” also allows different agents to stop at different
points. `MIG-007`'s 50 installations “spanning cohorts” and `MIG-008`'s 10/50/100% lack a denominator
snapshot and exact cohort floors. “Zero unexplained divergence” permits post-run explanation unless
the pre-registered allowlist rule is repeated here.

**Exact text change:** every workload manifest must declare exact `attempt_budget`, `offered_budget`,
per-operation diagnostic deadlines, global harness deadline, minimum admitted/committed/applied
counts, and a deterministic stop condition. Reaching the attempt/time cap without the minimum is
`fail` or `blocked` by a named external cause; it never keeps running. Replace all “zero unexplained
divergence” with “zero divergence outside the pre-run signed allowlist hash; changing the allowlist
invalidates and restarts the stage.” For `MIG-007`/`008`, pin the eligible-installation denominator,
assignment hash, consent record, exact minimum per OS/device/account/template cohort, real-user versus
synthetic counts, and evidence freshness. Synthetic traffic may satisfy operation/load floors but
never installation/cohort exposure.

### P1 — Several packets violate the declared subagent-size/boundary rule

`E2E-000` owns the PG fixture, stack controller, oracle, journal, fault gates, resource observer,
adapter protocol, and diagnostics. `PG18-001` spans schema introspection, publication admission,
fingerprint, backfill SQL, tuple decoding, and identity. `PG18-003` and every `E2E-*` packet are
cross-runtime integrations but are absent from section 13's integration-task list. `CAP-003` combines
a large combinatorial matrix, envelope search, statistical analysis, and the 10M corpus.

**Exact change:** label `PG18-003*`, `E2E-000`–`005`, and `CAP-003` integration tasks and give them
cross-boundary reviewers. Prefer these splits before assignment:

- `E2E-000A` source journal/SQL oracle/PG fixture;
- `E2E-000B` stack/process/fault/resource primitives;
- `E2E-000I` adapter integration and mutation proof;
- `PG18-001A` canonical publishable-column schema/admission;
- `PG18-001B` backfill/pgoutput/identity consumers;
- `PG18-001Q` public/reference integration scenario;
- `CAP-003A` matrix generator/envelope search and `CAP-003Q` immutable qualification run.

Each implementation child gets one principal write boundary; integration packets consume their
artifacts and must not repair implementation.

### P2 — The bootstrap ready set bypasses the validator that makes assignment safe

The initial ready set includes `GOV-001` and `TST-000` alongside `PLAN-001`, even though `PLAN-001`
is what creates owners, artifact boundaries, profile closure, and duplicate-ownership validation.
Those two packets are low-risk docs/evidence tasks, but merging them before the registry exists
still contradicts the scheduling authority rule.

**Exact change:** only `PLAN-001` may merge in bootstrap wave 0. `GOV-001` and `TST-000` may prepare
read-only evidence concurrently but merge only after the validator accepts their owner/artifact
records. Regenerate the ready set from the manifest; do not maintain the paragraph manually.

### P2 — The public revocation barrier needs a byte-commit definition

`GW-E2E-005` requires no bytes after a revocation barrier. Bytes already committed to a socket/TLS
record cannot be recalled, so an acknowledgement-based barrier is unverifiable unless the gateway
buffers a bounded long-poll response and rechecks authorization before the first public byte.

**Exact text change:** define the barrier as: stop admission; cancel and join all principal-owned
upstream reads; invalidate the generation; then acknowledge revocation. A response for which public
headers/body have begun is recorded as pre-barrier delivery; responses held upstream or not yet
publicly committed must emit zero bytes. `SEC-004` must buffer one bounded response or provide an
equivalent first-byte commit gate. The E2E probe records public header/first-body-byte sequence around
the acknowledgement.

## Coverage/ownership audit by area

| Area | Covered well | Missing or ambiguous owner |
| --- | --- | --- |
| PG18 admission | generated columns, publication/plugin/identity checks, slot invalidation, TLS, promotion reset | provider backup/PITR producer; early invalidation classification is coupled to late reset; full PG matrix owner precedes promotion owner |
| Server/DS | catalog, segmented log, retention, disk reserve, DS backup/restore, singleton engine/DS | whole-stack restore includes an unspecified PG artifact; candidate-versus-release artifact stage is unnamed |
| Gateway | auth, templates, proxying, revocation, quotas, audit, listener isolation | durable registry/ledger database, topology, migration, backup/restore, corruption and HA; one-process versus multi-replica launch topology |
| Swift | protocol, actors, bounded stream, codecs, lifecycle, security, device tests | exact optional-feature edges/tests; app credential adapter; app consumer/sink owner; shared codec corpus leaks through compatibility task |
| Migration | common fence, comparator, ownership switch, rollback freshness, staged exposure | acceptance/rehearsal order reversed; cut/rollback scenarios claimed before implementation; exact staged promotion authority and cohort denominator |
| Capacity | metric schema, open-loop driver, envelope discovery, recovery and device load | attempt/stop caps; exact optional-feature matrices; `CAP-003` too broad; maximum/post-release baseline values need manifest hashes and recovery deadlines |

## Recommended corrected execution spine

This is not a manually maintained wave plan; it is the order the corrected dependencies should
produce.

1. Canonicalize the registry and complete `PLAN-001`; freeze profile axes, owners, artifacts, and
   scenario hashes.
2. Run governance/baseline/support contracts: `GOV-001`, `GOV-002`, `TST-000`, `PG18-000`.
3. Build the split `E2E-000` core and run `PG18-001*` plus `PG18-002A` red/green through the isolated
   reference adapter.
4. Implement/qualify gateway registry storage, PG bootstrap/preflight, PG backup/PITR, DS/catalog/
   restore, and `PG18-002B` reset integration.
5. Build the protected public gateway and candidate artifacts; run `PG18-003Q`, `E2E-001`, and
   `E2E-002` without changing scenario semantics.
6. Complete exactly one client lane and selected optional feature closures; run `E2E-003C` or
   `E2E-003N` plus exact optional evidence.
7. Implement migration ownership, then run `E2E-004`; only afterward run `MIG-004` and `MIG-005`.
8. Run finite fixed-operation capacity/fault qualification and `E2E-005`; assemble/sign the tested
   digests in `RLS-001`.
9. Advance through explicit G10a–G10e exposure authorizations. A blocked/under-run stage remains
   non-promotable; it does not become N/A or pass with more calendar time.

## Machine-validator fixtures to add to `PLAN-001`

The corrected plan should seed and reject at least these mutations:

1. Duplicate `E2E-000` definitions with different dependencies across two notes.
2. A `COMPAT_V1` common task depending on `PROTO-003A` marked native-only.
3. A native optional feature selected without `NATIVE_CORE`.
4. A prose `A or B by profile` dependency with no structured conditional edge.
5. `E2E-004 -> MIG-005` combined with the corrected `MIG-004 -> E2E-004` edge.
6. A required dependency reported `not_applicable_by_profile`.
7. A runner reporting pass with zero tests, filtered scenarios, under-run counts, or wrong artifact
   digest.
8. A qualification artifact whose scenario contract hash differs from the implementation's green
   evidence.
9. Two tasks claiming the gateway registry schema/migration artifact.
10. A 10M-commit corpus with no attempt budget or terminal stop condition.

Until these fixtures pass and the two missing production-store owners are added, the literal
acyclicity result should be reported as **syntactic pass, execution-plan fail**.
