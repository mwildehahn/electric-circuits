# Performance and boundedness differential review

Scope: independent static review of `notes/16-production-readiness-and-swift-migration-spec.md`
against `notes/14-performance-and-capacity.md`, `notes/04-fork-production-readiness.md`, the memory
and engine-internals documentation, the benchmark/load-generator implementations, and the current
queue, backfill, durable-stream, replication, and sequencer paths. No benchmark was run and no
production number is inferred here.

## Verdict

The draft correctly makes bounded failure and measured capacity launch gates, replaces calendar
soaks with fixed work, and names most known hotspots. It is not yet executable as a production plan.
Four blocker defects remain: the dependency graph contains direct cycles; large transactions are
bounded only in the ingestor and still materialize end to end in the sequencer; durable disk cannot
be bounded while catalog and active shape streams remain append-only; and retained live state can
grow after admission even when every request started below quota. The current benchmark and loadgen
programs also cannot produce the evidence CAP-002--004 require.

The findings below are ranked. Each includes the exact task-level change needed before this spec can
be marked reviewed.

## Ranked findings

### 1. P0 — Four direct dependency cycles make blocker work unschedulable

Evidence:

- `PROTO-003` depends on `ENG-002`, while `ENG-002` depends on `PROTO-003` (spec lines 235--257 and
  454--473).
- `SEC-007` depends on `ENG-007`--`ENG-010`, while `ENG-008` and `ENG-009` depend on `SEC-007`
  (lines 387--404 and 551--619).
- `ENG-010` depends on `OPS-002`, while `OPS-002` depends on `ENG-010` (lines 603--619 and
  698--715).
- `ENG-013` depends on `OPS-004`, while `OPS-004` depends on `ENG-013` (lines 657--675 and the
  `OPS-004` header). The wave table schedules these tasks as though the cycles do not exist.
- `CAP-001` and `OPS-006` both own the same metrics surface. `CAP-001` depends on `OPS-006`, even
  though `OPS-006` is the task that asks the boundedness tasks to expose those metrics. This is not
  a direct cycle, but it creates duplicate ownership and forces instrumentation to wait for the
  dashboards that consume it.

Required task edits:

1. Split `PROTO-003` into `PROTO-003A — Decide stream framing and transaction semantics`
   (depends only on `PROTO-001`) and `PROTO-003B — Publish final fixtures and compatibility tests`
   (depends on `PROTO-003A`, `ENG-002`). Make `ENG-002` depend on `PROTO-003A`.
2. Split `SEC-007` into `SEC-007A — Define quota identities, units, hierarchy, and stable errors`
   (depends on `GOV-002`, `SEC-001`--`SEC-003`) and `SEC-007B — Integrate and prove quota
   enforcement` (depends on `SEC-007A`, `ENG-007`--`ENG-010`). Make `ENG-008` and `ENG-009` depend
   on `SEC-007A`, not `SEC-007B`.
3. Split `OPS-002` into `OPS-002A — Specify storage topology, reserve, accounting, backup, and
   restore contract` (depends on `OPS-001`) and `OPS-002B — Execute backup/restore and disk recovery`
   (depends on `OPS-002A`, `ENG-010`, and the storage tasks added in finding 3). Make `ENG-010`
   depend on `OPS-002A`.
4. Split `OPS-004` into `OPS-004A — Specify active/passive lease and failover protocol` (depends on
   `OPS-001`--`OPS-003`) and `OPS-004B — Execute failover` (depends on `OPS-004A`, `ENG-013`). Make
   `ENG-013` depend on `OPS-004A`.
5. Split `CAP-001` into `CAP-001A — Define bounded-resource metrics and evidence schemas`
   (depends on `GOV-002`) and `CAP-001B — Instrument and validate every bounded resource` (depends
   on `CAP-001A`, `CAP-000`, and the bounded engine/storage tasks). Bounded implementation tasks
   depend on `CAP-001A` for stable names/units. Make `OPS-006` depend on `CAP-001B` and own export
   authentication, dashboards, alerts, and overhead validation.

Acceptance: a machine-readable task graph topologically sorts with no cycle or vague dependency such
as "engine boundedness tasks"; every deliverable has one owning task; the checked-in wave table is
generated from or checked against that graph.

### 2. P0 — `ENG-007` cannot bound a large transaction end to end

The 128 MiB spill cap protects only the replication receiver. Current downstream materialization
includes:

- a durable-stream read body collected as a `String` and then a `Vec<Envelope>`
  (`apps/engine/src/ds.rs:654-659`);
- the sequencer's `held: Vec<Envelope>` for an incomplete transaction and a full complete transaction
  in the read page (`engine/sequencer.rs:387` and the transaction loop around lines 680--756);
- a per-transaction counts `deltas: Vec<_>` and `txn_pending: HashMap<String, Vec<Envelope>>`;
- clones into every pending create buffer (`sequencer.rs:724-727`);
- activation/replay `outs: Vec<Envelope>`; and
- one serialized request body per affected stream. `flush_pending` limits concurrent requests to 32,
  but does not chunk each stream's vector (`sequencer.rs:1663-1678`). A sufficiently large output for
  one stream can exceed the durable-stream 1 GiB request maximum after consuming transaction-scale
  memory.

Backpressure alone cannot fix this: the sequencer is both producer and consumer of `txn_pending`,
and it cannot reject a source transaction already committed in Postgres without violating the
stated large-transaction invariant.

Add `ENG-007A — Bound complete-transaction processing and output staging`.

- **Depends on:** `GOV-002`, `PROTO-003A`, `ENG-006A`.
- **Deliverables:** introduce a byte-accounted disk-backed sequencer transaction spool (or an
  equivalent streaming design) for held input and per-stream output; cap decoded read pages and
  single envelopes; chunk output below both configured and DS hard body limits; preserve
  `(lsn,seq)` de-duplication, per-stream order, transaction-end markers, and the rule that transaction
  N lands or retires all affected shapes before N+1. Include arrangements input, pending-create
  copies, aggregate activation, and dormant replay in the same accounting. Report current/peak
  decoded bytes, spool bytes, transaction bytes/age, output bytes/streams, and scratch free space.
- **Failure policy:** transaction size remains unrestricted for correctness. Exhausted scratch space
  must stop processing/acknowledgement and leave WAL replayable; it must not retire shapes merely
  because the transaction is large. `capacity-target.yaml` may declare the largest *qualified*
  transaction for availability sizing, but that is not an engine admission limit on already
  committed WAL.
- **Acceptance:** run 64 MiB, 128 MiB, `2 x` ingest cap, `2 x` output append cap, one row near the
  supported per-envelope maximum, target maximum, and a transaction larger than available scratch.
  At every input/output chunk and crash cut point: RSS stays below the declared transaction-memory
  bound plus one explicitly measured row/chunk allowance; scratch never exceeds its reservation;
  no partial transaction becomes observer-visible where transaction atomicity is promised; the
  checkpoint and slot never pass unlanded output; a following small commit's head-of-line delay and
  recovery are recorded.

Edit `ENG-007` so it owns asynchronous queues only and depends on `ENG-007A` for transaction staging.
Edit G4, CAP-001, CAP-003, and CAP-004 to name `ENG-007A` explicitly.

### 3. P0 — `ENG-010`'s disk acceptance is impossible without compaction, physical DS accounting, and a reserve

Current shape-byte accounting is process-local serialized payload (`apps/engine/src/ds.rs:237-241`),
not physical WAL/data bytes, and it undercounts after restart. More importantly:

- active shape streams are append-only and cannot be evicted by the existing retention policy;
- `meta/catalog` is append-only, receives lifecycle, lease-renewal, and frequent offset records, and
  is explicitly never compacted (`docs/ivm-engine-internals.md` section 4.5);
- catalog restore keeps every historical `eid` in a `HashSet`, so boot memory and time are O(all
  catalog events ever written) (`engine/catalog.rs:670-676,828-833`); and
- one physical disk may contain DS WAL/data, input segments, catalog, shape streams, transaction
  spill, and DBSP spill. Summing logical payload bytes does not reserve the physical bytes needed to
  write the `Dropped`/`Retired` records that recover from pressure.

An "eviction/refusal policy" cannot safely refuse a committed live append and keep the shape
registered. Without a compaction/replacement protocol, the only correctness-preserving pressure
action for an active shape is a durable typed retirement/reset, and even that needs reserved catalog
and deletion headroom.

Required new tasks:

1. `STO-001 — Add crash-safe catalog snapshot/compaction`.
   - **Depends on:** `PROTO-002`, `GOV-004`, `OPS-002A`.
   - **Deliverables:** versioned folded catalog snapshots carrying shape-id/eid/checkpoint/epoch/
     segment/retirement highwaters; atomic generation switch; reclamation of prior catalog history;
     bounded boot pages and de-dup state; upgrade and rollback rules.
   - **Acceptance:** after at least 10,000,000 lifecycle/checkpoint events, catalog physical bytes and
     boot RSS/time remain within manifest limits; crashes at every snapshot/switch/reclaim write
     restore the same fold; no shape ID or `eid` aliases and no pending retirement disappears.
2. `STO-002 — Bound active shape-history storage`.
   - **Depends on:** `PROTO-002`, `PROTO-003A`, `OPS-002A`.
   - **Deliverables:** choose either stream generations with snapshot-and-replace or explicit durable
     retirement at quota. Define slow/offline consumer behavior and the typed reset boundary. Do not
     claim in-place truncation is safe without a consumer acknowledgement protocol.
   - **Acceptance:** a continuously active shape crosses its logical and physical thresholds in
     fixed-operation tests; it either compacts with resumable generation semantics or retires and
     causes exactly one full refetch; no committed event is silently omitted.
3. `DS-001 — Expose and atomically enforce durable-stream physical budgets`.
   - **Depends on:** `OPS-002A`.
   - **Deliverables:** authenticated stream inventory/size, filesystem free/reserved bytes, WAL/data
     amplification, atomic append reservation, per-class quotas, bounded list pagination, and DS
     fsync/queue/FD metrics. Reserve control/catalog/change-log capacity independently from shape
     output and isolate local spill volumes or account for them separately.
   - **Acceptance:** concurrent appends cannot overbook a quota; restart reconstructs the same usage;
     injected ENOSPC at WAL, data, close, delete, catalog, and spill boundaries leaves the last
     acknowledged contract recoverable.

Then change `ENG-010` to integrate `DS-001`, `STO-001`, and `STO-002`, and depend on them plus
`OPS-002A`. Change `ENG-011` to depend on `DS-001`, not an undefined "DS list/index capability".
Make `OPS-002B` depend on all four, and make `OPS-005` depend on `OPS-002A` and `STO-001` rather than
`OPS-002B`, avoiding a new compaction/migration/storage-recovery cycle. This also gives CAP-003 a
finite disk quantity to test; retained input/output data must be modeled separately from leaks
rather than hidden under "after accounting for retained data."

### 4. P0 — Admission is only initial; retained live state can grow without another request

`SEC-007` and `ENG-009` limit request/snapshot/seed cardinality, but several structures can cross the
limit later under ordinary replicated writes:

- `PkDict` is explicitly append-only O(distinct PKs ever synced) and casts `reverse.len()` to `u32`
  without an exhaustion check (`apps/engine/src/pk_dict.rs:1-15,73-78`). Fixed live row count plus
  insert/delete churn therefore grows RSS forever and eventually risks ID aliasing.
- subquery feed bitmaps grow with current delivered rows, contributor/reverse indexes grow with live
  inner matches, and Electric handle `HashSet`s grow with each handle's live membership.
- dynamic `MIN`/`MAX` aggregates retain a `BTreeMap<Value,i64>` of values per shape.
- counts pipelines retain O(distinct groups), and their boot seed is materialized twice as vectors
  (`pg.rs:1125-1174`, `arrangements.rs:178-184,334-344`). New group keys can also arrive live.

Add `ENG-009A — Bound continuously growing resident state`.

- **Depends on:** `SEC-007A`, `PROTO-002`, `GOV-002`.
- **Deliverables:** byte/cardinality reservations and release accounting for PK interning, feed keys,
  contributor and reverse indexes, Electric handles and per-offset coalescers, aggregate multisets,
  and counts groups. Define a safe action at a live crossing: retire/reset affected shapes, reject a
  new group/shape before mutation where possible, or disable the optional circuit tier and fall back.
  Replace or generation-scope the append-only PK dictionary and explicitly handle `u32` exhaustion.
  Initial seed and live delta must use the same reservation path.
- **Acceptance:** fixed-size tables with at least 10,000,000 unique-PK churn operations, feed move-in/
  move-out, inner contributor churn, high-cardinality MIN/MAX values, and new count groups have a
  bounded owned-byte high-water after removals; crossing each cap produces the documented typed
  reset/fallback and exact SQL-oracle convergence; ID exhaustion is tested with a small injected ID
  space and cannot alias.

Edit `SEC-007B` to enforce both request admission and continuous tenant/global state accounting.
Edit CAP-003 to include churn at fixed live cardinality; a shape-count matrix alone will not detect
this leak class.

### 5. P1 — Create admission can starve correctness-critical Postgres work and hold old snapshots indefinitely

All backfills, subset queries, membership query-backs, and the slot sampler share one semaphore pool.
A native backfill holds its pooled connection and `REPEATABLE READ` snapshot while it awaits every DS
append. Twenty slow creates can therefore consume the default 20 connections, prevent flip
query-backs and observability from running, and grow the very flip/pending queues the spec plans to
bound. `statement_timeout` is off by default and does not bound time spent awaiting DS between cursor
chunks. Query-back paths still `collect()` a hot value or full re-derive into `Vec<Row>`
(`engine/membership.rs:60-85`).

Edit `ENG-008` as follows:

- **Depends on:** `GOV-002`, `SEC-007A`, `ENG-007`, not `SEC-007B`.
- **Deliverables:** reserve separate/fair PG capacity for replication-control/drift, flip/query-back,
  subset, and create work; bound waiters by count/bytes and acquisition deadline; make create permits
  global and per tenant/table; bound total snapshot age including DS waits; expose oldest snapshot,
  pool in-use/waiters/wait time by low-cardinality work class, and autovacuum/XID-age impact. Stream
  membership query-back candidates through bounded evaluation/emission rather than collecting a hot
  value or full table. When a pending-delta reservation cannot be made before cloning, abort that
  unacknowledged create; do not stall while already over budget.
- **Acceptance:** at the target reconnect wave with DS delayed and continuous membership flips,
  critical pool classes retain their reserved progress, maximum snapshot age and pending-delta bytes
  stay within limits plus one declared transaction/chunk allowance, vacuum advances, and accepted
  creates converge. Cancellation at permit wait, BEGIN, cursor row, DS append, activation, and share
  wait returns the permit and rolls back the snapshot/state.

The acceptance must state whether create overload is queued or rejected and give a maximum queue
wait. "Backpressure, spill, or reject" is not one testable product behavior.

### 6. P1 — `ENG-007` omits other unbounded lossless and request queues

The known flip and emission channels are not the complete queue inventory. The sequencer command
channel is unbounded (`engine/sequencer.rs:134`), the catalog writer channel is unbounded and can
stall forever on one DS event (`engine/catalog.rs:308-316`), and the retirement channel plus its
internal `VecDeque` are unbounded (`engine/retirement.rs:43-95`). PG semaphore waiters, inbound HTTP
requests, per-handle distinct-offset long-poll coalescers, and DS request concurrency also need
limits. Bounding only worker count leaves queued task/request bodies resident.

Expand `ENG-007`:

- Inventory every channel, semaphore waiter set, spawned task set, retry queue, HTTP accept/request
  queue, and per-handle coalescer. Record producer, consumer, ownership bytes, durability, overload
  action, and shutdown behavior.
- Use reservations obtained before state mutation for lossless catalog work. A full catalog queue
  cannot drop a post-mutation event; either backpressure before mutation or stage it durably. Deduplicate
  retirement work by stream/intent because `Dropped` already supplies recovery. Separate critical
  sequencer lifecycle commands from rejectable diagnostics so a debug flood cannot delay activation
  or removal.
- Bound requests by both items and owned bytes. A channel of 100 one-byte messages and 100 16 MiB
  messages is not the same bound.

Acceptance: for each inventoried queue, a generated test reaches `limit-1`, `limit`, and `limit+1`,
asserts current/peak/oldest/wait/reject metrics and the exact overload result, and proves that a DS
outage plus maximum admitted producers stays inside the process memory envelope. Client-visible
mutations still meet durable-before-ack semantics; shutdown either drains or leaves a durable replay
point.

### 7. P1 — The DS/HTTP page, body, timeout, and connection plane has no implementable owner

`PROTO-003` specifies a maximum page size, but the engine's DS client currently calls `res.text()`
and deserializes the complete response; it neither requests nor enforces a page byte limit
(`apps/engine/src/ds.rs:635-660`). Appends serialize a complete vector before sending
(`ds.rs:446-466`). `reqwest::Client::new()` supplies no project-level per-operation timeout,
connection cap, or cancellation policy. Gateway quotas do not protect DS when `SEC-004` chooses
direct signed capabilities, and `ENG-006` addresses only Postgres replication setup.

Add `ENG-006A — Bound internal and public HTTP transport resources`.

- **Depends on:** `SEC-004` decision, `PROTO-003A`, `DS-001`.
- **Deliverables:** DS read pagination contract enforced server and client side; Content-Length and
  incrementally decoded byte/envelope/depth limits; distinct connect/header/body/idle deadlines for
  append, HEAD, finite read, and long poll; bounded connection pools and in-flight requests per work
  class; cancellation joined to shutdown where correctness permits. Bound gateway inbound body,
  concurrency, accept backlog, response buffers, and long polls. If capabilities bypass the gateway,
  enforce equivalent identity/quota/verb constraints at DS.
- **Acceptance:** oversized/chunked/no-Content-Length and slowloris bodies, stalled headers/body,
  distinct-offset poll floods, reconnect/TIME_WAIT storms, connection refusal, and cancellation at
  every await stay within memory/FD/connection limits and yield stable retry/reset errors. A hung DS
  request cannot pin shutdown past the declared drain watchdog, and no timeout advances a
  checkpoint past unlanded output.

Clarify `ENG-006`'s "first receive" as first replication protocol frame/keepalive, not first row
change; an idle healthy database must not reconnect merely because there is no DML.

### 8. P1 — The existing harnesses cannot produce CAP-002--004 evidence

The gap is larger than adding a manifest:

- The fleet runner aggregates StatsD UDP samples in memory, drops tags, writes only Markdown
  percentiles, has no p999, and discards raw values (`packages/bench/src/electric-bench-runner.ts:
  69-110,327-378`). UDP loss is not detected. The Elixir child's exit status is ignored, and the
  overall process exits zero even when an individual row is `ERROR`.
- External-target mode does not provision identical isolated resources or collect target process/
  disk/IO metrics. It can compare endpoints, but not establish parity by itself.
- Loadgen terminates by `DURATION_S`, is closed-loop/think-time driven, and does not schedule a
  declared offered writes/s. As the system slows, offered work falls: classic coordinated omission.
  Its sampler records engine RSS/CPU plus directory sizes, but not DS/PG/gateway/client CPU/RSS/IO/
  FDs, end-to-end commit-to-client latency, queue waits, raw errors/retries, oracle divergence, or
  operation-count completion (`packages/loadgen/src/config.ts:11-68`, `run.ts:122-156`,
  `metrics.ts:18-35`).

Add `CAP-000 — Build the qualification driver and evidence schema` before any evidence run.

- **Depends on:** `GOV-002`, `CAP-001A`; no dependency on `CAP-001B` or CAP-002--004.
- **Deliverables:** deterministic operation-count traces; an open-loop arrival scheduler that records
  scheduled, offered, admitted, committed, applied, rejected, and dropped times/counts; separate
  closed-loop user realism runs; canonical SQL-oracle sampling/full final comparison; raw per-op
  latency/error data; process/cgroup CPU, RSS, IO, FD/socket/TIME_WAIT, filesystem, PG WAL/vacuum/
  connections, DS WAL/fsync/queue, gateway, and client-node metrics; fault trigger/event log; pinned
  topology manifest; and resumable artifact upload. Fleet runs must fail on a nonzero child, missing
  expected samples, UDP/sample loss, `ERROR` row, or incomplete workload. Preserve tags needed to
  distinguish operation/workload while bounding label cardinality.
- **Acceptance:** deliberately kill a driver child, drop telemetry, corrupt a raw artifact, under-run
  the operation count, and inject an oracle mismatch; each makes the workflow nonzero. A deterministic
  trace replay produces the same operation identities and fault points. A synthetic delayed server
  shows scheduled latency growing even when closed-loop latency would hide the queue.

Make `CAP-001B`, `CAP-002`, `CAP-003`, and `CAP-004` depend on `CAP-000`; the evidence runs also
depend on `CAP-001B`. The current tools remain useful development probes, but must not be cited as
qualification evidence until this task closes.

### 9. P1 — The numeric/statistical acceptance language is not yet falsifiable

Three independent repetitions can report run-to-run spread, but cannot support a meaningful
run-level confidence claim without a declared estimator. p999 is just the maximum when a workload
has only hundreds of observations. Ten million mutations and 100 failure injections are fixed and
reproducible, but their numbers are not tied to a detection probability, and correlated operations
must not be presented as independent reliability trials. "No unbounded positive slope" and
"material throughput collapse" have no units, estimator, confidence level, or pass threshold.

Edit the completion contract and CAP tasks:

- Pre-register for each metric the population unit, warm-up cutoff in operations, observation count,
  estimator, confidence method, and pass threshold. Preserve individual samples and run boundaries.
- Publish p999 only when there are at least 100,000 valid observations for that operation class
  (at least 100 observations in the nominal tail); otherwise mark it `insufficient sample`, never
  substitute max. Report exact quantile rank intervals or another predeclared non-parametric method.
- Treat three runs as a minimum reproducibility smoke, not a confidence guarantee. Derive additional
  run/operation counts from the desired interval width or failure-probability bound. If zero failures
  occur in 100 independent trials, report the approximate 95% upper bound (~3%), not "proved zero";
  deterministic cut-point runs are coverage evidence, not a failure-rate estimate.
- Replace the slope criterion with named resource models. For non-retained owned bytes use a robust
  per-million-operation slope and upper confidence bound after operation-based warm-up; for FDs and
  queue depth use exact high-water/return-to-baseline bounds; for lag use bounded drain operations;
  for disk separate expected retained bytes from unexplained residual and reconcile logical to
  physical accounting. RSS is a sizing high-water and allocator signal, not by itself a leak oracle.
- Define instrumentation overhead numerically in `capacity-target.yaml` (for example maximum allowed
  CPU and p99 delta versus the same trace with detailed telemetry disabled). CAP-001's current
  "material throughput collapse" must reference those fields. Drive 80% using tiny test limits, not
  80% of production disk/RSS.

This preserves the no-calendar-monitoring rule while making each result reproducible and honest.

### 10. P1 — CAP-003 tests a target but does not discover a supported capacity or define overload

`capacity-target.yaml` records demand, while CAP-003 is supposed to publish sustainable capacity.
Running only 70% and 2x of planned peak does not locate the saturation knee or establish safety
margin. The 2x burst also has ambiguous semantics: quota rejection, queued backlog, and committed
load are currently conflated. "Production admission defaults do not exceed measured capacity" is
too weak if the measured point has no confidence lower bound or reserved failure headroom.

Edit `GOV-002` so the manifest also contains:

- topology and per-component CPU/RAM/volume/IOPS/FD/connection limits;
- offered and accepted writes/s, transaction-size and row-width distributions plus maxima, operation
  mix/hot-key skew, query-template/selectivity/sharing mix, tenants and per-tenant skew, churn, lease/
  renewal rate, active/dormant/slow/offline consumers, retention and compaction windows;
- queue and rejection budgets, per-resource headroom, steady/burst operation counts, catch-up bound,
  and distinct RPO, refetch, readiness, and failover RTO definitions; and
- an explicit unsupported value for any unqualified dimension.

Edit `CAP-003`:

- use fixed-operation step loads to bracket saturation, then repeat points below/at/above the knee;
  sustainable capacity is the highest **accepted** rate whose latency, lag, queue, error, and resource
  confidence bounds all pass;
- run the 10,000,000-mutation qualification at the selected operating point, not automatically 70%
  of an unverified demand number;
- define burst offered rate, admitted rate, allowed stable quota errors, backlog, and drain operations;
  quota shedding is success only when it matches the declared overload contract; and
- set production defaults no higher than the conservative measured lower bound after reserving the
  manifest's failure/headroom margin.

Acceptance: a deliberately undersized deployment fails qualification and identifies the limiting
component; the capacity-table generator refuses a row with incomplete dimensions, insufficient
samples, unaccounted rejections, or a production default above the qualified bound.

### 11. P1 — Snapshot/materialization acceptance promises rejection before an allocation that has already happened

`ENG-009` says to reject before allocation, but data-dependent row width is known only after
Postgres has produced and the driver has allocated the JSON value (`pg.rs:999-1014`). The chunker
also deliberately accepts one row larger than its chunk budget (`pg.rs:920-928`). `/query` uses
`client.query` and materializes all rows before converting them (`pg.rs:1296-1303`), subquery seeds
and query-backs call `collect`, and `/v1/shape` folds a whole stream then clones rows into messages
and a handle key set (`electric.rs:516-537,908-925`). A static quota can be rejected before BEGIN;
a dynamic result-byte quota cannot truthfully be rejected before observing at least one row/chunk.

Edit `ENG-009`:

- distinguish **static admission** (template, projection, declared page limit, predicate/depth,
  concurrent-memory reservation) from **dynamic enforcement** (encoded rows/bytes observed while
  streaming);
- enforce a maximum encoded single-row/envelope size at the SQL/protocol boundary and count the
  one-row decoder allowance explicitly; use `LIMIT max_rows + 1` or cursor streaming for row caps,
  incremental byte checks, and immediate rollback/cleanup on crossing;
- remove or cap arbitrary OFFSET scans from the production template contract; use opaque keyset
  cursors for supported pagination;
- stream inner seeds into reserved state and query-back candidates into bounded processing rather
  than collecting; and isolate `/v1` snapshot concurrency because Electric requires one complete
  snapshot body.

Acceptance: test single-row, cumulative-row, encoded-response, and protocol-depth limits separately
at `limit-1/limit/limit+1`; peak owned bytes/RSS may exceed the configured request buffer only by the
documented decoder/framing allowance. A rejected dynamic result leaves no handle/claim/stream/catalog
record and its repeatable-read transaction is no longer visible in `pg_stat_activity`.

### 12. P1 — Time-triggered correctness still needs operation-based execution, not real calendar waits

The spec correctly removes 72-hour/24-hour calendar soaks, but leases, handle TTL, dormant TTL,
segment age, long-poll deadlines, retry backoff, and backup retention are still time-driven. A fixed
mutation count does not exercise them unless time advances, and waiting seven real days would
reintroduce the forbidden calendar gate.

Add a cross-cutting deliverable to `CAP-000`, `TST-003`, and `CAP-004`: inject clocks into policy
logic and use accelerated, explicitly configured short real-stack intervals for the small number of
socket/Postgres behaviors that cannot use a virtual clock. Artifact manifests must record logical
time advances and real operation/fault boundaries. Acceptance: every time threshold is crossed at
`t-1`, `t`, and `t+1`; repeating the same seed and logical clock schedule reaches the same result;
no qualification task says "monitor/soak for N hours/days."

### 13. P2 — Connection capacity needs component budgets and a gateway-mode decision

"Connection scale" in CAP-003 is not enough to configure a deployable system. A subscription may be
a gateway inbound poll plus gateway outbound DS connection, or a direct capability read; the two
topologies have different FD, ephemeral-port, TLS-handshake, revocation, and load-balancer costs.
The current loadgen's `USERS x FEEDS_PER_USER` estimate covers only a client-node rule of thumb.

Edit `SEC-004` acceptance to freeze one first-release topology before `ENG-006A` and CAP connection
work starts. Edit CAP-003's connection row to measure, per component: listening/accepted/established/
TIME_WAIT sockets, FDs, TLS handshakes/resumptions, accept backlog, gateway upstream pool, DS active
long polls, idle timeout churn, slow readers, reconnect admission/jitter, and client-node ephemeral
ports/RSS. Acceptance: target idle, fan-out, slow-reader, and reconnect-wave traces stay under every
manifest limit with the declared headroom; a limit crossing rejects before opening the next upstream
socket and does not cause another tenant's accepted polls to miss its error budget.

### 14. P2 — The capacity matrix needs the remaining single-thread and database bottlenecks as named axes

The draft mentions circuit/routing/fallback and aggregate/subquery cardinality, but CAP-003 should
name the bottlenecks whose interaction determines the knee: the one global sequencer, synchronous
membership-circuit step under the registry lock, one counts-circuit thread, O(K) fallback lists,
per-stream output serialization, DS group commit/fsync, PG query-back selectivity, repeatable-read
snapshot age/vacuum, logical-decoding/WAL generation, and catalog/checkpoint rate. Otherwise a
LinearLite-shaped mix can qualify while an allowed template mix saturates a single serialized stage.

Edit CAP-003 deliverables to require one isolation trace for each serialized stage and a mixed trace
at the maximum supported proportions from `capacity-target.yaml`. Acceptance: publish service time,
utilization, queue wait, throughput knee, and limiting component for each; every supported template
maps to circuit/routing/fallback and a measured selectivity/fan-out class. A template outside a
qualified class is marked unsupported rather than extrapolated.

## Gate and sequencing corrections

After incorporating the findings, update the launch gates and waves as follows:

- G4 must include `ENG-006`, `ENG-006A`, `ENG-007`, `ENG-007A`, `ENG-008`, `ENG-009`, `ENG-009A`,
  `ENG-010`, `ENG-011`, `ENG-012`, `DS-001`, `STO-001`, and `STO-002`. Its current closure omits
  production-config validation and the actual connection/body and continuous-state bounds.
- G9 must require `CAP-001A`, then `CAP-000`/bounded implementation, then `CAP-001B`, before
  `CAP-002`--`CAP-004`.
- `CAP-003` must list the boundedness task IDs explicitly rather than "engine boundedness tasks."
- `CAP-004` must depend on `OPS-002B` as well as `OPS-004B`; disk-full/restore evidence is invalid
  before the storage recovery implementation exists.
- Put `PROTO-003A`, `SEC-007A`, `OPS-002A`, and `OPS-004A` in the contract/foundation wave;
  `CAP-001A` in the contract/foundation wave; `CAP-000` immediately after that contract;
  `DS-001`, `ENG-006A`, `ENG-007A`, `STO-001`, `STO-002`, and `ENG-009A` in boundedness waves;
  `CAP-001B` after those implementations; and evidence runs only after every resource limit and
  overload action is implemented.

## What is already sound

- Replacing elapsed-day soaks with fixed operation counts, deterministic failures, retained seeds,
  and resource-drain assertions is the right direction.
- The target manifest as a single source of workload dimensions is correct; it needs the additional
  distributions, topology, quota, and headroom fields above.
- The proposed matrix correctly separates distinct/shared shapes, native/Electric snapshots,
  circuit/routing/fallback, subquery selectivity, aggregate groups, connections, large transactions,
  disk retention, and recovery. Those axes should be retained.
- XID-based `SnapshotGate`, append-before-ack, checkpoint/highwater, and transaction-end input markers
  give the boundedness work a credible correctness foundation. The changes above are about making
  resource use finite without weakening those invariants.
