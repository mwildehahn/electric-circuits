# Verification and rollout strategy: ElectricSQL → Electric Circuits for the Swift client

Status: proposed release plan

As-of: 2026-08-22

## Decision framing

The production client is [`../electric-sync-swift`](../../electric-sync-swift), not the TypeScript
Circuits-native client. It implements the Electric Shape protocol, persists opaque
`handle`/`offset`/`cursor` state through its app-provided `MetadataProvider`, applies messages to an
app-owned cache, and supports both SSE and HTTP long polling. Therefore the migration target is the
fork's Electric compatibility adapter, `GET /v1/shape`, rather than the fork's native tRPC API.

This is a protocol migration, not a stream handoff. Do **not** carry an upstream Electric handle,
offset, cursor, cached shape ownership, or server stream URL to Circuits. The values are opaque and
not portable. In particular, Circuits keeps Electric adapter handles in process memory; an engine
restart intentionally returns `409 must-refetch` for a prior handle. Start the Circuits generation
with `offset=-1` and a fresh client-side protocol/server generation namespace.

The same Postgres source can be shared, but each service needs its own logical-replication slot and
associated WAL budget. Never share a replication slot between Electric and Circuits, and never point
the candidate at a production database until replica identity, publications, privilege boundaries,
slot retention, and schema-drift handling have been checked.

Two compatibility facts need deliberate treatment:

1. `/v1/shape` is an HTTP long-poll adapter (`live=true`), not an SSE endpoint. Configure the Swift
   rollout path to use its `HTTPClientProvider` long-poll transport directly. Do not make repeated
   failed SSE upgrades the production transport-selection mechanism.
2. Circuits emits correct absolute insert/update/delete membership but deliberately does not emit
   Electric's row `tags` mechanism. The bundled Electric subquery suite currently has 13/15 passing
   tests; the two failures assert tag fields, not final membership. `MoveOutTagTracker` and the
   on-demand/progressive ownership paths in the Swift client make this a blocking Swift
   characterization item, not an acceptable undocumented exception.

## Existing evidence and reusable assets

| Asset | What it already proves | Reuse in this plan |
|---|---|---|
| `packages/conformance` (`harness.ts`, `conformance*.test.ts`) | Real Postgres → logical replication → engine → durable streams → Circuits client is equivalent to a Postgres oracle. It covers backfill/live fencing, NULLs, generated predicates, concurrent writers, subqueries, shape sharing, rotations, restart, shutdown, large transactions, catalog durability, storage loss, retention, epoch and drift. | Keep as the server correctness gate; parameterize its schemas and predicates from production shape inventory. Its `bootHarness` is the starting point for a Swift-facing integration fixture. |
| `electric-conformance/run.sh` | Electric's official Elixir client and `OracleHarness`/`ShapeChecker` drive Circuits `/v1/shape`; generated standard-schema property tests and hand-written subquery tests exercise the public wire surface. | Run `oracle`, `property`, and `subqueries` against every release candidate. Record the known two tag-only failures separately; do not let the result be reported as a blanket pass. |
| `apps/engine/src/electric.rs` unit tests and adapter implementation | Snapshot (`offset=-1`), headers, schema text encoding, positioned reads, 204 long-poll deadlines, 409 `must-refetch`, old-offset replay, and coalesced concurrent polls are defined and tested. `replica` is accepted and Circuits always sends full rows. | Use its exact response shapes to seed the Swift wire corpus; add black-box tests rather than duplicating private Rust behavior. |
| `packages/conformance/src/conformance-{restart,shutdown,large-txn,catalog-durability,native-storage-loss,epoch,schema-drift,changes-rotation,retention}.test.ts` | Crash/restart and durability properties at the server boundary, including raw-envelope duplicate checks during shutdown. | Run in every candidate CI; extend only where a mobile-visible outcome is absent. |
| `../electric/integration-tests/tests/*.lux` | Upstream operational scenarios: crash recovery, rolling deploy, Postgres disconnection, slot invalidation/self-conflict, large transactions, connection scarcity, TLS and IPv6 fallback. | Treat as scenario specifications. Port the relevant ones to a Circuits deployment test rather than assuming upstream behavior transfers. |
| `../electric-sync-swift/Tests/ElectricSyncTests` | Strong unit coverage for parser, request replica mode, backoff/circuit breaker, protocol quarantine, move-out semantics/tag tracker, batch buffering, stream-manager cancellation, replica-owner lifecycle, local GRDB behavior, and many `ElectricSyncClient` resume/ownership races. | Preserve all tests; replay their provider-level scripts using real Circuits fixture traces and add server-backed tests alongside them. |
| `packages/loadgen` and `packages/bench/electric-bench-runner.ts` | Loadgen drives native Circuits client and samples RSS/CPU/disk/append latency. The fleet runner drives Electric-compatible servers under common workloads. | Reuse for server capacity baselines only. Neither exercises the Swift client or a real `/v1/shape` mobile polling pattern, so neither is a mobile release gate alone. |
| `/metrics`, `/metrics/prometheus`, `/memory`, `/ready`, `/health`, `/v1/health` | Existing counters include envelope/appends, retention, epoch/drift, catalog/retirement retries, transaction spill/chunking, sequencer holds, replication lag/WAL retention, subscriptions and shutdown. | Use as the server half of release dashboards and automated abort criteria. `/ready`, not `/health`, gates traffic. |

The mandatory repository suites remain the baseline gate:

```bash
pnpm typecheck
pnpm engine:test
ELECTRIC_CIRCUITS_ENGINE_PREBUILT=1 pnpm test
ASDF_ELIXIR_VERSION=1.18.4-otp-28 ASDF_ERLANG_VERSION=28.1 \
  ./electric-conformance/run.sh oracle
ASDF_ELIXIR_VERSION=1.18.4-otp-28 ASDF_ERLANG_VERSION=28.1 \
  ./electric-conformance/run.sh property
ASDF_ELIXIR_VERSION=1.18.4-otp-28 ASDF_ERLANG_VERSION=28.1 \
  ./electric-conformance/run.sh subqueries
cd ../electric-sync-swift && ./Scripts/check-dependency-boundaries.sh && swift test
```

## Test architecture to add before a production cohort

These are missing today; they are concrete deliverables, not aspirational test categories.

| Missing fixture or harness | Minimum design | Why it is required |
|---|---|---|
| `SwiftCircuitsIntegrationHarness` | A test-only launcher, based on `packages/conformance/src/harness.ts`, that starts isolated Postgres, durable-streams, engine and `/v1/shape`; exposes base URL, SQL mutation helper, engine process control, metrics, and an explicit convergence barrier to Swift tests. It may be a JSON-line helper process, but Swift tests must interact only through HTTP and Postgres—not Rust internals. | The Swift package currently uses scripted provider tests; it has no end-to-end Circuits adapter test. |
| `CircuitsWireCorpus` | Versioned fixtures containing request query parameters, status, headers and JSON body for snapshot, empty page, changes, errors and controls. Generate each expected fixture from a real Circuits process and retain a corresponding official-Electric fixture where semantics should agree. Normalize opaque handles/offsets only by mapping them to symbolic tokens within one trace. | Prevents accidental changes to headers, 204 behavior, textual PG values, or control-message handling. |
| `SwiftShapeMaterializer` test adapter | A production-equivalent `HTTPClientProvider` using `URLSession`, plus a recording `MetadataProvider` and transactional GRDB test store. It must expose rows, persisted metadata and applied-message audit records. | Scripted messages cannot find URL encoding, HTTP header, cancellation, decoder, or durable metadata errors. |
| `ShapeTwin` differential driver | Executes the same seeded DDL, writes and shape definitions against upstream Electric and Circuits (separate slots), while Swift and the official Electric client materialize independently. It compares canonical PK → decoded row maps to Postgres at quiescent barriers and records every divergence with seed/trace. | “Both returned 200” is not cross-client correctness. |
| Fault proxy and clock | Toxiproxy (or an equivalent deterministic HTTP/TCP proxy) between Swift↔adapter, engine↔Postgres, and engine↔durable-streams; controllable app clock/background-task provider; failpoints only in test deployment. | Current conformance faults are valuable but do not model an iPhone losing a request mid-response or being suspended. |
| iOS host app and test plan | A small GRDB-backed iOS test host with stable accessibility IDs, launch arguments for backend/generation/fault profile, real `URLSession`, and XCUITest plus real-device jobs. | SwiftPM tests do not establish iOS lifecycle, background time, URLSession behavior, or UI/cache atomicity. |
| Verification coordinator | Server-side sampled shadow evaluator that hashes shape identity, waits for a declared Postgres/WAL or quiescence fence, runs the authorized SQL oracle, and accepts only normalized aggregate/row fingerprints from the app. | Live dual reads otherwise compare different moments and create false “mismatches.” |

No production identifier, predicate literal, row value, raw handle, offset, auth token, or stream URL
may be emitted in telemetry or stored in the corpus. Use keyed hashes and synthetic fixtures.

## Contract and Swift verification matrix

### 1. Wire contract tests

Run every fixture through the production `HTTPClientProvider`/decoder and through the existing
scripted provider. Cover at least:

- Snapshot request with qualified and public-schema table names, columns, `where`, `params`,
  `offset=-1`, `replica=default` and `replica=full`; snapshot response `200`,
  `electric-handle`, `electric-offset`, `electric-schema`, `electric-up-to-date`, inserts, then
  `up-to-date` control.
- Continuation with the opaque handle/offset/cursor passed through byte-for-byte; change pages with
  insert, update and delete; empty non-terminal page; body-less `204` with
  `electric-up-to-date`; request cancellation and retry of an older offset.
- `409 must-refetch`, expired/unknown handle, table/handle mismatch, `400` validation errors,
  auth failures, `500` retryable faults, and `503` draining/degraded response with `Retry-After`.
  A retry must never advance durable client metadata before the successful response is committed.
- Nulls, booleans and text as Electric textual values; finite floats; signed values; integer
  boundaries; primary-key escaping/composite-key behavior supported by the app model; projection
  always retaining required PK fields. Test the production schema rather than assuming the
  TypeScript client's bigint representation applies to the Electric adapter.
- Multi-page snapshots, live updates while snapshotting, updates that cross predicate boundaries,
  delete of a row the cache lacks, duplicate delivery, reordered retry pages, and a response split
  across arbitrary byte/SSE-parser boundaries.
- Circuit-specific semantics: `replica` is accepted but full row values are sent; adapter handles
  do not survive engine restart; idle handle expiry requires resnapshot; `tags` are absent. A
  tag-dependent progressive/on-demand path must either demonstrate correct cache convergence from
  absolute deletes or be feature-gated out of the first release.

**Acceptance:** each corpus case produces the exact expected local map and metadata transition;
no handler treats `204` as an empty data transaction, a `409` as a fatal app error, or an opaque
offset as ordered/numeric. The corpus has a reviewed compatibility version and runs in CI against
the pinned engine image.

### 2. Swift unit, integration, concurrency, and fuzz tests

Keep the existing tests, especially `SSETransportTests`, `SubscribeBackoffTests`,
`ElectricProtocolQuarantineTests`, `MoveOutSemanticsTests`,
`ElectricCollectionStreamManagerTests`, `ElectricReplicaOwnerLifecycleTests`,
`BatchBufferingTests`, and `ElectricSyncClientTests`. Add the following:

- Swift Testing parameterized corpus tests for every supported model/shape family. Use
  condition/latch based waits, not sleeps. Run parallel cases only when their Postgres database,
  cache path and engine catalog are isolated.
- Server-backed integration cases against `SwiftCircuitsIntegrationHarness`: initial snapshot,
  reconnect, process restart → `must-refetch` → atomic resnapshot, predicate move-in/out,
  user-visible subquery membership, concurrent writer/backfill fence, transaction larger than a
  page, and long-poll deadline. Assert cache state against Postgres, not merely message count.
- Metadata transaction tests that kill/cancel between receiving a page and committing local rows
  plus cursor state. Restart the app/test store and prove either zero application or one complete
  application—never a cursor advanced past unapplied rows.
- Concurrency stress with randomized task start/cancel/retry, multiple collections sharing a
  model, cache transaction delays and callbacks while a refetch replaces a stream. Run strict
  Swift concurrency compilation and Thread Sanitizer/actor-data-race-check jobs. Audit every
  existing `@unchecked Sendable` boundary that participates in this path; a warning-free build is
  not evidence of correct runtime isolation.
- A seeded protocol-state-machine fuzzer: generate legal and hostile sequences of pages,
  controls, disconnects, duplicate/older offsets, `409`, `503`, suspension and local transaction
  failure. Minimize and save failing traces to `CircuitsWireCorpus`. The oracle is a simple
  serialized PK map plus explicit metadata-state model, then the real Postgres oracle in
  server-backed replays.

**Acceptance:** 10,000 deterministic state-machine traces per release candidate with no divergence,
and at least 100 repetitions of each cancellation/restart race under sanitizers with no data-race,
leak, double-apply, hanging task or non-deterministic metadata outcome. Seed, app version, engine
image and reduced fixture accompany any failure.

### 3. Cross-client differential tests

For a representative and privacy-safe production schema fixture, run the same mutation journal
through:

1. Circuits `/v1/shape` + the Swift production-equivalent provider/cache;
2. Circuits + Electric's official Elixir client/`ShapeChecker`;
3. upstream Electric + the same Swift provider/cache (where the shape is supported); and
4. Postgres `SELECT` as the ultimate expected set.

Compare normalized PK → row maps only at an explicit barrier: quiesce writes or fence on an
observed source transaction/WAL position and wait for each system to report up-to-date. Do not
compare numeric offsets between servers. Include production predicate templates, column
projections, schema-qualified tables, `NULL`, multi-row commits, subquery move-in/move-out,
concurrent shape creation, and the iOS client's progressive/on-demand modes.

**Acceptance:** zero unexplained row or metadata-state mismatches over the full corpus and 100
seeded random journals. Any difference is a release blocker unless it is a documented, user-safe
semantic choice with an app-side test and a product sign-off; missing `tags` is not pre-approved.

### 4. Server conformance, chaos, and recovery

Run the mandatory Circuits suites above first. Then use the existing conformance scenarios to
exercise the candidate deployment and add black-box tests for the gaps below.

| Failure | Reuse | Required mobile-visible assertion |
|---|---|---|
| Graceful and forced engine restart | `conformance-restart`, `conformance-shutdown`, upstream `crash-recovery.lux` | Parked long-poll unblocks; next Swift request handles 409/refetch; cache converges without duplicate observer events. |
| Postgres loss/reconnect and slot conflict | upstream `postgres-disconnection.lux`, `replication-slot-self-conflict.lux` | `/ready` removes candidate from traffic; app backs off without hot looping; recovery converges. |
| Slot loss/epoch break | `conformance-epoch`, upstream `invalidated-replication-slot.lux` | Alert fires; the selected reset/refuse policy is observed; clients receive a recoverable resync path, never silently stale data. |
| Durable-streams outage, partial writes, retirement | `conformance-catalog-durability`, `-retirement-completion`, `-native-storage-loss` | No acknowledged data is lost; creates/releases are not falsely acknowledged; recovery or resync is observable. |
| SIGKILL, network partition and half response | existing restart/shutdown tests cover only part | Proxy cuts upload/download at every request phase, drops responses after server commit, delays/reorders pages and verifies one cache application. |
| Large commits and rotations | `conformance-large-txn`, `-changes-rotation`, upstream large-transaction Lux scenario | A mobile poll never exposes a partial source transaction as a completed local state; eventual cache equals Postgres. |
| Schema drift/TRUNCATE/identity change | `conformance-schema-drift` | Client gets a resync-safe failure; rollout process detects the counter/alert before serving stale schema. |
| Rolling deploy | upstream `rolling-deploy.lux` is a specification, not proof for Circuits | Old and new pods have explicit single-slot ownership and readiness handoff. Validate what happens to all in-memory adapter handles; expect refetch if no shared adapter registry exists. |

For every scenario run a fault matrix of 100 iterations in pre-production with randomly selected
cut points. A raw stream audit must check duplicates as well as a folded map; a map alone masks
repeat application.

**Acceptance:** no loss, stale success, or duplicate local observer transaction; recovery reaches
Postgres equivalence within the agreed recovery SLO; `/ready` and all epoch/drift/WAL alerts behave
as designed. A failure that requires manual database repair blocks rollout.

### 5. Capacity, soak, and mobile lifecycle

First establish an upstream baseline using the same Postgres size, shape distribution, write rate,
network latency and device mix. Then run:

- `pnpm bench:fleet` against upstream and Circuits for Electric-compatible request pressure;
- `packages/loadgen` for engine CPU/RSS/disk/WAL and retention behavior; and
- a new Swift-polling load generator using the actual long-poll request pattern, connection count,
cache commit cost and reconnect cadence of the app. Native-client loadgen results must not stand in
for this third run.

Run a minimum 72-hour pre-production soak at forecast P95 concurrent subscriptions plus 30% capacity
headroom, with production-shaped long-poll deadlines and churn. Exercise high fan-out writes,
large transactions, shape TTL/refetch, engine rolling restart, durable-streams failover and a
Postgres reconnect during the soak. Profile real iPhones as well as simulators for battery, network
bytes, wakeups, memory, cache size, CPU and foreground responsiveness.

The iOS matrix includes cold start; app kill/relaunch; foreground/background/foreground; background
time expiring during poll and during cache commit; Low Power Mode; offline launch; Airplane Mode;
Wi-Fi↔cellular; captive portal; VPN/proxy; IPv6-only; slow/lossy links; TLS failure; server 503; and
authentication/session refresh. Treat reachability as a hint only: attempt the request and handle
its outcome, rather than pre-checking connectivity. Every lifecycle transition must cancel owned
tasks, persist only a transactionally valid resume point, and restart using structured
concurrency without blocking the main actor.

**Acceptance:** no correctness mismatch during soak; no unbounded task/connection/cache/RSS/WAL
growth after steady state; all resource and latency SLOs meet a pre-approved baseline budget with
30% headroom; no main-thread hang, crash, watchdog termination or unexpected background wake loop
on the supported device/OS matrix. Capacity that meets only a synthetic native-client workload does
not pass this gate.

## Correctness telemetry and operational gates

Instrument both sides before shadow traffic. Existing server metrics are necessary but insufficient
to identify a bad mobile generation.

Client telemetry (privacy-preserving) must include backend generation, app/client/engine image,
hashed shape-template and tenant cohort, transport, request outcome/status, retry delay, snapshot
duration/row count, up-to-date latency, `must-refetch`, reconnect, metadata commit failure,
cache-apply count, duplicate/no-op delete, local cancellation, background expiry, and verifier
result. Correlate request/verification IDs with server access logs using non-secret IDs; hash all
handles, offsets and predicates.

Server dashboards and alerts must include existing `/metrics/prometheus` counters plus per-adapter
status/latency/bytes, 409 rate, 4xx/5xx/503 rate, long-poll timeout and coalescing rate, active
handles, snapshot/refetch rate, shape creation failures, replica lag/WAL retained bytes, catalog and
retirement retries, epoch/drift events, sequencer held runs, and durable-streams availability. Set
alerts from the upstream baseline before enabling users.

Immediate abort signals are: any verified row-map mismatch; an epoch break, unresolved schema drift,
or unbounded replication lag; sustained candidate 5xx/409/refetch or retry rate above the agreed
baseline; loss/duplicate application; crash/ANR regression; readiness degradation; or source
Postgres WAL approaching its safe retention limit. Alerting must page a named on-call and include
the cohort, image, hashed shape and replay seed/verification record.

## Staged rollout and release gates

| Phase | Scope and action | Exit / acceptance criteria |
|---|---|---|
| 0. Inventory and contract freeze | Enumerate every Swift model, query template, projection, sync mode, auth path, local metadata key and app version. Freeze a Circuits engine image, adapter configuration, protocol corpus and rollback feature flag. Provision independent slot/publication and production-like pre-prod. | Inventory has 100% of observed production shapes; no unsupported predicate/table/mode is routed; security review approves adapter auth/TLS/proxy configuration; all baseline suites and Swift unit suite pass. |
| 1. Pre-production proof | Implement the missing harnesses, corpus and twin driver. Run server, Swift, differential, fault, lifecycle and 72-hour capacity gates against a restored/sanitized production-sized dataset. | All acceptance criteria in the test matrix pass. The tags/on-demand characterization has a signed result; unsupported behavior is disabled, not merely noted. |
| 2. Passive server shadow | Deploy Circuits with its own slot and no client traffic. Mirror sampled, authorized shape definitions into a shadow materializer and verify against fenced Postgres queries. Monitor ingest lag/WAL, error rates, drift and capacity. | At least seven days and a statistically representative sample of shapes/writes with zero verified divergence, no unsafe resource trend, and no source-DB impact beyond budget. |
| 3. Beta dual read | Internal then opt-in beta app cohort runs Circuits in a separate, disposable candidate cache/metadata namespace while the upstream-backed cache remains authoritative for UI and writes. A verifier compares state only at fences; never merge candidate rows into the production cache. | At least one full release cycle, all lifecycle/network fault scenarios, and zero verified divergence. Candidate-only retries/refetches remain within the agreed budget. |
| 4. Canary cutover | Route a deterministic 1% employee/test cohort, then 5%, 10%, 25%, and 50% by stable install hash. On first Circuits use, invalidate legacy Electric resumable state and take a clean candidate snapshot; retain legacy cache/metadata read-only through the rollback window. Writes continue to Postgres exactly once—there is no dual write. | Hold each step long enough for daily active, cold-start and background populations; require green dashboards, no abort signal, and explicit SRE/mobile/product approval before increasing. |
| 5. General availability | Default new installs and then all eligible existing users to Circuits. Keep the upstream service, old app path, corpus verifier, independent slot and rollback flag throughout the declared observation window. | 14 consecutive days at 100% with correctness, crash, latency, capacity and support metrics within budget; post-rollout review approves retirement planning. |
| 6. Decommission | Stop shadow reads only after the observation window. Retire upstream slots and old metadata only after a separately approved, reversible cleanup plan. | A restore rehearsal proves no user needs the obsolete stream state; source WAL/slot cleanup is documented and monitored. |

Feature eligibility must be server-controlled and reversible. It includes app build, protocol corpus
version, schema/template support, capability result for tag-sensitive mode, cohort and kill switch.
Do not use a percentage rollout as proof of correctness without independent sampled verification.

## Rollback runbook

Rollback is a routing change, not an attempt to translate streams:

1. Freeze cohort expansion; set the server-side flag to stop issuing Circuits snapshots/long polls.
2. Return affected clients to upstream Electric, cancel candidate tasks, and mark the Circuits
   metadata generation invalid. Do not send its handles or offsets upstream.
3. Keep the known-good upstream cache/metadata namespace intact for the planned rollback window. If
   it cannot safely be reused, force an upstream `offset=-1` snapshot before exposing cached data.
4. Preserve Circuits engine/durable-streams/Postgres logs, metrics, normalized verification record,
   client diagnostic IDs and the failing corpus seed. Do not delete the candidate slot until the
   incident is understood; watch retained WAL while it remains paused.
5. If data may have been shown incorrectly, disable affected cached views or force a blocking
   refresh according to the product incident policy; application writes remain in Postgres and must
   not be replayed merely because sync is rolled back.
6. Re-enter at Phase 1 or 2 after a root-cause fix, a minimized regression fixture, and a reviewed
   decision record. A rollback caused by a correctness mismatch requires a full pre-production
   differential rerun, not a percentage retry.

## Release checklist

The migration may ship only when all items are true:

- The pinned Swift client uses long polling for Circuits and has a tested `409 must-refetch` recovery.
- Legacy and Circuits stream state are segregated by an explicit generation; no opaque-state import
  path exists.
- All mandatory Circuits, Electric conformance and Swift suites pass, with the status of all 15
  Electric subquery tests and the Swift tag-sensitive characterization recorded.
- The Swift integration harness, wire corpus, twin driver, fault proxy, host app and verification
  coordinator exist and have produced retained evidence for the candidate image.
- Cross-client/Postgres differential, fault/restart, fuzz/concurrency, capacity/soak and real-device
  lifecycle gates meet their stated criteria.
- Slot/WAL budgets, backup/restore, readiness routing, alert thresholds, dashboards, on-call
  ownership and the tested rollback flag are approved by database, SRE and mobile owners.

Anything less is useful development evidence, but not sufficient evidence to replace the production
sync backend.
