# Testing, release, and DAG differential review

Date: 2026-08-22

Scope: independent execution-program review of
`notes/16-production-readiness-and-swift-migration-spec.md`, checked against `AGENTS.md`,
`notes/11-test-and-rollout-strategy.md`, and the research-note conclusions in `notes/00` through
`notes/15`. This is a document/DAG audit. It does not edit the execution spec or implementation.

## Verdict

The spec is a strong risk register but is not yet an executable multi-agent program. It defines 72
unique task IDs and has no duplicate heading or undefined literal ID reference, but it contains four
hard dependency cycles, several non-resolvable dependency phrases, wave assignments that violate
declared prerequisites, and gates that make explicitly excluded features mandatory. It also stops at
pre-release qualification: the passive-shadow, dual-read beta, canary, GA, and decommission phases
from note 11 have no task owners or fixed-operation exit criteria.

The release baseline is currently red. A fresh differential run reported `cargo fmt --check`,
`pnpm typecheck`, and `pnpm engine:test` green, but full Vitest at 284/285. The isolated retention
suite repeats the failure: after force purge the shape record is 404 while its stream is still 200.
The code intentionally acknowledges durable `Dropped` before asynchronous close/delete, while the
test and API wording require immediate stream deletion. `TST-001` and G6 cannot close until that
contract is selected and made consistent.

## Ranked findings and required edits

### P0 — the declared DAG has four hard cycles

| Cycle | Why it blocks delegation | Exact dependency correction |
| --- | --- | --- |
| `PROTO-003 -> ENG-002 -> PROTO-003` | Contract and implementation wait on each other. | Make `PROTO-003` depend only on `PROTO-001` and produce the normative event/transaction-framing fixtures. Make `ENG-002` depend on `PROTO-003` and prove the implemented server against those fixtures. Move disconnect/resume acceptance that needs a real producer from `PROTO-003` to `ENG-002`/client conformance. |
| `OPS-002 -> ENG-010 -> OPS-002` | Backup/restore and disk accounting cannot start. | Make `OPS-002` depend on `OPS-001` and own the DS backup/restore contract. Make `ENG-010` depend on `GOV-002`, `ADM-001`, and `OPS-001`; it may consume the backup format but must not wait for the completed drill. Add an `OPS-002 -> ENG-010` integration test after both merge, not a task edge in both directions. |
| `ENG-013 -> OPS-004 -> ENG-013` | The fence implementation and failover exercise wait on each other. | Make `ENG-013` depend on `OPS-001` and `OPS-003` (deployment identity and PG setup). Keep `OPS-004` dependent on `ENG-013` and `OPS-001`–`OPS-003`. |
| `SEC-007 -> ENG-007`–`ENG-010`, while `ENG-008/009 -> SEC-007` | The shared admission policy and two implementations are conflated. | Add `ADM-001` below. Make `ENG-007`–`ENG-010` depend on `ADM-001` and implement global accounting/admission hooks. Make `SEC-007` depend on `SEC-002`, `SEC-003`, `ADM-001`, and the completed hooks, and limit it to principal/tenant gateway enforcement. Remove `SEC-007` from `ENG-008` and `ENG-009` dependencies. |

The spec should store an explicit adjacency list, not infer dependencies from prose or en-dash
ranges, and CI should reject cycles and references to missing/disabled tasks.

### P0 — the current retirement contract/test mismatch needs its own closure task

Add `ENG-014 — Reconcile purge acknowledgement and retirement completion`.

**Depends on:** none. **Assumptions:** existing ADR-0007 and durable catalog behavior, each anchored
to its present regression tests. **Owner:** engine lifecycle/retirement.

Required decision and acceptance:

- Select one public contract. Either a success response means only “durable retirement intent
  accepted,” in which case return a typed pending outcome (preferably 202) and require clients/tests
  to poll or observe stream closure; or it means “stream is closed and deleted,” in which case the
  request-independent retirement task must complete before the success response. Do not preserve a
  200/`ok` response that ambiguously means both.
- Preserve request-cancellation safety, durable `Dropped -> Retired`, close-before-delete, long-poll
  wakeup, retry queue, and restart completion.
- Add focused tests for response loss/cancellation before and after `Dropped`, close, delete, and
  `Retired`; assert the selected response-time state exactly.
- Add `ENG-014` to G3 and G6. `TST-001` may install a red release gate before this lands, but cannot
  record a passing baseline or close G6 until this task and the 285th test are green.

Also amend section 18: the statement that retirement completion is current evidence needs to mention
this unresolved acknowledgement-boundary failure until the task closes.

### P0 — gate closure contradicts the supported/excluded feature profile

The launch gates currently describe one monolithic product:

- G6 requires `ENG-001`–`ENG-006`, even though native subsets are explicitly excluded until
  `ENG-001`; it also indirectly forces `ENG-002` through G5/`PROTO-003` although transaction-atomic
  observers are explicitly not claimed.
- G7 requires every `CMP-*` and every `SWF-*`. That makes `SWF-007` (optional sink), `SWF-008`
  (inventory-dependent aggregates), and `SWF-009` (gated subsets) mandatory. `SWF-013` repeats this
  by depending on `SWF-001`–`SWF-012`.
- `SWF-012` depends on optional `SWF-007`, and `SWF-011` depends on the range `SWF-003`–`SWF-010`, so
  the optionality cannot be represented.
- G1 includes `SEC-008`, which depends on `SWF-012`; server isolation therefore cannot close without
  completing the native mobile package. Container supply-chain and mobile cache handling are two
  different gates.
- `CMP-006` depends on “production server gates.” If that phrase includes G7, it is self-referential.
  If it does not, the actual predecessor set is unspecified.

Add `GOV-005 — Compile the selected launch profile and gate manifest`, depending on `GOV-002` and
`CMP-001`. It should emit a machine-readable list of supported features, required task IDs, excluded
task IDs, and gate predicates. CI must reject a release evidence bundle that omits an enabled task or
requires an excluded one.

Then make these exact gate edits:

- Add `GOV-005` to G0 so the supported topology/features are an executable release profile, not only
  prose.
- Split G7 into **G7a compatibility** (`CMP-001`–`CMP-006`, compatibility portions of `TST-006` and
  `TST-008`) and **G7b native** (`SWF-001`–`SWF-006`, `SWF-010`–`SWF-013`, `TST-007`, `TST-008`, plus
  `SWF-007/008/009` only when enabled by `GOV-005`).
- Make `SWF-013` depend on the core native set plus the profile-selected optional tasks, not the
  numeric range `SWF-001`–`SWF-012`. Make `SWF-012` audit whichever sink/features are enabled rather
  than requiring `SWF-007` unconditionally.
- Re-scope `SEC-008` to server/container/dependency supply-chain work and remove its `SWF-012`
  dependency. Keep mobile cache/credential/privacy acceptance in `SWF-012` and G7b.
- Make G6 require `ENG-014`, all baseline/external suites, and only the feature-specific `ENG-001`
  and `ENG-002` tasks selected by `GOV-005`.
- Add `TST-007` and the re-scoped `TST-002` to G5; schemas without shared runnable fixtures do not
  close the protocol gate.
- Spell `CMP-006` predecessors as G0–G6, G8, and G9 for the compatibility profile; do not use
  “production server gates.”
- Make G4 also include `ADM-001`, `ENG-006`, `ENG-012`, and `SEC-007`; the gate text promises bounded
  connections/configured admission but its present closer list omits them.
- Add `RLS-001` to G8, add `CAP-005` to G9 when a Swift/mobile lane is selected, and make G10 close
  only after `MIG-001`–`MIG-009`, not after the pre-production/rollback subset `MIG-001`–`MIG-005`.

### P0 — there is no owned production rollout after laboratory qualification

`MIG-004` performs a pre-production workload and `MIG-005` proves rollback, then the execution wave
says “release.” Note 11 separately requires passive server shadow, beta dual-read, canary, GA, and
decommission. None has a task ID, principal owner, evidence artifact, or abort/rollback entry.

Add these exact edges in addition to the deliverables below: `MIG-006` depends on `MIG-004`,
`MIG-005`, `OPS-009`, and the profile-selected `CMP-006`/`SWF-013`; `MIG-007` depends on `MIG-006`
and `TST-008`; `MIG-008` depends on `MIG-007`; `MIG-009` depends on `MIG-008`.

Add:

- `MIG-006 — Execute passive server shadow`: one isolated slot/publication; each production template
  must observe a declared number of source transactions, mutations, schema events, restarts, and WAL
  catch-up operations. No client-visible cutover.
- `MIG-007 — Execute opt-in dual-read beta`: isolated candidate cache; fixed minimum unique installs,
  cold starts, background/foreground cycles, network transitions, credential refreshes, and mutations
  per template. The old cache remains authoritative.
- `MIG-008 — Execute per-template canary and GA promotion`: steps are based on a fixed minimum of
  eligible installs and completed operations at each cohort, not elapsed days. Promotion requires no
  unexpected divergence and all rate/resource thresholds; any abort signal invokes `MIG-005` and
  returns to `MIG-004` after a regression fixture lands.
- `MIG-009 — Decommission the old path`: depends on `MIG-008`; requires a fixed number of successful
  rollback/restore rehearsals, proof no supported app build uses the old generation, explicit slot/WAL
  cleanup, and a separate destructive-change approval.

Change the final order to `MIG-004 -> MIG-005 rollback rehearsal -> MIG-006 -> MIG-007 -> MIG-008 ->
MIG-009`. “Rollback proved” is a prerequisite to exposing a cohort, not merely the last migration
activity.

### P0 — the operation-count policy is internally inconsistent and under-specifies exposure

Section 5 sets a default floor of 10,000,000 mutations, 10,000 lifecycle cycles **per client
implementation**, and 100 injections per cut point. `CMP-005` and `MIG-004` specify only 1,000,000
mutations without the ADR the same section requires. `MIG-004` is also ambiguous whether its 10,000
cycles are total or per client/template.

Edit all workload tasks to read counts from a versioned workload manifest. Until an approved ADR
changes it:

- `CMP-005` and `MIG-004` use at least 10,000,000 committed mutations for the long correctness run;
- lifecycle counts are stated per client implementation and per enabled transport profile;
- every template receives a declared minimum number of qualifying in/out/delete/reorder/null/large-
  transaction events; a global write count cannot let a rare template receive zero useful events;
- every count records attempted, accepted, committed, observed, reset, rejected, and compared totals;
- “zero unexplained divergence” becomes “zero unexpected divergence”; allowed normalizations and
  expected resets must be registered before the run, not explained after it;
- seeds, fault schedule, workload manifest, source/image digests, raw observations, and first-failure
  reduction are required artifacts.

Fixed operation count is preferable to an arbitrary calendar soak, but it is not sufficient by
itself for mobile diversity. `MIG-007/008` must also require fixed unique-device/session/build/OS
sample floors. Long lease expiry, certificate rotation, disk growth, and background execution should
be accelerated with injected clocks/configuration and counted transitions.

### P1 — the test program omits three harnesses required by its own evidence claims

Add the following tasks, taken directly from the gaps established in note 11:

1. `TST-007 — Build the shared real-stack client harness and cross-language corpus runners`.
   Depends on `PROTO-004`, `TST-001`, and `SEC-002`. It owns an isolated
   PG/DS/engine/gateway launcher, JSON-line/test API, convergence barrier, fault hooks, and canonical
   fixture loader. Rust/TS runners land first; Swift registration is completed by `SWF-002`.
2. `TST-008 — Qualify the iOS host and real-device lifecycle matrix`. Depends on `CMP-003`,
   `SWF-006`, `SWF-010`, and `TST-007` for the enabled lane. It owns the application-equivalent
   transactional cache, stable launch controls, real `URLSession`, simulator jobs, a declared real-
   device/OS/network matrix, and cache/UI oracle assertions. Split model-based scheduling from
   `TST-006`; that task is already too large.
3. `CAP-005 — Build and run the Swift long-poll/device load generator`. Depends on `TST-008` and
   `CAP-001`. It must reproduce the app's poll deadline, cache commit cost, connection/reconnect
   cadence, background behavior, and supported device memory/battery/network limits. Native TS
   loadgen and `bench:fleet` results cannot substitute for it.

The `ShapeTwin`/verification-coordinator responsibilities fit `MIG-001`, but its acceptance should
explicitly require fenced/quiescent comparison and privacy-safe hashes. A map comparison alone must
not satisfy transaction/duplicate-effect assertions.

### P1 — CI/release evidence can be “complete” while required suites are missing or blocked

Section 5 allows an environmental blocker to be recorded for a task. That is reasonable for a PR
handoff, but a blocker must never count as gate closure. Add a state model of `pass | fail | blocked |
not-applicable-with-profile-ADR`; only `pass` and approved `not-applicable` may promote.

Make these exact changes:

- `TST-001` must name and run `pnpm test:fuzz` separately, all three external commands (`oracle`,
  `property`, `subqueries`), the Swift dependency-boundary script and `swift test`, and the
  AGENTS-required live demo/browser verification for engine live-path candidates. A suite hidden by a
  green aggregate is not evidence.
- Remove ambient `../electric` as a CI assumption. Pin and fetch the Electric test source/image or
  package it as an immutable CI artifact; record its digest/revision in the evidence bundle.
- Preserve the two tag assertions as individually visible expected failures only for a profile that
  excludes tag-dependent modes. They must not be silently skipped or counted as a full external-suite
  pass.
- Add `RLS-001 — Build immutable release artifacts and evidence attestation`. Re-scope `GOV-004` to
  version/compatibility policy. `RLS-001` owns clean-checkout builds, image/package signing, SBOM and
  provenance, symbol/API diffs, evidence schema, and artifact hashes. `OPS-009` consumes `RLS-001`,
  `CAP-004`, `CAP-005` when mobile is enabled, all selected security/test tasks, and the rollback
  compatibility result.
- An evidence result must be fresh for the exact source SHA, dirty-tree status, image/package digest,
  protocol-fixture hash, capacity-target hash, deployment-manifest hash, toolchain, and workload seed.
  Promotion must reject stale evidence from a prior candidate.

### P1 — several “one subagent/PR” packets are programs, not packets

The following tasks cross too many ownership boundaries to merge safely as one agent PR:

| Existing task | Conflict | Exact split |
| --- | --- | --- |
| `GOV-004` | Policy plus multi-ecosystem artifact pipeline. | Keep policy in `GOV-004`; move build/evidence automation to `RLS-001`. |
| `SEC-008` | Container supply chain plus iOS credential/cache/privacy behavior. | Keep server supply chain in `SEC-008`; move all client behavior to `SWF-012`. |
| `ENG-003` | Rust `/v1` behavior and native TS client lifecycle share no primary file boundary. | Keep adapter/server recovery in `ENG-003`; add `TSC-001 — Recover native TypeScript readers from terminal streams`, depending on `PROTO-002` and `ENG-003`. |
| `OPS-002` | Backup tooling, corrupt-artifact handling, epoch compatibility, and 100-cut-point execution. | Keep backup/restore implementation in `OPS-002`; execute its cut points under `TST-012`. |
| `OPS-004` | Leadership implementation and failover campaign. | Keep leadership in `ENG-013`; keep operational promotion/controller and drill in `OPS-004`. |
| `OPS-006` / `CAP-001` | Both claim metric instrumentation. | `CAP-001` owns emitters and raw workload schema; `OPS-006` depends on `CAP-001` and owns collection, dashboards, alerts, and redaction. |
| `TST-002` | Fixture generation plus three-language implementations has inverted ownership (`SWF-002` currently both consumes it and is its dependency). | Let `TST-007` own the corpus/harness; `SWF-002` depends on it and adds the Swift runner; re-scope `TST-002` to final cross-language semantic qualification after `SWF-002`. |
| `TST-003` | Catalog, stream, replication, checkpoint, segmentation, schema, epoch, backup, leadership, and migration failpoints. | Keep `TST-003` as coordinator/report; add `TST-010` catalog/lifecycle/retirement, `TST-011` replication/transaction/segment/schema/epoch, and `TST-012` DS backup/migration/leadership cut-point suites. |
| `CAP-003/004` | Harness creation and a large qualification campaign are combined. | Keep reusable server load/fault harness changes in `CAP-003`; make `CAP-004` consume them for failure evidence; use `CAP-005` for actual Swift/device pressure. |

`OPS-001`, `SEC-002`–`SEC-004`, and `SEC-007` will all edit the gateway/deployment boundary. Assign
one gateway integrator and serialize gateway schema/middleware merges. `PROTO-*`, `ENG-001/002`,
`TST-007`, and `SWF-002` all touch contract fixtures; only the protocol maintainer should merge the
canonical fixture directory. Swift tasks must state which repository owns the new package and app
integration changes, with starting SHA and cross-repository tag dependency; the current workspace
cannot make a clean-checkout Swift release self-contained by implication.

For the fault split, use explicit edges: `TST-010` depends on `ENG-003`, `ENG-010`, `ENG-011`, and
`ENG-014`; `TST-011` depends on `ENG-001`, `ENG-002`, `ENG-004`, `ENG-005`, and `ENG-006`;
`TST-012` depends on `ENG-013`, `OPS-002`, `OPS-004`, and `OPS-005`; coordinator `TST-003` depends
on `TST-010`, `TST-011`, and `TST-012` and only aggregates/replays their evidence.

### P1 — the published execution waves violate prerequisites and contain integration collisions

Examples are sufficient to invalidate the current table as a scheduler:

- Wave 0 places `GOV-002`, `CMP-001`, and `SEC-001` together although the latter two depend on
  `GOV-002`, and `GOV-002` depends on `GOV-001` in the same row.
- Wave 1 schedules `OPS-002` before its wave-2 `ENG-010` predecessor, `OPS-003` before wave-2
  `SEC-006`, `ENG-012` before `SEC-006`, and `CMP-002` before `ENG-003`.
- Wave 2 schedules `OPS-005` before wave-5 `GOV-004` and contains the `SEC-007`/`ENG-008/009` cycle.
- Wave 3 schedules `SWF-001` before wave-5 `GOV-004`, plus dependency chains
  `CMP-003 -> CMP-004`, `PROTO-003 -> PROTO-004`, and `SWF-001 -> ... -> SWF-005` as if parallel.
- Wave 4 schedules `CMP-005` before wave-5 `MIG-001`, schedules `TST-006` alongside its
  `SWF-004`–`SWF-010` predecessors, and lists `OPS-008` a second time.
- Wave 5 presents `CAP-002`–`CAP-004` and `MIG-001`–`MIG-003` as parallel despite their internal
  chains.
- Wave 6 presents `OPS-009`, `SWF-013`, and `MIG-004` as parallel although G7/G8 make the first two
  predecessors of `MIG-004`.

Replace the hand-maintained wave table with one generated from the explicit adjacency/profile file.

## Task-ID integrity and broken-reference inventory

Defined headings: **72**, all unique: `GOV` 4, `PROTO` 4, `SEC` 8, `ENG` 13, `OPS` 9, `CAP` 4,
`CMP` 6, `SWF` 13, `TST` 6, and `MIG` 5. Every literal task ID appearing in the spec resolves to a
heading; there are no literal dangling IDs.

The following dependency references are nevertheless broken for a scheduler because they are ranges,
external concepts, gate aliases, or open-ended phrases rather than exact task IDs:

- ranges such as `PROTO-001`–`PROTO-003`, `ENG-007`–`ENG-010`, `OPS-001`–`OPS-003`, and every
  `SWF-*`/`TST-*` range;
- “existing `SnapshotGate` and catalog,” “schema generations/resolve locks,” and “DS list/index
  capability” (the last is also a deliverable inside `ENG-011`, so the predecessor does not exist);
- “all other OPS tasks,” “engine boundedness tasks,” “production server gates,” “engine/OPS
  durability tasks,” “native tasks used by the app,” and “gates G0–G9.”

Add `DST-001 — Implement an authenticated, bounded durable-stream inventory primitive` and make
`ENG-011` depend on it. `DST-001` depends on `OPS-001` and `SEC-001`. Add `ADM-001 — Define the
shared resource-accounting and admission contract`, depending on `GOV-002`; its outputs are the
machine-readable limit names, units, accounting boundaries, shed/reject semantics, and metrics that
`ENG-007`–`ENG-010` and `SEC-007` implement. Convert every other phrase/range to an explicit list
selected by `GOV-005`.
Track prerequisites that are already implemented as `assumptions` with a source/ADR/test anchor, not
as unresolvable `Depends on` entries.

New IDs proposed by this review, all currently unused: `GOV-005`, `ADM-001`, `DST-001`, `ENG-014`,
`TSC-001`, `RLS-001`, `TST-007`, `TST-008`, `TST-010`, `TST-011`, `TST-012`, `CAP-005`, and
`MIG-006`–`MIG-009`.

## Compact revised execution waves

Arrows below are merge barriers within a lane; comma-separated items at the same arrow level may run
in parallel. Disabled optional tasks are removed by `GOV-005`, not left as unmet blockers.

| Wave | Work after corrected dependencies | Integration checkpoint |
| --- | --- | --- |
| **0 — restore truth** | `GOV-001`; `TST-001` gate wiring; `ENG-014`; independent `ENG-004/005/006` | Ownership selected; baseline either fully green or explicitly red on owned tasks. |
| **1 — select product** | `GOV-001 -> GOV-002`; then `CMP-001`, `SEC-001`, `ADM-001`; `GOV-003` | Capacity/query inventory and route map exist; `GOV-005` can compile the exact profile. |
| **2 — freeze boundaries** | `PROTO-001 -> GOV-004`; `SEC-001 -> OPS-001`; `CMP-001 + TST-001 -> MIG-001`; `GOV-002 + CMP-001 -> GOV-005` | Contract, version policy, protected deployment skeleton, and profile DAG freeze. |
| **3 — server foundations** | `PROTO-002`, `PROTO-003 -> ENG-002`, `ENG-001/003`; `ENG-007/008/009`; `OPS-002`; `SEC-002`, `SEC-005`; `DST-001`; `SEC-006` after `OPS-001` | Stable errors/framing, bounded core primitives, storage recovery contract, authenticated staging edge. |
| **4 — integrate server** | `PROTO-004 -> TST-007`; `SEC-003/004`; `ENG-010`; `DST-001 -> ENG-011`; `SEC-006 -> OPS-003/ENG-012`; `GOV-004 -> RLS-001`; `ENG-003 -> TSC-001/CMP-002` | No server cycle remains; canonical fixtures and staging topology are integration-testable. |
| **5 — leadership and observability** | `SEC-007`; `ENG-013 -> OPS-004`; `OPS-005/008`; `CAP-001 -> OPS-006`; `CMP-002 + SEC-002 -> CMP-003` | Fenced active/passive candidate, resource emitters/dashboards, compatibility transport. |
| **6 — client cores** | `CMP-003 -> CMP-004`; `SWF-001 -> SWF-002 -> SWF-003 -> SWF-004 -> SWF-005`; begin `TST-010/011/012` by subsystem | Isolated compatibility cache and native protocol/lifecycle core; fault families run as components land. |
| **7 — selected product surfaces** | `CMP-004 + MIG-001 -> CMP-005`; `SWF-006`, selected `SWF-007/008/009`, then `SWF-010/011/012`; `TST-002/004/005/006`; `OPS-007` | Compatibility and native RCs meet the selected profile; security/oracle/model evidence is complete. |
| **8 — qualification tooling/evidence** | `TST-008 -> CAP-005`; `CAP-002/003 -> CAP-004`; finish `TST-003/010/011/012`; `MIG-002 -> MIG-003`; `SEC-008`; finish `RLS-001` | Real-device/mobile load, server capacity/fault evidence, signed immutable candidate, cutover controls. |
| **9 — release qualification** | `OPS-009`, profile-selected `CMP-006`/`SWF-013`; then `MIG-004 -> MIG-005` | All selected gates pass for exact digests; rollback is proved before users are exposed. |
| **10 — production rollout** | `MIG-006 -> MIG-007 -> MIG-008 -> MIG-009` | Fixed-exposure shadow/beta/canary/GA evidence; decommission only after independent approval. |

The release maintainer should generate this ordering and the gate-status report from the same
machine-readable graph. Human prose may explain the graph, but must not be the authoritative
scheduler.
