# Server differential hardening review

Date: 2026-08-23  
Repository: `/Users/bozilabs/labs/electric-circuits`  
Reviewed HEAD: `0f94a029dc82a29c6f0f36ff82d262f49572c232`  
Canonical spec: `notes/18-production-readiness-spec-reviewed.md`

## Verdict

The canonical note has incorporated nearly all P0/P1 findings from the earlier server-correctness,
operations/durability, PostgreSQL 18, and E2E/TDD reviews. It is materially stronger than the current
server, but it is still a target plan, not evidence that the server is ready. I found two P0
correctness defects (startup ordering/ownership and sequencer fail-closed processing) that are not
closed by the current task wording, and several P1/P2 dependency or gate gaps that can allow a green
packet to omit a required server invariant.

The most urgent issues are boot ordering and fail-closed sequencing: the current engine mutates
PostgreSQL publication/replica identity state before it reads the durable catalog and verifies the
epoch, marks a second engine active when the slot is busy, and advances its source highwater after
logging `process_envelope` failures. The first production profile says the runtime role must not do
those mutations, a successor must not be routable until exclusive ownership is proven, and no
committed effect may be acknowledged before it lands. The plan needs explicit startup and sequencer
owners/gates, not only later deployment prose.

## Ranked findings

### P0-SRV-01 — Boot mutates PG and serves a busy/unchecked epoch before the required preflight

Evidence in the current tree:

- `Engine::setup_postgres` calls `ensure_publication` (which can create `FOR ALL TABLES`) and then
  `ensure_replica_identity_full` for each table at
  [`apps/engine/src/engine/mod.rs:1191-1211`](../Users/bozilabs/labs/electric-circuits/apps/engine/src/engine/mod.rs:1191).
- Only after those writes does it fold the catalog, initialize the change log, and call
  `verify_epoch_at_boot` at
  [`apps/engine/src/engine/mod.rs:1217-1245`](../Users/bozilabs/labs/electric-circuits/apps/engine/src/engine/mod.rs:1217).
- A `Verdict::Busy` is treated as a restorable epoch (`Some(fold)`) at
  [`apps/engine/src/engine/mod.rs:1245-1247`](../Users/bozilabs/labs/electric-circuits/apps/engine/src/engine/mod.rs:1245).
  The code then starts arrangements, applies the catalog, starts the sequencer, and eventually
  stores `HEALTH_ACTIVE` at
  [`apps/engine/src/engine/mod.rs:1264-1321`](../Users/bozilabs/labs/electric-circuits/apps/engine/src/engine/mod.rs:1264).
- The epoch verdict explicitly treats timeline changes as non-breaking and does not compare a
  durable frontier or invalidation state at
  [`apps/engine/src/engine/epoch.rs:165-206`](../Users/bozilabs/labs/electric-circuits/apps/engine/src/engine/epoch.rs:165).
  `health_status` has no `Busy`/preflight state; it reports active unless the epoch is already
  latched broken or degraded.

This violates the target ordering in `DSR-002`, `ENG-015`, `ENG-017`, `OPS-003A/B`, `PG18-002C`, and
`OPS-004`: an old/foreign cluster, a DS restore behind the slot, or a second engine holding the
slot can cause PG DDL, catalog/shape restoration, or public readiness before continuity and
ownership have been established. A PostgreSQL slot-busy observation is not a downstream DS fence.
The one-engine topology reduces the supported failure set, but it does not make an active successor
safe by itself.

Required specification changes:

1. Expand `ENG-006` (or add a narrowly owned `ENG-006B`) with a named startup state machine. Until
   DS store identity/recovery, catalog readability, source-frontier/slot continuity, publication
   admission, and exclusive ownership all pass, the engine must remain unready and must not restore
   shapes, start the sequencer, accept mutating routes, or become gateway-routable. `Busy` is
   `unready`, not a successful restore case.
2. Make `OPS-003A/B` authoritative for all production PG DDL. In production mode the engine must
   not call `CREATE PUBLICATION`, `ALTER PUBLICATION`, `ALTER TABLE ... REPLICA IDENTITY`, or create/
   drop a slot during ordinary setup. The sanctioned reset/publication workflow gets separate
   credentials and a readiness fence.
3. Require `DSR-002`/`ENG-015` to perform the read-only DS/catalog/frontier checks before any PG or DS
   mutation. Add a negative cut for a foreign PG endpoint and for a same-primary slot held by a
   former engine; assert zero PG DDL, zero DS mutation, no sequencer, and non-ready status.
4. Add the startup gate as a dependency of `OPS-004`, `PG18-003A`, `PG18-003Q`, `TST-012`, and
   `E2E-001Q`; put the no-public-success/no-private-mutation assertion in G1/G3/G8. `SRV-E2E-013`
   is the correct public scenario, but its implementation owner must be the startup gate rather
   than only the deployment controller.

### P0-SRV-02 — Sequencer advances highwater/checkpoint after a processing error and silently drops a committed effect

The live loop treats `process_envelope` as best-effort: an error is logged, but the envelope is still
counted, `touched` is set, `highwater` is advanced, and the normal processed/checkpoint path runs at
[`apps/engine/src/engine/sequencer.rs:728-741`](../Users/bozilabs/labs/electric-circuits/apps/engine/src/engine/sequencer.rs:728).
`process_envelope` propagates malformed/missing row values, schema conversion failures, unknown
operations, and subquery registry/query-back failures from `apply_envelope` at
[`apps/engine/src/engine/sequencer.rs:1325-1345`](../Users/bozilabs/labs/electric-circuits/apps/engine/src/engine/sequencer.rs:1325).
An unknown table follows the same drop-and-advance path at lines 702-705. The activation/replay
paths also use `let Ok(...) = apply_envelope(...) else { continue }` and silently skip malformed
buffered or durable-log envelopes at lines 984, 1024, and 1104. `apply_envelope` has real error
conditions (missing value, non-object row, schema decode failure, unknown operation) at
[`apps/engine/src/engine/output.rs:9-48`](../Users/bozilabs/labs/electric-circuits/apps/engine/src/engine/output.rs:9).

This is stronger than a duplicate/replay concern: once the position/highwater is checkpointed, a
restart will not read the committed change again, so a malformed envelope or transient deferred
failure can leave the SQL prefix acknowledged by the source but absent from one or more public
shapes. It violates `ENG-007A`'s “scratch/execution failure stops processing/ack/checkpoint” rule,
`TST-011`'s “slot/checkpoint never passes an unlanded committed effect” acceptance, and the
`E2E-001R/Q` causal materialization contract. Logging the error is not typed retirement/reset and
cannot be treated as a safe event-level duplicate replay.

Required changes:

1. Make `ENG-007A` (or a narrowly owned `ENG-007B`) require a fatal/retryable processing-error
   transition: do not advance `highwater`, `processed`, or the catalog offset until every envelope
   and affected deferred lane has either landed or been durably recorded for replay. Unknown table,
   invalid schema/operation, and registry/query-back failures need explicit permanent-versus-
   transient classification; permanent cases must fail closed with a typed retirement/generation
   reset, not continue.
2. Remove the `else { continue }` skips from activation/replay or make those paths return a typed
   restore failure. Add a focused malformed-envelope/schema-drift genuine-red test that proves the
   checkpoint remains before the bad commit, followed by the permitted reset/retirement outcome.
3. Add this cut to `TST-011`, `TST-012`, `E2E-001R/Q` (especially `SRV-E2E-001`, `011`, `012`) and
   G3/G6. The gate must inspect the durable checkpoint and independent SQL/journal oracle, not only
   logs or an engine health response.

### P1-SRV-02 — Deferred subquery emission is neither bounded/owned at shutdown nor tied to the source checkpoint

Current evidence:

- Subquery emission uses `mpsc::unbounded_channel` and detached `tokio::spawn` writer tasks in
  [`apps/engine/src/engine/emission.rs:40-61`](../Users/bozilabs/labs/electric-circuits/apps/engine/src/engine/emission.rs:40).
- The lane calls `DsClient::append_reliable`, which retries indefinitely and has no shutdown token
  ([`apps/engine/src/ds.rs:508-548`](../Users/bozilabs/labs/electric-circuits/apps/engine/src/ds.rs:508)).
- The sequencer flushes synchronous `txn_pending` output, publishes its processed position, and
  checkpoints without awaiting `pending_flips`/lane completion at
  [`apps/engine/src/engine/sequencer.rs:689-770`](../Users/bozilabs/labs/electric-circuits/apps/engine/src/engine/sequencer.rs:689).
  The emission tasks are not registered shutdown parties.

The event-level profile may choose a typed reset/retirement after a crash, and the current catalog
policy drops subquery shapes on restore. That exception is not stated at this boundary, however:
`E2E-000A` requires `server.drainedThrough` to include deferred work, while `SRV-E2E-011` and
`E2E-001Q` describe final fenced materialization without saying when a deferred stream must be
retired/reset. A SIGKILL or forced shutdown can therefore leave a queued membership effect absent
from the stream after the source/checkpoint has advanced, with no durable child journal.

Required changes:

- Expand `ENG-007` to own emission-lane task handles, queue byte/item caps, shutdown safe points,
  and a durable/replayable child intent. Either hold the source checkpoint until all affected lanes
  land, or durably mark the affected subquery generation for reset/retirement before advancing it.
  A bounded queue that merely rejects a committed effect is not valid.
- Make `ENG-002`/`PROTO-003B` state whether deferred streams are excluded from transaction atomicity
  or are coordinated by the same arbiter. Keep core event-level delivery explicit about safe
  duplicate replay versus typed reset.
- Add lane enqueue, append-response loss, SIGTERM, SIGKILL, and forced-grace cuts to `TST-011`,
  `E2E-001R/Q` (`SRV-E2E-001`, `004`, `011`, `012`), and the G3/G4/G6 matrix. The assertion must
  be “landed effect or one typed retirement/reset,” never an unobservable stale live stream.

### P1-SRV-03 — Required producers are missing from three continuity/GC qualification edges

The task text is directionally correct, but the DAG does not force the producer of the evidence that
the consumer claims:

- `PG18-002C` (durable landed source frontier and slot incarnation) depends only on `PG18-000`,
  `E2E-000I`, and `ENG-006` ([spec:1122-1136](../Users/bozilabs/labs/electric-circuits/notes/18-production-readiness-spec-reviewed.md:1122)).
  It does not depend on `STO-001`/`ENG-015`, which own durable checkpoint/catalog validation. The
  acceptance can otherwise be satisfied with an in-memory or post-read scan that is not the
  restart/restore frontier used by `DSR-002`.
- `PG18-003A` claims `PG18-E2E-008` (immutable publication and sanctioned change workflow), but
  does not depend on `OPS-008`; `PG18-003Q` inherits the gap ([spec:1191-1206](../Users/bozilabs/labs/electric-circuits/notes/18-production-readiness-spec-reviewed.md:1191)).
- `ENG-011` runs storage-wide orphan GC from `DST-001`, `DSR-002`, `STO-001`, and `ENG-010` but not
  the transactional catalog application or authorized reset (`ENG-015`, `DSR-003`) that establish
  the catalog/store generation ([spec:943-950](../Users/bozilabs/labs/electric-circuits/notes/18-production-readiness-spec-reviewed.md:943)).

Required edge changes:

```text
STO-001 + ENG-015 -> PG18-002C
OPS-008 -> PG18-003A -> PG18-003Q
ENG-015 + DSR-003 -> ENG-011
```

Add the source-frontier artifact, its atomic write/recovery point, and catalog/store-generation hash
to G3 and `TST-012`. Add a mutation that makes GC run during restore/reset and assert it neither
deletes a live stream nor treats a foreign generation as an orphan.

### P1-SRV-04 — External Electric qualification is unconditional although native-only production excludes the compatibility surface

`TST-005` is marked `COMMON_SERVER_QUALIFICATION` and its acceptance requires all three external
Electric lanes against the exact release engine/gateway ([spec:1910-1921](../Users/bozilabs/labs/electric-circuits/notes/18-production-readiness-spec-reviewed.md:1910)). The release profile simultaneously excludes raw predicates/compatibility routes from `NATIVE_CORE` and makes the authenticated template gateway the only public surface. A native-only gateway cannot be exercised by Electric's `/v1/shape` oracle without either exposing a disabled capability or using an isolated direct adapter that is not production evidence.

Required change: make `TST-005` conditional on `lane == COMPAT_V1`, or split it into a
`COMPAT_V1` promotional gate and a native `qualification-only` isolated adapter characterization.
If the latter is retained, remove it from native G6/G8 promotion and record the direct adapter as
`not_applicable_by_profile` only through the validator. Apply the same conditionality to `ENG-003`,
`TSC-001`, and any `E2E-001Q` dependency that assumes Electric's raw compatibility protocol.

### P2-SRV-05 — Security gate requires two gateway replicas while the selected topology has one

`SEC-007` explicitly excludes multi-replica quota coordination in the first profile (one gateway/
registry writer), but `TST-004` is `Profiles: all` and its acceptance requires “two-replica quota
races” ([spec:1895-1908](../Users/bozilabs/labs/electric-circuits/notes/18-production-readiness-spec-reviewed.md:1895)). This is not a runnable common-server gate for either first profile.

Change `TST-004` to test concurrent `limit+1` admission against the selected single writer in
`COMMON_SERVER`; move two-replica fencing/quota races behind a future HA gateway feature/profile.
The generated gate matrix must not turn an excluded HA capability into a required gate.

### P2-SRV-06 — `TST-010` excludes valid typed admission/refusal outcomes

The catalog/lifecycle fault matrix currently permits only “exact continuation, completed retirement
or one typed generation reset” ([spec:1973-1983](../Users/bozilabs/labs/electric-circuits/notes/18-production-readiness-spec-reviewed.md:1973)). `ENG-007`–`ENG-009` intentionally introduce bounded admission, quota, and cancellation refusal before state mutation. A limit/queue test can therefore be correct while returning a typed admission refusal, yet fail this acceptance.

Add “typed admission/refusal before mutation” to the allowed outcomes and assert no catalog/share/
stream/PG state was created. Keep post-acknowledgement overload outcomes restricted to landing,
retirement, or reset.

## Current baseline evidence

- `git status --short` was clean; no repository files were changed.
- `pnpm exec vitest run packages/conformance/src/conformance-retention.test.ts --reporter=dot`
  ran 7 tests and failed 1. The failing assertion is the known purge contract mismatch:
  `conformance-retention.test.ts:188` expected the backing stream `404` immediately after
  `DELETE ?purge=true`, but received `200`. This is consistent with current
  `purge_shape_durable` returning after durable `Dropped` while detached close/delete continues.
  The canonical `ENG-014` correctly makes this a required baseline repair; it is not evidence of a
  green release.
- Current catalog restore still logs `catalog restore failed (continuing empty)` at
  [`apps/engine/src/engine/mod.rs:1276-1279`](../Users/bozilabs/labs/electric-circuits/apps/engine/src/engine/mod.rs:1276),
  and `fold_catalog` still silently skips JSON events that do not deserialize at
  [`apps/engine/src/engine/catalog.rs:899-900`](../Users/bozilabs/labs/electric-circuits/apps/engine/src/engine/catalog.rs:899).
  `ENG-015` explicitly assigns the required fail-closed fix; until that packet is integrated and
  qualified, G3/TST-001 must remain blocked.

## No-finding areas (spec now covers the earlier review findings)

These are not claims that the code is already fixed; they are areas where the canonical task design
now has the right owner and acceptance boundary:

- DS-only rollback behind an advanced PG slot: `DSR-001`–`DSR-003`, `PGR-001`, and `PG18-002C` define
  a manifest/frontier decision and a whole-generation reset path.
- Catalog growth and partial restore: `STO-001`, `ENG-015`, `ENG-010`, and `TST-010` explicitly
  require compaction, bounded writer state, atomic application, and no empty/partial serving.
- Purge completion: `ENG-014` names terminal close/delete/`Retired` semantics and repairs the
  known 284/285 baseline failure.
- Large transactions and downstream staging: `ENG-007A`, `ENG-007`, `ENG-010`, and `BND-E2E-001`
  prohibit rejecting a committed transaction for size and require spill/chunk/replay evidence.
- Subset visibility/key identity: `ENG-001`, `ENG-001A`, `E2E-000A`, and `TST-002V` cover
  SnapshotGate inputs, deferred/direct causal stamps, composite identity, and collision fixtures.
- Publication/RLS/generated-column/TLS hardening: `ENG-017`, `PG18-001A/B/Q`, `PG18-002A/C`,
  `OPS-003A/B`, and `PG18-003A/Q` now specify immutable publication ownership, RLS rejection,
  canonical publishable columns, per-connector TLS tests, slot invalidation, and timeline reset.
- Causal E2E/TDD: `E2E-000S/A/B/I` and `E2E-001R/Q` distinguish source, server-drained, and actual
  client-application receipts, independent SQL/journal oracle state, external versus instrumented
  cuts, genuine red evidence, and qualification artifacts.
- Profile/DAG control: `PLAN-001` and `GOV-005` are correctly the only initial scheduling authority;
  until their generated manifest, conditional profile closure, scenario hashes, and clean evidence
  runner exist, no later packet is merge-ready under `AGENTS.md`.

## Required gate summary

Before any production-profile promotion, regenerate the machine manifest with the above edges and
require these direct/qualification checks:

1. Startup preflight/ownership gate (including Busy, foreign PG, DS-behind-slot, and zero-mutation
   assertions) in G1/G3/G8.
2. Deferred-lane shutdown/replay/reset cuts and bounded queue/task evidence in G3/G4/G6.
3. Source-frontier producer, publication workflow, and catalog-generation dependencies present in
   the generated DAG; missing edges are validator failures, not reviewer judgment.
4. Profile-conditional external Electric and gateway-replica tests; excluded capabilities must be
   `not_applicable_by_profile`, never silently skipped.
5. Typed admission/refusal outcomes separated from post-ack landing/retirement/reset outcomes.
6. The current purge baseline must be green under the unchanged semantic contract before TST-001 or
   any release qualification can report pass.
