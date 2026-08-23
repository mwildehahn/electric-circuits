# Server-correctness differential review

Scope: differential audit of `notes/16-production-readiness-and-swift-migration-spec.md` against
`AGENTS.md`, notes 04/06/15, the as-built engine, and its current conformance tests. This review is
limited to replication, snapshot fencing, sequencing, catalog/epoch/drift safety, the segmented
change log, retention/retirement, source transactions, subsets, subscriptions, and output delivery.
No implementation or spec file was edited.

## Verdict

**No-go as an execution plan until the blocker findings below are incorporated.** The draft correctly
recognizes most of the server's existing correctness mechanisms, but several work packets cannot be
scheduled as written, and three launch claims are not implementable by the proposed mechanism:

1. the dependency graph contains four direct cycles;
2. a PostgreSQL advisory lock cannot fence a partitioned former leader from durable-streams;
3. restoring durable-streams behind a slot whose `confirmed_flush_lsn` has advanced is not detected by
   the current epoch binding;
4. the catalog is both an unbounded in-memory queue and an append-only, never-compacted disk log;
5. output transaction markers cannot describe deferred subquery emissions without either changing the
   execution model or excluding that stream class; and
6. the release baseline is already red because purge acknowledgement semantics and a conformance test
   disagree.

The draft should remain a synthesis draft until these are resolved. They are not capacity-tuning
details: each can produce split-brain writes, silent omission after restore, unbounded memory/disk, or
an unshippable gate.

## Coverage confirmed

The following parts of the draft accurately cover real mechanisms and should be retained:

- `ENG-001` identifies the correct residual subset defect. A `SnapshotGate` classifies visibility by
  xid, not WAL position (`apps/engine/src/pg.rs:799-875`), while `/query` currently returns only an LSN
  (`apps/engine/src/pg.rs:1203-1207,1256-1303`) and the client compares only LSNs
  (`packages/client/src/subset.ts:304-349,483-500`).
- `ENG-003` maps the two known terminal-stream gaps from parent issues #17/#18 and preserves the
  essential replacement/reset rule. The live append path already reconciles a claimed terminal
  response with engine registration plus a storage `HEAD` before it discards output
  (`apps/engine/src/engine/lifecycle.rs:2010-2068`).
- `ENG-004` and `ENG-005` correctly carry the two outstanding schema issues from note 15. Current
  `TRUNCATE` handling retires dependents unconditionally (`apps/engine/src/engine/drift.rs:371-386`),
  and the reconciler examines only already-tracked tables (`apps/engine/src/engine/drift.rs:617-656`).
- The draft preserves append-before-slot-ack, transaction-end input markers, the sequencer highwater,
  and the requirement that shape appends land or the shape retire before progress is declared. Current
  code appends every input chunk before `update_applied_lsn` (`apps/engine/src/replication.rs:455-470`),
  holds an unterminated input run (`apps/engine/src/engine/sequencer.rs:620-666,901-923`), and flushes
  each synchronous per-stream transaction batch before the next transaction
  (`apps/engine/src/engine/sequencer.rs:689-764,1655-1679`).
- `TST-003` names the right durable fault domains: catalog events, checkpoint/highwater, rotation,
  drift, epoch, and retirement. Existing code really does pair `Dropped` intent with `Retired`
  completion and requeue unmatched intents (`apps/engine/src/engine/retirement.rs:1-19,148-166`), and
  the catalog fold retains the maximum id ever minted (`apps/engine/src/engine/catalog.rs:646-676,
  710-720`).
- Segmented-log semantics are represented correctly: transaction-boundary rotation, type-based
  controls, `(segment, offset)` positions, durable-checkpoint deletion floors, and dormant/reactivating
  pins (`apps/engine/src/changelog.rs:1-40,69-96`; `apps/engine/src/engine/mod.rs:1330-1389`).
- `ENG-007` through `ENG-010` correctly recognize that ingest spill alone does not bound downstream
  work, and `ENG-003`, `SWF-004`, and `SWF-005` preserve named subscription, lease, replacement, and
  old-reader cleanup concerns.
- The draft appropriately uses fixed operation counts, deterministic cut points, and oracle
  convergence instead of calendar monitoring.

## Blocker findings

### B1. The declared dependency graph is cyclic

Affected tasks: `ENG-002`, `PROTO-003`, `ENG-008`, `ENG-009`, `SEC-007`, `ENG-010`, `OPS-002`,
`ENG-013`, `OPS-004`; section 4 and execution waves 1-4.

Four direct cycles make the plan non-delegable:

- `PROTO-003` depends on `ENG-002`, while `ENG-002` depends on `PROTO-003`.
- `ENG-010` depends on `OPS-002`, while `OPS-002` depends on `ENG-010`.
- `ENG-013` depends on `OPS-004`, while `OPS-004` depends on `ENG-013`.
- `SEC-007` depends on `ENG-007`-`ENG-010`, while `ENG-008` and `ENG-009` depend on `SEC-007`.

This also contradicts the wave table, which places protocol design before transaction implementation
but encodes the reverse dependency in the task bodies.

Required correction: introduce design/contract prerequisites that do not depend on implementation,
then make implementation and qualification one-way dependencies. The exact corrected edges are in
the dependency section below.

### B2. `ENG-013` cannot meet its split-brain acceptance criterion with its proposed mechanism

Affected tasks/gates: `ENG-013`, `OPS-004`, `TST-003`, `CAP-004`, G3/G8/G9.

`ENG-013` proposes a PostgreSQL advisory lock or equivalent lease and requires a paused or partitioned
old leader to be unable to append after losing the fence. A PostgreSQL session lock can stop a second
walsender/control plane, but it cannot revoke the first engine's already-open connection to a separate
durable-streams service. If the old engine loses PostgreSQL connectivity, a replacement can acquire the
database lock while the old engine still has DS connectivity and queued catalog/shape writes.

Code evidence:

- Slot-busy handling only waits on PostgreSQL's active walsender (`apps/engine/src/engine/epoch.rs:
  165-206`); it is not a DS write fence.
- `CatalogWriter` sends plain events over an unversioned DS client and carries no leadership token
  (`apps/engine/src/engine/catalog.rs:189-218,305-330`).
- `ChangeLogWriter` explicitly assumes it is the only writer (`apps/engine/src/changelog.rs:308-313`)
  and appends without a fencing generation (`apps/engine/src/changelog.rs:340-380`).
- Shape appends likewise contain no leader/epoch precondition (`apps/engine/src/ds.rs:446-465`).

Required edit: rewrite `ENG-013` to require a monotonically increasing fencing token enforced by the
storage mutation boundary for **catalog, changes, shape append, close, and delete**, or narrow the
supported failover contract to operator-confirmed termination/STONITH and remove the partition/resume
acceptance. The current acceptance requires the former. Epoch identity in event payloads is insufficient
unless DS rejects stale writers before committing them.

### B3. G3/`OPS-002` promises safe restore without an engine mechanism that can detect DS rollback

Affected tasks/gates: `OPS-002`, `OPS-004`, `OPS-005`, `TST-003`, G3.

The ingestor acknowledges PostgreSQL after a commit reaches the change log. A later restore of DS from
an older backup can erase that acknowledged change, its shape output, and newer catalog checkpoints
while the same PostgreSQL slot remains present at a later `confirmed_flush_lsn`. The current epoch
verdict accepts that state: `SlotBinding` records only `system_identifier`, `timeline_id`, slot, and
time (`apps/engine/src/engine/epoch.rs:82-96`), and the verdict checks cluster/slot/plugin/WAL status,
not restored DS progress (`apps/engine/src/engine/epoch.rs:179-206`). `init_change_log` proves only that
the catalog-named local segment exists (`apps/engine/src/engine/mod.rs:1335-1379`).

Consequently, “record the relationship” in `OPS-002` is not a deliverable strong enough to close G3.
A DS-only rollback on the same cluster/slot can silently pass the current checks.

Required new task `ENG-014 — Bind storage restore state to replication progress`:

- Choose one supported rule: either a verifiable cross-store generation/progress record that detects
  every DS/slot rollback mismatch, or **every DS restore forces an authorized epoch reset and full
  shape retirement before readiness**.
- Make the check happen before catalog shapes resume, the change-log writer starts, or readiness turns
  200.
- Cover DS-only rollback, PG-only rollback, coordinated restore, slot ahead/behind/equal, missing
  segment, and interrupted recovery. “Compare after restore” is not sufficient; unsafe combinations
  must be rejected before serving.

### B4. G4 cannot close while the durable catalog is unbounded in memory and on disk

Affected tasks/gates: `ENG-007`, `ENG-010`, `CAP-001`, `CAP-003`, `OPS-005`, `OPS-006`, G3/G4/G9.

The boundedness inventory omits the most correctness-sensitive queue and log:

- `CatalogWriter` uses `mpsc::unbounded_channel` (`apps/engine/src/engine/catalog.rs:189-197,314-316`).
- Every renewal queues another `Joined` event even when the claim already exists
  (`apps/engine/src/engine/lifecycle.rs:196-212`). During a DS outage a fixed number of clients can
  therefore grow the queue without bound.
- `meta/catalog` is append-only. The architecture explicitly says it is never compacted and calls
  snapshot/truncate or an embedded KV a future step (`docs/ivm-engine-internals.md`, section 4.5).
- The retirement queue is also unbounded (`apps/engine/src/engine/retirement.rs:40-57,71-95`), although
  durable `Dropped` intents make that queue reconstructible.

`ENG-010` accounts for catalog bytes but provides no way to keep the catalog inside a disk budget.
Refusing later catalog records is not safe: offsets, rotations, drops, lease events, and retirements
are part of the restart contract. A hard disk limit will eventually deadlock progress or exhaust disk.

Required new task `ENG-015 — Bound and compact the durable catalog`:

- Specify a crash-safe catalog snapshot/generation switch preserving epoch binding, every live claim
  and lease age, dormant gates/positions, checkpoint+highwater, segment inventory, maximum shape id,
  and unmatched `Dropped` intents.
- Bound the writer's resident bytes. Admission must happen before a state mutation whose catalog event
  cannot be queued; engine-generated mandatory events need a durable spool or an equivalent lossless
  design. Do not merely replace `unbounded_channel` with a blocking send while holding engine state.
- Coalesce renewal churn only with a proof that `Joined` cannot cross or erase a later `Left`, `Dropped`,
  or new owner transition.
- Crash at every snapshot/switch/truncate point and fold old+new generations to one identical state.

### B5. `ENG-002`/`PROTO-003` cannot promise source-transaction completion for all current streams

Affected tasks/gates: `ENG-002`, `PROTO-003`, `PROTO-004`, `SWF-005`, `TST-002`, G5/G6/G7.

For plain/routed/aggregate output, the sequencer sees a complete source transaction and stages a
per-stream vector, so it can mark the last envelope before chunking. Subquery flip output is different:

- inner-set flips are deliberately returned for deferred PostgreSQL query-backs after the sequencer's
  synchronous work (`apps/engine/src/subquery.rs:1363-1476`);
- those batches drain asynchronously through independent lanes
  (`apps/engine/src/engine/emission.rs:1-22,40-85`);
- deferred shape output currently strips the LSN and emits `last: None`
  (`apps/engine/src/subquery.rs:1877-1899`; `apps/engine/src/engine/output.rs:127-166,173-192`); and
- later transactions can be evaluated/enqueued before an earlier transaction's deferred query-back
  finishes. Absolute per-pk emission guarantees convergence, not contiguous source-transaction
  framing (`apps/engine/src/subquery.rs:1905-1932`).

Stamping the last synchronous envelope is therefore false, while stamping a later deferred envelope
does not make the source transaction contiguous in stream order. A consumer buffering “until last” can
mix transactions or wait forever. The task also asks for a boundary for a transaction producing no
event for a shape while simultaneously forbidding fake changes; no envelope exists on which to put it.

Required edit: `PROTO-003` must first define the scope as **each non-empty projected transaction on one
stream**, with no marker for a no-output transaction and no cross-stream atomicity. Then either:

- exclude subquery-bearing streams from transaction-atomic delivery in the support matrix and expose a
  capability bit; or
- add a transaction-aware output arbiter that includes deferred propagation, prevents later output on
  an affected stream from overtaking completion, and gives every emitted batch a stable source token.

Until one is chosen, remove `ENG-002` from the universal G6 gate or remove the document's claim that
event-level delivery is an allowed first-release mode. Currently section 2 says atomic observers are
excluded until `ENG-002`, while G5/G6 require `PROTO-003`/`ENG-002` for every production release.

### B6. `TST-001` is not presently satisfiable: purge acknowledgement and its test disagree

Affected tasks/gates: `TST-001`, `PROTO-002`, `ENG-003`, `SEC-005`, G6.

Fresh reproduction on this checkout:

```text
pnpm exec vitest run packages/conformance/src/conformance-retention.test.ts --reporter=dot
Test Files  1 failed (1)
Tests       1 failed | 6 passed (7)
line 188: expected stream GET 404, received 200
```

The test says `DELETE ?purge=true` removes the backing stream immediately
(`packages/conformance/src/conformance-retention.test.ts:177-193`). The implementation intentionally
does something else: it removes the record, waits until `Dropped` is durable, spawns `finish_purge`,
then returns without waiting for close/delete (`apps/engine/src/engine/lifecycle.rs:940-990`). This is
consistent with durable intent/eventual retirement, and the HTTP comment promises restart-safe intent,
not completed deletion (`apps/engine/src/http.rs:555-581`).

Required new task `TST-000 — Establish a green, semantically correct baseline` before `TST-001`:

- Specify that purge success means the record is gone and `Dropped` is durable; physical close/delete
  is eventual unless the product deliberately chooses a stronger response contract.
- Change the immediate-404 assertion to poll to a fixed operation/deadline bound and retain the test
  that an already-waiting long poll receives `stream-closed` before deletion.
- If immediate physical retirement is selected instead, change `purge_shape_durable` to await an
  engine-owned completion future without making request cancellation lose the obligation.
- Record the exact baseline SHA and full-suite result. No release work should treat inherited red as
  an expected deviation.

## High findings

### H1. `ENG-007`'s transaction bound conflicts with the no-size-invalidates-correctness invariant

Affected tasks: `ENG-002`, `ENG-007`, `CAP-004`, `TST-003`, `TST-005`.

The ingestor spills and accepts transactions of any size, but the sequencer holds a complete source
transaction and all per-stream output in memory (`apps/engine/src/engine/sequencer.rs:620-666,
689-756`). `flush_pending` then sends one body per stream with no output chunking
(`apps/engine/src/engine/sequencer.rs:1655-1679`). `ENG-007` says “bound `txn_pending`” and mentions
backpressure/rejection, but a committed PostgreSQL transaction cannot be rejected or used to retire a
shape merely for exceeding a cap. Reconnect would redeliver the same transaction forever.

Required edit: require spillable sequencer hold/output staging plus chunked output appends. Memory caps
may backpressure, but disk-spill failure must fail closed without acknowledging/checkpointing past the
transaction. Acceptance must include transactions larger than every memory and append cap; there must
be no “transaction too large” retirement/rejection branch.

### H2. `ENG-001` does not cover every input needed for a sound subset fence

Affected tasks: `ENG-001`, `PROTO-001`, `SWF-002`, `SWF-009`, `TST-002`, `TST-005`.

The task is directionally right but underspecified in four correctness-critical ways:

1. **Every page needs a fence.** `loadMore` runs in its own repeatable-read transaction and races the
   same tail (`packages/client/src/subset.ts:587-633`); fixing only initial page creation leaves stale
   page resurrection possible.
2. **Every relevant output needs a causal source stamp.** Normal output retains xid-as-`txid` and LSN,
   but deferred subquery output passes `lsn: None` (`apps/engine/src/subquery.rs:1884-1891`). The current
   client treats a missing LSN as always fresh and drops the tombstone watermark on a delete
   (`packages/client/src/subset.ts:313-329`). A raw `SnapshotGate` token alone cannot classify such an
   event.
3. **Composite-PK subsets are not implemented.** The server orders/tie-breaks on only
   `ts.pk_index` (`apps/engine/src/pg.rs:1271-1282`), and the client identifies rows with one declared
   `primaryKey` and `String(row[pk])` (`packages/client/src/subset.ts:366-372,483-496`). This does not
   match the injective composite envelope key promised elsewhere.
4. **The deterministic WAL-written/not-visible test is not actionable without a hook.** Ordinary SQL
   cannot pause PostgreSQL between commit-record flush and `ProcArrayEndTransaction`. Existing
   concurrency coverage is probabilistic (`packages/conformance/src/conformance-concurrency.test.ts:
   39-76`).

Required edit: choose a server-owned page+overlap reconciliation operation, or define a versioned
opaque fence and a server classification operation that applies to initial/replacement/load-more pages
and all direct/deferred events. Do not overload application `txid` as a guaranteed xid. Either implement
full ordered PK tuples and opaque composite identity or explicitly reject composite-PK subsets. Make the
deterministic acceptance use a named test hook/custom PostgreSQL fixture; otherwise use a synthetic gate
unit test plus fixed concurrent E2E repetitions and do not claim the exact internal window was held.

### H3. `ENG-004` acceptance contradicts catalog restore policy for subqueries

Affected tasks: `ENG-004`, `OPS-005`, `TST-003`, `TST-005`.

Active `ShapeRecord` does not persist its seed gate (`apps/engine/src/engine/introspection.rs:8-46`);
the gate lives in executors (`apps/engine/src/engine/executors.rs:37-45,197-205,330-343`) and only a
dormant event stores one (`apps/engine/src/engine/catalog.rs:58-67`). More importantly, catalog restore
always drops subquery shapes because inner-node state is not persisted
(`apps/engine/src/engine/catalog.rs:949-958`). Therefore “a post-TRUNCATE subquery shape remains live
across restart/replay” cannot pass without adding subquery-state persistence, which `ENG-004` does not
propose.

Required edit: limit survival acceptance to restorable plain/routed/aggregate/circuit dependents and
assert the documented typed retirement for subqueries, or explicitly expand the task into durable
subquery-state recovery. Specify which per-dependent seed gate is persisted/reseeded and add its catalog
format to `OPS-005` fixtures.

### H4. No task bounds retained derived state after admission

Affected tasks/gates: `ENG-009`, `SEC-007`, `CAP-001`, `CAP-003`, G4/G9.

`ENG-009` bounds request-time materialization, but accepted state can grow later from ordinary source
writes: subquery contributor/feed sets scale with membership, MIN/MAX maintains a distinct-value
multiset (`apps/engine/src/engine/executors.rs:325-349`), and counts pipelines scale with distinct
groups. A create-time cardinality check does not bound tomorrow's committed changes. After a source
transaction commits, silently dropping the state update is not an option.

Required new task `ENG-016 — Bound retained derived state`:

- Account bytes/count for membership-circuit state and spill, host feed sets/dictionaries/recency,
  aggregate MIN/MAX multisets, counts groups, and per-shape indexes.
- Define transition behavior when already-acknowledged workloads cross a limit: spill/backpressure,
  retire affected acknowledged shapes with durable intent before discarding output, or fail closed.
- Test growth past the limit from live DML, not only an oversized initial seed, and prove the sequencer
  never advances past an unhandled effect.

### H5. Tenant-safe subscription ownership is asserted but not designed

Affected tasks/gates: `PROTO-002`, `SEC-002`, `SEC-003`, `SEC-004`, `SEC-007`, `TST-004`, G2.

The engine treats a subscription as a free-form global string and maps only subscription to shape
(`apps/engine/src/http.rs:199-247`; `apps/engine/src/engine/mod.rs:418-430`). Knowledge of another
subscription and shape id is enough to attempt its renewal/release through the internal API. The gateway
must therefore be the durable ownership boundary, but the spec does not say how ownership survives a
gateway restart, how retries derive the same id, or how a client-provided id is prevented from naming
another principal's claim.

Required new task `SEC-009 — Bind claims to authenticated principals` (or expand `SEC-002/003`):

- Derive an internal subscription id from principal/tenant, installation, template, and logical feed,
  or persist an authenticated mapping transactionally.
- Never forward a raw client-chosen engine subscription id.
- Authorize renew/release/replacement and stream capability minting against that binding.
- Test stolen subscription+shape pairs, gateway restart, lost create/release responses, tenant switch,
  and shared identical definitions. Unauthorized attempts must make no catalog event.

## Medium findings

### M1. `ENG-005` hot table discovery conflicts with fixed circuit structure

Affected tasks: `ENG-005`, `OPS-008`, `TST-005`.

The deliverable includes installing “circuit input” at runtime, but counts circuit structure is built
once from boot-resolved schemas; unknown configured tables are skipped
(`apps/engine/src/engine/mod.rs:893-949`). Repository design explicitly requires new templates/layouts
to rebuild and reseed the circuit. Split the task: hot discovery may install dynamic routing/fallback
state under resolve locks, while a table needed by a compiled counts pipeline must trigger a controlled
rebuild/restart. The exclusion branch should be a validated restart requirement, not a silent skip.

### M2. `ENG-011` orphan GC lacks leadership and namespace dependencies

Affected tasks: `ENG-011`, `ENG-013`, `ENG-014`, `ENG-015`, `OPS-002`.

Listing `shape/*` and comparing with one catalog is unsafe during failover, restore, or catalog
generation switching. The “creation/retirement fence” deliverable is necessary but insufficient unless
GC runs only under the same storage-enforced leadership token and against one verified catalog/storage
generation. Add dependencies on corrected `ENG-013`, restore binding, and catalog compaction.

### M3. `OPS-008` could accidentally weaken the schema-drift invariant

Affected tasks: `OPS-008`, `ENG-004`, `ENG-005`.

The acceptance says additive/type/rename fixtures may “stay live safely,” but current policy retires on
any fingerprint mismatch; it does not tolerate additive drift. State explicitly that the default result
for any current fingerprint change is typed retirement/reseed. Staying live requires a separate ADR and
proof that every schema holder, decoder, circuit layout, projection, and backfill/live representation
can change atomically. Do not let a deployment task introduce additive tolerance implicitly.

### M4. Several acceptance criteria quantify an undefined set

Affected tasks: `TST-003`, `TST-005`, `CAP-004`, `OPS-002`, `OPS-004`.

“Every cut point,” “each supported backup type,” “every new pure-property family,” and “all failover cut
points” are not machine-checkable until their universe is checked in. Add versioned manifests listing
the exact cut-point ids, backup/restore matrix, seed families, and expected terminal state. CI should
fail when code declares a new cut point that is absent from the manifest. This retains fixed actionable
tests without substituting calendar monitoring.

## Proposed concrete edits and new tasks

| Spec area | Concrete change |
| --- | --- |
| `ENG-013` | Replace advisory-lock-only wording with storage-enforced monotonic fencing on every DS mutation, or explicitly narrow HA to STONITH/manual promotion and delete the partition acceptance. |
| New `ENG-014` | Detect DS rollback relative to slot progress before restore, or force an epoch reset after every DS restore. Add a fixed asymmetric-restore matrix. |
| New `ENG-015` | Add crash-safe catalog snapshot/compaction plus bounded resident writer state; preserve EIDs, max shape id, leases, checkpoint/highwater, rotations, epoch, and pending retirement. |
| New `ENG-016` | Bound retained derived state under live growth and define spill/retire/fail-closed behavior that never drops a committed effect. |
| `ENG-001` | Cover initial, replacement, and load-more pages; direct and deferred/subquery emissions; source-xid semantics; and composite-PK rejection or implementation. |
| `ENG-002`/`PROTO-003` | Define per-stream non-empty transaction framing and capability negotiation. Exclude subquery streams or redesign deferred delivery through a transaction-aware arbiter. |
| `ENG-007` | Include sequencer hold/output spill and all lossless queues: flip, emission, catalog, retirement, and sequencer command/create control. State which can block, spill, coalesce, or reject before mutation. |
| `ENG-010` | Define reclaim order. Input log/catalog cannot be evicted; a registered output stream may be retired only after durable `Dropped`; if catalog cannot accept that intent, the source transaction stays unadvanced. |
| `ENG-004` | Persist/reseed a per-dependent gate, add catalog migration fixtures, and align subquery acceptance with drop-on-restore unless subquery persistence is added. |
| New `SEC-009` | Make gateway claim ownership durable/derivable and principal-bound; authorize every renewal, release, replacement, and stream capability. |
| New `TST-000` | Repair the purge contract/test mismatch and record a green baseline before release-gate work. |
| `TST-003` | Add a checked-in cut-point manifest and explicit assertions for catalog EID dedup, post-durability recheck, lease ages, max-id preservation, close-before-delete, unmatched retirement, durable checkpoint deletion floor, and missing-start-segment refusal. |

## Dependency corrections

Use these one-way edges:

```text
PROTO-001 -> PROTO-003-design -> ENG-002 -> PROTO-003-fixtures -> PROTO-004/TST-002

GOV-002 -> ENG-008/ENG-009 -> SEC-007
GOV-002 -> ENG-010 + ENG-014 + ENG-015 -> OPS-002
OPS-001 + fencing-design -> ENG-013 -> OPS-004
ENG-013 + ENG-014 + ENG-015 -> ENG-011
ENG-015 -> OPS-005
TST-000 -> TST-001 -> every release/cutover gate
```

Specific task edits:

- Remove `PROTO-003` from `ENG-002`'s dependency, or split `PROTO-003` into design and post-implementation
  fixtures. Do not leave a mutual edge.
- Remove `OPS-002` from `ENG-010`; make `OPS-002` depend on `ENG-010`, `ENG-014`, and `ENG-015`.
- Remove `OPS-004` from `ENG-013`; make `OPS-004` depend on the implemented fence.
- Remove `SEC-007` from `ENG-008` and `ENG-009`; the capacity manifest defines engine budgets, engine
  tasks implement them, and `SEC-007` exposes corresponding tenant/global admission.
- Make `ENG-011` depend on corrected leadership, restore-generation, and catalog-generation work.
- Make `OPS-005` depend on the catalog snapshot/compaction format and include `ENG-004`'s persisted gate.
- If transaction-atomic delivery is optional, remove `ENG-002`/`PROTO-003` from universal G5/G6/G7 and
  gate it per template/capability. If it is mandatory, remove the contrary first-release exclusion and
  require it in `SWF-005` rather than “if enabled.”

## Acceptance-test corrections

1. **`ENG-001`:** replace the unactionable “holds the PostgreSQL internal commit window” sentence with
   a named test mechanism. Required matrix: initial/replacement/load-more x insert/update-in/
   update-out/delete/reorder x simple/subquery feed x direct/overlap/deferred delivery. Include one
   synthetic `SnapshotGate` test for every xmin/xmax/xip class and a fixed concurrent E2E corpus.
2. **`ENG-002`:** assert markers only for a non-empty projected transaction on a single stream.
   Backfill envelopes and no-output source transactions have no transaction marker. Assert stable
   transaction token uniqueness as `(source position, xid)` or a new opaque id, not raw `txid` alone.
   Run separate cases for plain, routed, fold aggregate, circuit aggregate, and—only if supported—
   deferred subquery output.
3. **`ENG-003`:** on a native 404/410, first renew/recreate with the same principal-bound subscription.
   If the same handle returns, treat the data-plane result as false and retry; reset only on a replacement
   handle or authoritative retirement. This makes the “false proxy 404” acceptance executable.
4. **`ENG-004`:** require survival for restorable tiers only. For a subquery present before the crash,
   require one typed retirement/reset under the current catalog policy; do not assert it remains live.
5. **`ENG-007`:** drive transactions at `cap-1`, `cap`, `cap+1`, 2x memory cap, 2x output append cap,
   and a fixed high-fan-out case. Every committed transaction must land or the engine must remain
   fail-closed and unadvanced; “rejected because too large” is not an accepted result.
6. **`ENG-010`/`OPS-002`:** for catalog disk failure, assert no shape batch is discarded merely because
   its required `Dropped` cannot be recorded. For restore, enumerate DS-only rollback, PG-only rollback,
   coordinated rollback, missing slot, lost slot, and same-slot-ahead cases with the pre-readiness result.
7. **`ENG-013`/`OPS-004`:** make the test observe DS's committed fencing token for every write class,
   not merely readiness or row convergence. Pause the old engine after it has queued a catalog and shape
   append, promote, resume it, and assert DS rejects both stale writes.
8. **`TST-001` purge:** success means durable intent plus eventual close/delete unless the implementation
   contract is changed. Poll retirement to a fixed bound; do not require immediate stream 404.
9. **`TST-003`:** replace broad category prose with a versioned cut-point manifest. At minimum include
   input chunk append/ack, output append/checkpoint, highwater-only checkpoint, rotation pointer/close/
   catalog record, durable-offset/segment delete, Created/Joined/Left/Dropped/Retired append-response
   loss, post-durability purge/TRUNCATE/epoch races, and catalog snapshot-generation switch.
10. **`ENG-016`/capacity:** begin below the retained-state limit, cross it through committed live DML,
    and assert exact oracle state or a durable typed retirement/fail-closed boundary. A seed rejected
    before admission does not test the dangerous case.

With these changes, the draft can become an executable production plan while preserving the fork's
strongest property: it never converts unavailable dependencies, replay, or lifecycle races into a
silently stale registered shape.
