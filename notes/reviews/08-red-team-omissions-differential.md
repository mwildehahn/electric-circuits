# Red-team omissions differential: production-readiness and Swift migration spec

Reviewed against local `474577a` on 2026-08-22. Scope: the complete repository `AGENTS.md`, the
proposed execution spec, every supporting research note `01`–`15`, and the implementation paths cited
below. This is a hostile architecture/specification review, not a test run.

## Verdict

**No-go as written.** The draft identifies most visible work, but its task graph can still certify an
unsafe release. The highest-risk gaps are:

1. a restricted or malformed existing PostgreSQL publication can omit live changes while boot and
   snapshots succeed;
2. restoring durable-streams behind an already-advanced PostgreSQL slot can silently remove the only
   copies of acknowledged changes;
3. the stated PostgreSQL 16 active/passive outcome has neither a safe logical-slot failover story nor
   a split-brain-capable output fence; and
4. the proposed output transaction marker ignores asynchronous membership query-backs that can emit
   later and out of source-transaction order.

The plan also has hard dependency cycles, lacks a durable public principal-to-subscription binding,
and contradicts the repository's “a committed source transaction is never too large” invariant.
These are launch blockers, not residual risks to observe after release.

Severity below is ranked by launch impact. `P0` means the production outcome or a non-negotiable gate
can be false while its listed acceptance criteria pass. `P1` means the plan is materially incomplete
or non-executable. `P2` means a required product disposition is missing.

## P0 findings

### 1. Existing-publication adoption can silently stale a selected table

**Differential.** The outcome promises PostgreSQL 16 with a dedicated publication and G3/G6 promise
that WAL cannot be silently skipped ([spec lines 31–39, 61–73](../16-production-readiness-and-swift-migration-spec.md)).
`OPS-003` checks only a hand-made **column-list** publication, missing slot, plugin, and cluster
identity. It does not require that every selected table is actually in the publication, that the
publication has no row filters, or that its `publish` operations include insert/update/delete/truncate
([spec lines 717–734](../16-production-readiness-and-swift-migration-spec.md)).

The source adopts any publication with the expected name. `ensure_publication` returns when the name
exists, while `inspect_publication` only looks for `prattrs`; it never proves selected-table coverage,
`prqual` absence, or operation flags ([`pg.rs` lines 709–780](../../apps/engine/src/pg.rs)). A shape on
an omitted or row-filtered table can backfill successfully from Postgres and then never receive some
live changes. The existing tests can therefore report a healthy snapshot and readiness while serving
permanently stale data.

**Required spec change.** Add a blocker `ENG-014 — Validate the publication as a complete change
source`, before `OPS-003`, with these mandatory checks:

- every selected table is explicitly covered, directly or by a deliberately supported schema/all-
  tables rule;
- no publication row filter is present on a tracked table;
- no column list is present;
- insert/update/delete/truncate are all enabled;
- partition-root behavior is selected and fingerprinted;
- the publication identity/configuration is included in the epoch/support manifest; and
- any runtime publication change raises a fail-closed generation change and retires affected shapes.

Make the production path prefer an admin-created explicit table publication. The runtime engine role
must be able to use it without `CREATE PUBLICATION`, table ownership, or `ALTER TABLE`; setup-time DDL
belongs to the bootstrap role.

**Fixed test.** Pre-create seven PG16 fixtures: missing tracked table, extra untracked table, row
filter, column list, `publish='insert'`, partition-root on, and a valid whole-row/all-operation
publication. Boot each fixture three times. The first, third, fourth, and fifth must fail before
readiness or any shape/catalog mutation; the valid fixture must process exactly 10 transactions of
100 insert/update/delete/move-out operations and equal the SQL oracle. Mutate each publication after
readiness and assert a typed retirement/refusal before the next affected shape read can return
success.

### 2. DS restore versus an advanced slot has an undetectable acknowledged-change gap

**Differential.** G3 says catalog/change/shape storage survives restore and the slot/catalog epoch
cannot silently skip WAL. `OPS-002` says to “record the relationship” and refuse a mismatch, but it
does not define any cross-store witness or reset protocol that makes that possible
([spec lines 66–67, 698–715](../16-production-readiness-and-swift-migration-spec.md)). The operations
research explicitly says no backup mechanism exists and that catalog, changes, and shape streams
must be mutually consistent ([operations note, finding 8](../05-operations-and-sre-readiness.md)).

A concrete unsafe sequence is currently indistinguishable from normal boot:

1. snapshot DS at catalog/checkpoint `C1`;
2. append and acknowledge source changes through `C2`, then let the PostgreSQL slot advance;
3. restore DS to `C1` while keeping the live PostgreSQL cluster/slot at `C2`;
4. boot with the same system identifier, slot name, plugin, and `SlotBound` epoch.

The older DS copy no longer contains `C1..C2`, and the slot need not retain it. The current epoch
verdict accepts the same system identifier and merely records the slot's current confirmed-flush LSN
([`epoch.rs`/`pg.rs` observation](../../apps/engine/src/engine/epoch.rs),
[`pg.rs` lines 665–706](../../apps/engine/src/pg.rs)). An old backup's `SlotBound` is not evidence that
its stream contents reach the live slot position.

**Required spec change.** Split `OPS-002` into:

- `DSR-001`: define an online-consistent DS snapshot API/manifest containing storage generation,
  catalog tail hash/eid, every included stream tail/checksum, current change-log position/highwater,
  epoch, and source LSN;
- `DSR-002`: implement a monotonic cross-store recovery witness that cannot roll back with the DS
  volume, or require a coordinated PostgreSQL physical restore whose logical-slot state is proven not
  ahead of the DS manifest; and
- `DSR-003`: define the only fallback: fail before readiness and perform an authorized whole-epoch
  reset/full client rehydrate. Never attempt a same-epoch resume when the proof is absent.

G3 must say which acknowledged promise survives the configured backup RPO. If the product chooses
reset rather than zero-RPO preservation, the gateway's durable public-handle generation must also be
invalidated so every client receives an explicit reset instead of a 404/stale success.

**Fixed test.** Create one shape and commit transactions `T001..T100`, each changing 100 distinct
rows. Snapshot DS after `T020`; wait until the slot and clients acknowledge `T100`; restore the DS
snapshot without rewinding PostgreSQL. Repeat 100 times at each of five cuts: after change-log
append, after shape append, after catalog offset, after replication feedback, and during snapshot
finalization. Every run must either reconstruct all `T001..T100` or refuse readiness and issue one
generation-wide reset. Serving a shape missing any of `T021..T100` is an unconditional failure.

### 3. PostgreSQL 16 failover and the proposed advisory lock do not provide the claimed fence

**Differential.** The first release names PostgreSQL 16 and active/passive recovery; `OPS-004`
explicitly includes PostgreSQL primary failover. `ENG-013` proposes a PostgreSQL advisory lock and
claims the old leader cannot append after losing it ([spec lines 31–36, 657–675, 736–753](../16-production-readiness-and-swift-migration-spec.md)).
The accepted epoch ADR instead says timeline/failover handling is deferred, and the code treats a
timeline change as warning-only ([ADR-0004](../../docs/adr/0004-slot-epoch-and-reset.md),
[`epoch.rs` lines 384–406](../../apps/engine/src/engine/epoch.rs)).

PostgreSQL 17 introduced synchronized logical failover slots; PostgreSQL 16 does not provide that
automatic failover-slot mechanism. See the PostgreSQL 17
[logical-replication failover documentation](https://www.postgresql.org/docs/17/logical-replication-failover.html)
and [release notes](https://www.postgresql.org/docs/17/release-17.html). More importantly, an advisory
lock on the old primary cannot fence an engine partitioned from that primary but still connected to
DS. After promotion, both database timelines can believe they own a local lock. A stale engine may
still append buffered output/catalog mutations to the shared DS namespace.

**Required spec change.** Make an explicit product choice before implementation:

- **PG16 first release:** exclude non-destructive PostgreSQL-primary failover. Any timeline change is
  an epoch break that fails closed and requires an authorized reset/full rehydrate. Active/passive
  means engine-process replacement against the same primary only.
- **Seamless primary failover:** raise the floor to PG17+, create failover-enabled slots, configure
  and verify slot synchronization/readiness, and add a storage-enforced monotonically increasing
  fencing token to every catalog/change/shape mutation. DS must reject stale tokens; checking an
  advisory lock only before the append is insufficient.

Replace the `ENG-013`↔`OPS-004` dependency cycle with `LEAD-001` ADR → `ENG-013` fencing primitive →
`OPS-004` orchestration. Bind gateway routing and readiness to the same leader generation.

**Fixed test.** In 100 seeded runs, pause the old engine after it has buffered one source transaction
but before each DS append phase; partition it from the old primary while leaving DS reachable;
promote the standby and new engine; then resume the old process. Exactly one fencing generation may
append. Audit raw DS records, not only folded state, and assert zero stale-token catalog, change-log,
shape, checkpoint, and retirement writes. For the PG16 exclusion branch, repeat 100 promotions and
assert that the first observed timeline change yields no successful shape read and exactly one typed
epoch reset boundary.

### 4. `ENG-002` can stamp `last` before deferred subquery emissions exist

**Differential.** `ENG-002` is scoped to the sequencer/output layer and tests generic one/many shapes
and chunks ([spec lines 454–473](../16-production-readiness-and-swift-migration-spec.md)). The source
has a second output pipeline: membership flips are handed to an unbounded dispatcher, query-backs run
concurrently, and resulting batches are enqueued into independent ordered-emission lanes
([`mod.rs` lines 270–378](../../apps/engine/src/engine/mod.rs),
[`emission.rs` lines 1–85](../../apps/engine/src/engine/emission.rs)). The sequencer flushes its direct
`txn_pending` map and proceeds to the next source transaction while flip work is still outstanding
([`sequencer.rs` lines 650–760 and 1538–1566](../../apps/engine/src/engine/sequencer.rs)). Current
output helpers correctly have `last: None`, including absolute, row, delete, and aggregate envelopes
([`output.rs` lines 80–212](../../apps/engine/src/engine/output.rs)).

Simply setting `last=true` on the final envelope currently known to `txn_pending` would lie: a delayed
inner-set flip may later emit more envelopes bearing the same txid, potentially after a subsequent
transaction. This defeats both the observer-atomicity claim and crash/resume buffering.

**Required spec change.** Expand `ENG-002` into a transaction-output coordinator spanning direct
routing, circuit aggregates, subquery node reconciliation, deferred query-backs, and emission lanes.
It must:

- register every async child before the source transaction can close;
- preserve per-stream source-transaction order, not merely per-pk eventual convergence;
- append `last` only after all children that can emit to that stream have resolved;
- define a durable/spillable representation for a child that survives crash or else hold the source
  checkpoint before it; and
- define retirement as the only permitted substitute for landing the complete transaction.

Add subquery/null-sensitive/deferred-create cases to `PROTO-003`, `TST-003`, and `CAP-004`; the
current “one/many shapes” language is too weak.

**Fixed test.** Seed 10,000 outer rows and two nested membership levels. In one source transaction,
flip an inner value, update an outer candidate, and change an aggregate; block each query-back after
its repeatable-read snapshot. Commit a second transaction on the same outer keys, then release the
blocked work. Repeat at 100 scheduler seeds. For every affected stream, raw envelopes for transaction
1 must be contiguous, exactly one actual final envelope must carry `last=true`, no transaction-2
envelope may precede it, and crash/restart after every append boundary must produce one local
transaction effect or one typed reset.

### 5. Tenant-safe query construction is specified, but lifecycle capabilities are not bound to a principal

**Differential.** G2 promises no cross-tenant stream or subscription access. `SEC-003` protects query
definitions and `SEC-004` protects stream reads, but no task defines the durable public object that
binds `{tenant, subject/session, template version, engine shape, engine subscription, stream path}`
for create, renew, read, release, replacement, and revocation. `SEC-002` merely asserts that a gateway
restart will not duplicate claims ([spec lines 65–66, 294–348](../16-production-readiness-and-swift-migration-spec.md)).

The engine cannot supply this security property: subscription strings are caller-controlled and
globally interpreted but explicitly have no provenance; shape IDs/paths are returned directly and
are not capabilities ([`http.rs` lines 187–260](../../apps/engine/src/http.rs)). The security research
correctly required a gateway-owned opaque handle mapping, but that design was lost in synthesis
([security note, “shape IDs, leases and purge”](../12-security-and-multitenancy.md)). A stateless pass-
through gateway can therefore authenticate a user yet still let a replayed internal shape/subscription
identifier renew, release, or read another principal's claim.

**Required spec change.** Add blocker `SEC-009 — Own public synchronization capabilities`:

- mint a high-entropy public feed ID server-side; never accept or expose the internal shape ID,
  stream path/URL, or engine subscription on public routes;
- durably bind it to tenant, subject/session/device, template+version, internal claim, generation,
  and authorization expiry;
- authorize every renew/read/release/replacement against that binding;
- define gateway replication/HA and atomicity between mapping persistence and the engine mutation;
- rotate the internal claim safely after a lost response; and
- specify an exact maximum revocation/capability TTL in the support matrix. `leaseSeconds=0` must be
  rejected in production because it disables the server-side liveness repair.

`PROTO-001` must describe only the public opaque ID. Internal DS paths and engine subscription IDs
belong in a separate private contract.

**Fixed test.** Use two tenants, ten principals per tenant, five templates, and 100 live public feeds.
For each create/renew/read/release/replace operation, execute 100 guessed-ID, stolen-ID, wrong-tenant,
wrong-subject, old-generation, and lost-response attempts before and after gateway restart. Forbidden
attempts must produce zero catalog/stream/PG mutation. With an injected clock, test exactly at
`TTL-1`, `TTL`, and `TTL+1` ticks after role revocation and logout; no read or renewal may succeed at
or after the declared bound, and an authorized client must resume the same opaque offset identity
through credential refresh.

### 6. “Bound `txn_pending`” contradicts the no-transaction-too-large safety invariant

**Differential.** G4 asks for transaction limits and `ENG-007` says to bound in-memory fan-out with
backpressure. `CAP-004` tests only through a “declared max” and its acceptance permits work to be shed
or rejected ([spec lines 67, 551–566, 915–933](../16-production-readiness-and-swift-migration-spec.md)).
But a PostgreSQL transaction is already committed before the engine learns its output size. Rejecting
or dropping it is not admission control. The repository invariant is explicit: transaction size
never invalidates a transaction; the ingestor spills, while the sequencer held run and `txn_pending`
remain transaction-sized (`AGENTS.md`, large-transaction invariant). The current sequencer constructs
the full per-stream `HashMap<String, Vec<Envelope>>` before flushing.

Backpressure alone cannot bound this state: it can stop reading more WAL, but it cannot make the
already-committed transaction smaller. Retiring a shape because a commit exceeded a configured size
would also violate the stated invariant and turn a capacity limit into mass client resets.

**Required spec change.** Replace “bound `txn_pending`” with a spillable, transaction-scoped output
journal whose memory is capped and whose disk reservation/accounting is established before replaying
the transaction. Specify:

- memory cap, per-transaction scratch accounting, global scratch reserve, checksum/recovery, and
  cleanup after crash;
- one-pass chunked fan-out that still discovers the true per-stream final marker;
- disk-full behavior that halts before checkpoint/slot advance and resumes from durable input; and
- that `declared_max_transaction_bytes` is a capacity claim, not a correctness refusal threshold.

`ENG-007`, `ENG-010`, and `CAP-004` must all state that no source transaction is rejected or silently
retired for size after PostgreSQL commit.

**Fixed test.** With the default 128 MiB ingest cap, execute transactions of 127, 128, 129, and
512 MiB ten times each, each fanning out to the declared maximum shapes and including subquery and
aggregate output. Hold DS for exactly ten append attempts, then release it. Assert RSS stays below
the manifest memory cap, scratch stays below its reserved bound, checkpoint/slot do not pass the
transaction early, and every client equals the SQL oracle. Repeat with scratch exhaustion injected
at each 64 MiB boundary; the engine must remain unready/fail closed and converge after space returns,
without size-based retirement.

### 7. The dependency graph is cyclic and several tasks are impossible to accept in their assigned wave

**Differential.** The spec says every packet fits one primary subagent/PR and tasks in a wave run only
after dependencies ([spec lines 93–114, 1508–1521](../16-production-readiness-and-swift-migration-spec.md)).
The declared graph has hard cycles and reverse-wave dependencies:

- `PROTO-003` depends on `ENG-002`; `ENG-002` depends on `PROTO-003`.
- `SEC-007` depends on `ENG-007`–`ENG-010`; `ENG-008` and `ENG-009` depend on `SEC-007`.
- `ENG-010` depends on `OPS-002`; `OPS-002` depends on `ENG-010`.
- `ENG-013` depends on `OPS-004`; `OPS-004` depends on `ENG-013`.
- `PROTO-001` acceptance requires Swift to consume fixtures, but `SWF-001` depends on `GOV-004`,
  which depends on `PROTO-001`.
- `SWF-002` and `TST-002` each depend on the other side producing/consuming the shared corpus.
- `CMP-005` depends on `MIG-001`, yet the waves schedule `CMP-005` before `MIG-001`.
- `SWF-001` depends on `GOV-004`, yet the waves schedule `SWF-001` two waves before `GOV-004`.

These are not editorial nuisances: no assignee can satisfy the completion contract without merging
work whose declared prerequisite is unfinished.

**Required spec change.** Replace the prose dependencies with a machine-readable DAG and split
contract/design tasks from implementation/conformance tasks. At minimum:

1. protocol schema/ADR → engine implementation → language bindings → cross-language conformance;
2. quota/accounting contract → engine primitives → gateway enforcement → mixed-tenant test;
3. storage accounting primitive → backup manifest/witness → enforced recovery policy;
4. leadership ADR/token contract → engine/DS fence → failover orchestration; and
5. shadow harness core → compatibility shadow run.

Move `GOV-004` before Swift scaffolding or remove it as a scaffold prerequisite. Make fixture
generation its own task before any language is required to consume it.

**Fixed test.** Add a CI script that parses every task ID, `Depends on`, wave, and gate closure. It
must reject unknown IDs, cycles, a task scheduled before any prerequisite, a gate whose closing set
contains an explicitly disabled feature, and an acceptance criterion that names an artifact from a
later task. Check in one intentionally cyclic fixture and one reverse-wave fixture and assert both
fail.

## P1 findings

### 8. Durable-streams is an unowned production database dependency, not just an engine component

**Differential.** The outcome makes file-backed DS with backup/restore authoritative for catalog,
change log, and shape output. Yet governance/versioning covers engine images, native protocol, TS,
and Swift—not the DS server's storage format, image, API, or support policy
([spec lines 31–42 and `GOV-004`](../16-production-readiness-and-swift-migration-spec.md)). The local
wrapper downloads `durable-streams` 0.1.5 from crates.io at runtime/build time and offers only test
start/stop behavior ([`packages/ds-rust`](../../packages/ds-rust/src/index.ts)). The operations note
records that no DS backup/list mechanism exists and the DS image is not in the current publish
workflow ([operations note findings 8 and 13](../05-operations-and-sre-readiness.md)). `ENG-011`
explicitly depends on a “DS list/index capability” that is not a task ID.

**Required spec change.** Add `GOV-005/DS-001` to name the owner and supported DS version/storage
format; pin source commit and image digest; build/sign the DS image in the release; inventory upstream
API/format guarantees; and define upgrade, rollback, corruption, backup, list, and security behavior.
Turn “DS list/index capability” into a real predecessor task with its own protocol/auth/compatibility
fixtures. Add DS server and format versions to the release evidence and support matrix.

**Fixed test.** For the current and previous supported DS versions, generate 1,000 streams containing
10,000 total appends plus close/delete states, restart at 100 enumerated WAL write cuts, and verify
byte-identical reads/HEAD/list results. Upgrade and roll back the fixture; the old binary must either
read it exactly or refuse before mutation. The release workflow must fail when the DS image, SBOM,
signature, source digest, or format fixture is absent.

### 9. PostgreSQL TLS and restricted-publication work is only implied, leaving public upstream #14 unmapped

**Differential.** `SEC-006` explicitly asks for CA/client-certificate configuration in the Rust **DS**
client, but does not assign the analogous PostgreSQL query and replication implementations. The
source uses `NoTls` for ordinary connections and the separate replication library uses its default
TLS config ([`pg.rs` lines 25–47](../../apps/engine/src/pg.rs),
[`replication.rs` lines 322–341](../../apps/engine/src/replication.rs)). This is the still-open public
upstream [`electric-sql/electric-circuits#14`](https://github.com/electric-sql/electric-circuits/issues/14),
not parent-fork #14 (which is the walsender timeout and is mapped to `ENG-006`). `OPS-003` saying
“TLS” and `ENG-006` testing a missing CA do not identify which connector owns the work.

The same packet also omits encryption at rest for DS volumes and transaction-spill/scratch files,
which contain replicated row data. Backup encryption alone is not a live-volume control.

**Required spec change.** Add a separate `SEC-006A — PostgreSQL transport` covering both pool/query
and walsender connections with the same `verify-full` hostname/CA/client-cert semantics, rotation,
redaction, and configuration parser. Add `SEC-006B — Data at rest` covering DS PVs, spill/scratch,
backup objects, keys, rotation, restore, and deletion. State whether a service mesh is the selected
implementation; “equivalent private transport” must name and test one concrete topology.

**Fixed test.** Run both PostgreSQL connection families through seven fixtures: trusted CA, untrusted
CA, wrong hostname, expired server cert, required-but-missing client cert, rotated CA overlap, and
plaintext listener. Each negative case must fail before readiness. During rotation, execute exactly
1,000 transactions and 100 backfills while replacing each connection once; all 100,000 row effects
must converge. Mount an encrypted DS/spill volume, write a known 1 MiB sentinel row, and prove neither
the detached block snapshot nor a failed spill file contains the plaintext sentinel.

### 10. Native Swift requires durable identity/offset persistence before the optional sink exists

**Differential.** The product says local persistence is optional. `SWF-004` nevertheless requires
persisting subscription identity before tailing, and `SWF-005` requires persisting offsets after
acknowledgement. The only storage abstraction is introduced later and conditionally by `SWF-007`
([spec `SWF-004`–`SWF-007`](../16-production-readiness-and-swift-migration-spec.md)). A dependency-free
actor cannot durably persist across process death by itself. The package can be ephemeral, or it can
resume durably through an application store; it cannot promise both without an explicit state-store
contract.

The iOS 18/macOS 15 floor is also embedded before `CMP-001` inventories the real application and
fleet. It is copied from the sibling package research, not validated as a migration requirement
([Swift strategy](../09-swift-library-strategy.md)). This may make the chosen app unable to adopt the
library even if protocol work succeeds.

**Required spec change.** Before `SWF-004`, add `SWF-003A` defining a tiny transactional
`SubscriptionStateStore` for `{publicFeedID, definition hash, generation, handle, acknowledged
offset, reset state}`. Define two modes:

- ephemeral mode: no persisted identity/offset, always mint and fully resnapshot after process death;
- durable mode: state and sink effects commit atomically through the supplied transaction boundary.

Make `SWF-007` depend on and extend that protocol rather than introducing persistence after the
lifecycle actor. Move the platform floor into `CMP-001/GOV-002`; require a fleet/deployment-target
decision before it becomes a supported-release fact.

**Fixed test.** Run 10,000 lifecycle traces in both modes. Kill the process after state reservation,
server create, snapshot envelope, sink apply, offset write, replacement reservation, and release.
Ephemeral mode must always create a new public ID and full snapshot; durable mode must expose either
the pre-commit or post-commit generation, never a row/offset split. Compile the package against the
actual application deployment target selected by `CMP-001`; CI must reject a package floor above it.

### 11. Gates are unconditional even where the feature tasks say “if supported”

**Differential.** G0–G10 must all close before production, but G6 unconditionally names
`ENG-001`–`ENG-006` and G7 names every `SWF-001`–`SWF-013`. The task text says dynamic table discovery
may instead be excluded, aggregates are required only if inventory needs them, and subsets are gated;
the first release exclusions likewise allow no native subsets ([spec lines 44–55, 61–73,
`ENG-005`, `SWF-008`, `SWF-009`](../16-production-readiness-and-swift-migration-spec.md)). Conversely,
G1 includes `SEC-008`, which depends on native Swift `SWF-012`, so a compatibility-only production
pilot cannot close isolation until an unshipped native client is complete.

Without release profiles, teams will either perform the entire multi-product program before any
launch or waive tasks informally. Informal waivers are exactly how an unsafe feature escapes a gate.

**Required spec change.** Add a machine-readable `release-profile.yaml` generated by `GOV-002` with
booleans for `/v1`, native shape, aggregate, subset, dynamic selectors, PG failover, capability/direct
DS, and each platform. Gates close the dependency closure of enabled features. Every disabled feature
must have a protocol/configuration negative test and a support-matrix exclusion; it may not merely
remain unfinished.

**Fixed test.** Check in three profiles: compatibility-only, native-shape+aggregate, and full subset.
The DAG checker must derive exact task sets and reject (a) aggregate traffic when aggregate is off,
(b) subset negotiation before `ENG-001/SWF-009`, (c) dynamic selector reload when excluded, and
(d) a compatibility profile missing gateway/tenant/durability/capacity tasks. Run 1,000 rejected
requests per disabled capability and assert zero catalog/stream/PG mutations.

### 12. Several acceptance clauses can pass without a reproducible safety claim

**Differential.** The spec admirably rejects calendar monitoring, but still uses undefined phrases:
“material throughput collapse,” “same result,” “every scenario,” “declared capacity/error budget,”
“no unbounded positive slope,” “current and previous supported,” and “every possible boundary.” The
capacity manifest does not require statistical method, sample cadence, exact error budget, warm-up
cut, slope tolerance, or comparison margin ([spec lines 110–114, `CAP-001`–`CAP-004`](../16-production-readiness-and-swift-migration-spec.md)).
Three repetitions do not by themselves define a pass. A flat folded row map can also hide duplicate
raw events and observer-visible partial transactions.

**Required spec change.** Extend `capacity-target.yaml` with:

- exact workload seeds and distributions;
- warm-up operation count and sample-every-N-operations cadence;
- p95/p99/p999/max thresholds and allowed per-run variance;
- `max_rss_residual_bytes_per_mutation`, `max_fd_slope_per_million`,
  `max_lag_slope_bytes_per_mutation`, and retained-data normalization;
- tenant error/latency isolation thresholds;
- exact RPO/RTO in operations and seconds; and
- raw-event duplicate/transaction-boundary assertions in addition to final-state comparison.

Require every “all/every/current/previous” set to be enumerated in a checked-in manifest. Replace
“unexplained divergence” with “zero divergence,” plus a separate allowlisted representation-difference
file reviewed by task ID.

**Fixed test.** For the 10,000,000-mutation run, sample after every 100,000 mutations; discard exactly
the first 1,000,000 as warm-up; compute Theil–Sen slopes over the remaining 90 samples after
subtracting the declared retained-data model; compare to the explicit manifest thresholds. Run the
three fixed seeds listed in the manifest. Promotion fails if any raw envelope is duplicated beyond
the protocol's declared replay semantics, if a `last` group is partial, if any threshold fails in any
run, or if a workload dimension differs between candidate and baseline.

### 13. Several “one subagent/PR” packets are programs, creating shallow-acceptance pressure

**Differential.** The completion contract says every task fits one primary subagent/PR, but several
packets cross repositories, storage formats, deployment platforms, and teams:

- `PROTO-001`: design every public endpoint/type plus OpenAPI/schema and Rust/TS/Swift consumers;
- `SEC-002`: implement a new gateway, product IdP integration, credential refresh, persistence, and
  restart correctness;
- `ENG-007`: redesign three lossless queues, transaction fan-out, backpressure, cancellation, and
  metrics;
- `OPS-002`: invent DS snapshot/restore/integrity/cross-store fencing and run crash tests;
- `TST-003`: add failpoints around every durability subsystem and 100 runs per cut; and
- `CAP-003`: execute and analyze the full production capacity matrix.

These cannot be reviewed safely as single coherent PRs. The likely outcome is a paper deliverable or
a test that covers only the happy path while the task is marked complete.

**Required spec change.** Split each into design/contract, implementation slices with one principal
file boundary, focused regression, and integration/evidence tasks. A task may close only artifacts it
directly owns. Specifically split `PROTO-001` by lifecycle, stream, subset, values/keys, and schema;
`SEC-002` by identity validation, public-handle store, policy integration, and stream broker;
`ENG-007` by flip queue, emission lanes, transaction journal, and shutdown; `OPS-002` per `DSR-*`
above; and `TST-003/CAP-003` into harness versus execution reports.

**Fixed test.** The DAG manifest must require every work packet to declare one owner, one principal
write boundary, no more than one public contract surface, exact produced artifacts, and a separate
integration task when it spans two runtimes. Seed the validator with the six oversized packets above;
it must reject them until split. This is a structural gate, not a LOC limit.

## P2 findings

### 14. Public upstream caching/CDN issues have no explicit task or support exclusion

**Differential.** `GOV-003` generically promises to map every tracker item, but the executable task set
does not name public upstream
[`#10` (`/v1/shape` HTTP caching)](https://github.com/electric-sql/electric-circuits/issues/10) or
[`#11` (authenticated DS/CDN delivery)](https://github.com/electric-sql/electric-circuits/issues/11).
The source still sends `cache-control: no-store`; the research calls caching/CDN a real remaining
launch decision for high-fan-out compatibility deployments ([upstream issue inventory](../02-upstream-open-issues.md)).
The new gateway/capability topology also changes cache keys and revocation semantics, so “add a CDN
later” is not automatically safe.

**Required spec change.** Add a decision under `GOV-002` with exactly two branches:

- **unsupported at first release:** put edge/CDN caching in the support matrix and require capacity
  evidence for one origin long-poll per active handle at the full reconnect wave; or
- **supported:** add `CACHE-001` for authenticated cache keying, cursor/conditional reads, private/no-
  store error responses, tenant/principal variation, capability expiry, purge/reset invalidation, and
  stampede collapse. Map upstream #10/#11 directly to it.

Also make `GOV-003` emit a machine-readable namespace-qualified mapping. Bare `#14` is already
ambiguous between public-upstream TLS and parent-fork connect timeout.

**Fixed test.** For the unsupported branch, execute exactly the target active-poll count plus the
declared reconnect wave for 1,000,000 completed polls through the gateway and meet the capacity
manifest without any CDN. For the supported branch, issue 10,000 identical authorized polls and
assert the declared single-flight origin count; then run 100 cross-tenant/cache-key substitutions,
100 capability expiries, 100 resets, and 100 error responses, proving no cached body crosses a
principal/tenant/generation or survives its authorization bound.

## Required edit order

The next revision should not add more implementation packets before repairing the plan itself. The
minimum safe edit order is:

1. add the release profile and machine-readable acyclic dependency graph;
2. add `ENG-014`, `DSR-001..003`, the leadership/fencing ADR, and `SEC-009`;
3. rescope `ENG-002` and `ENG-007` around deferred emissions and spillable transaction output;
4. split PostgreSQL TLS/publication setup from DS TLS and add DS ownership/versioning;
5. repair Swift persistence/platform dependencies; and
6. replace ambiguous acceptance language with the fixed manifests and tests above.

Until those edits land, G2, G3, G4, G6, and G8 are not trustworthy launch gates even if every task
currently listed under them is marked complete.
