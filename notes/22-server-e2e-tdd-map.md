# Server E2E / TDD acceptance-test map

Status: reviewed scenario input, integrated into notes 18/24. Scope: production server path, not
implementation detail; canonical task authority remains note 18.

## Decision

Use **PostgreSQL 18 as the primary production and acceptance-test target**. The engine already
contains explicit PG18 handling for `pg_publication.pubgencols`; the tests, Docker Compose files,
and several documents still name PostgreSQL 16. Do not call PG18 support complete merely because
the generic logical-replication suite happens to pass on it. Make a PG18 job the required release
gate, and add the generated-column/publication scenario below. A PG16 lane can remain a
compatibility lane while it is supported, but it must not be the only green signal.

The acceptance contract is intentionally small and refactor-safe:

> Given committed application writes to Postgres, authorized clients eventually observe exactly
> the rows (and aggregate values) selected by the documented query template at a declared source
> transaction boundary; failures either preserve that contract or make the required recovery
> visible and force a safe resync.

Tests may read Postgres for the oracle and use public HTTP/stream APIs plus deployment-process
control. They must not assert circuit nodes, Rust types, catalog event layout, in-memory counters,
or private function calls. Metrics and readiness are allowed only as operational observations or
barriers, never as the truth oracle.

## Test topology and tiers

Keep fast tests in `packages/conformance`; add a separate deployment acceptance package so a
developer's in-process test server cannot accidentally become the production proof.

| Tier | New/updated location | Runtime | Release purpose |
| --- | --- | --- | --- |
| Contract E2E | `packages/conformance/src/conformance-pg18-generated-columns.test.ts` and the focused files below | ephemeral PG18 binaries + real engine, DS, API, native client | Fast, deterministic Postgres-to-client correctness. Required per PR. |
| Production topology | `packages/acceptance/src/*.test.ts`, `packages/acceptance/src/stack.ts` | isolated client/management networks with file-backed DS, **PG18**, engine and authenticated gateway candidate images; API absent unless a private consumer is selected | Black-box image/configuration/restart/migration/security proof. Required for candidate qualification; smoke per image PR. |
| Electric wire compatibility | existing `electric-conformance`, plus `packages/acceptance/src/electric-gateway.test.ts` once gateway exists | pinned official Electric test harness/client against public gateway | The compatibility endpoint is a real public client contract, not an engine-unit-test detail. |
| Fixed-operation capacity | existing loadgen and fleet runner, plus fixed acceptance workload manifests | PG18 production-shaped stack | Boundedness and SLO evidence; nightly/pre-release, not a correctness substitute. |

`packages/acceptance` should deliberately depend on *published/built images and HTTP endpoints*,
not `createApiServer`, `bootHarness`, engine module imports, or test-only fault strings. Its stack
launcher should accept exact candidate digests and the selected public gateway profile. A separate
`direct-isolated` adapter may drive early PG18 failures but is never promotion evidence. The launcher must
write all logs/metrics/inspect output to a CI artifact directory, and tear down only resources it
created. Give every test an isolated Compose project, database, slot, DS data volume, and gateway
database/schema.

## Harness primitives to build once

Extend `packages/conformance/src/harness.ts` rather than adding sleeps to individual tests. Extract
the same public interface for `packages/acceptance/src/stack.ts`; implementations differ, the test
language does not.

| Primitive | Observable implementation | Why it is deterministic |
| --- | --- | --- |
| `pg.commit(sql, params)` / `pg.transaction(fn)` | Application-role SQL connection; returns the committed transaction's marker ID. | All test writes are committed source-of-record writes. |
| `causalFence.commitAndApply()` | Write harness sentinel as the last statement of the same source transaction; obtain `source.committed`, adapter-specific `server.drainedThrough`, then a target `client.appliedTailAfter` receipt keyed by principal/template/generation after a post-barrier read/cache commit. | It binds SQL prefix, direct/deferred server completion and the actual target materializer. `/replication/lsn`, changelog tails and pending-work gauges remain private adapter diagnostics. |
| `expect.materialized(shape, sqlOracle)` | At the barrier, query Postgres with the canonical template predicate and compare normalized PK → projected row map to a real native-client materialization or a gateway client materialization. | Map equality, not message count, is the data correctness oracle. Preserve a raw event audit for duplicate/transaction checks. |
| `stream.parkAtTail(feed)` / `stream.awaitTerminal()` | Read public feed to `up-to-date`, then issue its documented long poll. | Lets tests prove immediate close/refetch/retry behavior without racing a timer. |
| `process.stop(kind)` / `process.start()` / `process.waitReady()` | Compose service stop/kill/recreate or the existing child-process handle. `waitReady` polls `/ready` state transitions, not a fixed delay. | Makes restart and readiness handoff observable from outside the engine. |
| `fault.hold(name)` / `fault.release(name)` / `fault.hit(name)` | Deployment test proxy with named HTTP/TCP cut points and a hit latch. Conformance may retain `wrapEngineDs`, table locks, and backend termination as lower-tier equivalents. | A fault is released only after the relevant request has definitely reached it. |
| `gateway.request(principal, request)` / `gateway.audit()` | Test JWT issuer/authorizer plus recorder at the private gateway→engine/DS boundary. | Security tests can prove an unauthorized request made *zero* internal calls, rather than infer it from a 403. |
| `observe.resources()` | Scrape `/metrics/prometheus`, `/memory`, DS volume bytes, `pg_replication_slots`, container RSS/FDs/connection counts at named barriers. | Assertions use bounded deltas/plateaus over a fixed operation count, never elapsed-time guesses. |

Use bounded polling only to wait for an explicit state change, with the last observed state in a
failure message. Do not add `sleep(250)`, “eventually after N seconds”, or a bare `SELECT` that can
pass while the engine is stalled. Existing `drainEngine`, `waitFor`, Postgres table-lock waiters,
stream-tail readers, raw stream folds, and DS fault proxies are good seeds; move their mechanics
behind the primitives above.

## Existing coverage to preserve and reuse

Do not duplicate these tests just to give them a new filename. Promote their fixtures/barriers into
the common acceptance vocabulary and add a client-visible assertion only where noted.

| Existing evidence | Reuse as | Missing acceptance observation, if any |
| --- | --- | --- |
| `conformance*.test.ts`, `compare.ts`, `harness.ts` | Base Postgres → replication → DS → native client vs Postgres oracle, including predicate/fuzz/subquery/null coverage. | Run on PG18; add raw event transaction assertions in the new transaction suite. |
| `conformance-backfill`, `-backfill-streaming`, `-concurrency`, `-subset-positioning` | Snapshot fence, backfill chunking, concurrent write and position overlap cases. | A public Electric/gateway client must also show a safe snapshot/refetch path. |
| `conformance-restart`, `-shutdown`, `-changes-rotation`, `-large-txn` | Crash, graceful shutdown, log segmentation, spill/chunking and checkpoint behavior. | Deployment process/volume version of the recovery cases. |
| `conformance-retention`, `-subscriptions`, `-shape-sharing`, `-native-storage-loss`, `-retirement-completion`, `-catalog-durability` | Lifecycle, named-claim ambiguity, stream loss, close-before-delete and durable intent. | Gateway ownership/idempotency/revocation mapping is not implemented yet and needs its own suite. |
| `conformance-schema-drift`, `-epoch`, `-degraded-create`, `-boot-errors` | Fail closed for DDL, slot loss, lost membership effects and unsafe boot. | Public resync/error contract plus alert/readiness evidence. |
| `electric-conformance` oracle/property/subquery suites | Official Electric client and OracleHarness protocol oracle. | Pin upstream revision and report the two known `tags` cases as explicit profile-gated expected failures, never a blanket pass. |
| `packages/loadgen`, `packages/bench` | Workload generator, resource samples, fleet-compatible request pressure. | Fixed PG18 capacity acceptance profile and plateau assertions; they are not authorization or E2E correctness tests. |

## New acceptance suites

The names below are deliberately acceptance language. Each test should be a single
Given–When–Then scenario, with a Postgres SQL oracle at the final barrier.

### A. PG18 / data and protocol contract

`packages/conformance/src/conformance-pg18-generated-columns.test.ts`

1. **Given** a PG18 table with a stored generated column and a `FOR ALL TABLES` publication whose
   `pubgencols` setting publishes stored generated columns, **when** a base-column insert and update
   commit, **then** a freshly created and an already-live shape expose the generated value exactly as
   the PG18 `SELECT` oracle does; public event replay follows the selected profile and never loses an
   acknowledged applied effect.
2. **Given** that same shape survives an engine crash, **when** the generated-column publication
   setting changes or the generated expression/column changes while down, **then** the old feed
   reaches its documented terminal/resync state and a new feed backfills the new PG18 value; it
   never silently resumes with an old projection.
3. **Given** a publication that does not publish stored generated columns, **when** the engine
   starts against the generated-column table, **then** it either serves the explicitly supported
   projection exactly or refuses with a named configuration error before readiness. Decide the
   product policy first; the red test prevents an accidental half-supported state.
4. **Given** a PG18 `VIRTUAL` generated column, **when** its table/projection is admitted, **then**
   admission fails closed with the profile's typed unsupported-schema error before any snapshot or
   claim is acknowledged. The current snapshot-correct/live-`null` behavior is a release-blocking
   red case, not a supported projection.

`packages/conformance/src/conformance-client-transaction-visibility.test.ts`

1. **Given** a client parked at a feed tail and a transaction that changes two matching rows plus a
   row that leaves a predicate, **when** its one Postgres `COMMIT` is ingested across multiple
   change-log appends, **then** the server source-log checkpoint does not advance past a partial
   transaction and final public state folds to the SQL oracle. Core public resume/checkpoint remains
   event/response-level and may replay safely. Only `NATIVE_TXN_ATOMIC` additionally requires one
   eligible per-stream observer batch after a final marker, with no cross-stream claim.
2. **Given** a crash/SIGTERM after one chunk of that source transaction lands but before its final
   marker, **when** the engine restarts, **then** the selected profile safely replays or resets and
   final client state is complete. Focused invariant tests audit operation-ID'd source markers;
   public core does not promise exactly-once delivery.
3. **Given** a snapshot creation blocked after its repeatable-read snapshot is fixed and a concurrent
   committed writer, **when** the block releases, **then** the client map equals the oracle exactly
   once. Reuse `lockTable`/`tableLockWaiters`, not timing.

`packages/acceptance/src/electric-protocol-recovery.test.ts`

1. **Given** a public Electric-compatible snapshot and persisted opaque handle/offset, **when** a
   write moves a row in/out and a normal long poll follows, **then** the official Electric client
   reaches the PG oracle.
2. **Given** a process restart or handle-TTL expiry, **when** the client continues with its old
   handle, **then** it receives documented `409 must-refetch`, snapshots a new generation, and does
   not treat the old offset as portable.
3. **Given** a live request with no write, **when** the configured deadline expires, **then** it gets
   the documented `204`/headers and does not create a busy-loop or local empty transaction.

### B. Lifecycle, durable recovery, and faults

`packages/acceptance/src/lifecycle-recovery.test.ts`

1. **Given** two authorized clients sharing a template/feed, **when** one closes/retries its release
   and the other receives a Postgres write, **then** the remaining client continues and no second
   release is consumed. Reuse the named-subscription ambiguity fixture underneath.
2. **Given** the final claim is released, **when** idle → dormant → rejoin happens with writes during
   dormancy, **then** reactivation produces the exact current PG map; after eviction, the old public
   feed is terminal and a new creation has a new opaque public handle.
3. **Given** a tailing poll, **when** a forced purge, drift retirement, or epoch reset is completed,
   **then** it is released promptly with the documented terminal result; it must not hang until the
   ordinary long-poll deadline.
4. **Given** a durable-streams outage at create, release, and retirement separately, **when** the
   proxy releases storage after the request reaches the named cut point, **then** no success is
   returned before durable intent, a response-lost retry is idempotent, and recovery leaves one
   correct owner/stream state. Reuse catalog-durability's proxy cases as the red-test prototypes.

`packages/acceptance/src/restart-and-rollover.test.ts`

1. **Given** active feeds and file-backed DS, **when** the engine is SIGKILLed, writes commit while
   it is absent, and a new image process takes the slot, **then** `/ready` transitions unavailable →
   active, existing native feeds converge without re-registration, and Electric handles follow their
   explicit must-refetch contract.
2. **Given** a SIGTERM and a parked poll, **when** shutdown starts, **then** readiness turns 503
   before listener removal, the parked poll returns, exit is clean inside the configured grace, and
   a successor converges. Use a readiness probe recorder/latch rather than elapsed-time ordering.
3. **Given** forced small change-log segments and a dormant feed, **when** writes cross two rotations
   and a crash occurs, **then** resume reads every intervening committed effect once; only segments
   unpinned by durable progress may disappear. Reuse the rotation test's segment-status oracle.

### C. Migration, schema and epoch safety

`packages/acceptance/src/migration-safety.test.ts`

1. **Given** live feeds on `items` and `other`, **when** an additive migration plus a write reaches
   `items`, **then** only dependent `items` feeds terminally retire, `other` remains live, and a new
   `items` feed has the new schema/value.
2. **Given** a migration with no follow-up DML or a migration while the engine is down, **when** the
   reconciler/restore runs, **then** affected feeds are still retired before stale data can be served.
3. **Given** `TRUNCATE`, replica-identity regression, primary-key/type/drop-column change, and an
   intentionally blocked re-introspection, **when** each occurs, **then** the system exposes the
   documented resync/unresolved condition, rejects unsafe creates, and returns to active only after
   Postgres is settled. The unaffected-table shape remains a control.
4. **Given** the selected DBSP/circuit deployment profile, **when** a tracked-table migration or
   truncate requires a process restart, **then** the orchestration actually performs the declared
   drain/restart/resubscribe sequence; this is a production image test, not an internal circuit test.

`packages/acceptance/src/epoch-recovery.test.ts`

1. **Given** a healthy slot and active feeds, **when** the slot is dropped or its WAL status is made
   lost, **then** the default reset policy closes/retires every old feed, binds a new epoch, and only
   new snapshots serve data.
2. **Given** reset-on-loss is false, **when** the same break occurs, **then** readiness/data routes
   fail closed, an authenticated operator status surface identifies the broken epoch, and only an authenticated
   operator reset restores service. No data write during the unknown span is represented as safely
   delivered.
3. **Given** a replacement database/system identifier (restore/major-upgrade rehearsal), **when** the
   engine starts with the old catalog, **then** it follows the same selected policy. This is the
   migration-rehearsal test that protects a PG major upgrade, including PG16 → PG18.

### D. Gateway/security acceptance (new production surface)

These contract patches remain stacked red artifacts until their implementation turns them green;
they are never merged skipped/inverted/expected-failure. Current engine/API/DS endpoints
are privileged and unauthenticated; they are not safe stand-ins for these scenarios.

`packages/acceptance/src/gateway-authz.test.ts`

1. **Given** a valid principal and a server-owned template `tenant_id = principal.tenant`, **when**
   the client mutates table, predicate, projection, parameters, duplicate query keys, path encoding,
   or an opaque feed id, **then** the gateway rejects before any engine/DS request, or produces the
   same canonical internal request. The client never supplies an authority-bearing AST.
2. **Given** two tenants with overlapping primary keys, **when** each creates/reads/retries a feed,
   **then** each sees only its own SQL-oracle rows and cannot enumerate, join, renew, release, or
   infer the other tenant's feed by changing any opaque identifier.
3. **Given** malformed/expired/wrong issuer/audience/signature/session/revoked credentials, **when**
   a request arrives, **then** it has a non-enumerating denial, no registry mutation, and recorder
   counts of zero for engine, DS, and Postgres data queries.

`packages/acceptance/src/gateway-lifecycle.test.ts`

1. **Given** a public idempotency key, **when** a client retries after response loss at every
   gateway-registry ↔ engine-create/release boundary, **then** restart reconciliation yields exactly
   one owned internal claim or none according to the last acknowledged public outcome.
2. **Given** a policy/session revocation after a public long poll is held at the gateway, **when** the
   authorization barrier acknowledges, **then** the gateway cancels upstream and returns zero body
   bytes after the barrier, releases the exact internal claim once, and all later operations deny.
3. **Given** public network access, **when** a scan/request targets engine, API, DS, Postgres,
   metrics, catalog, graph/state/trace, or admin routes directly, **then** only the gateway is
   reachable. Gateway feed endpoints allow only documented GET/HEAD and allowlisted headers.

### E. Boundedness and operational safety

`packages/acceptance/src/boundedness.test.ts`

1. **Given** a fixed large transaction and a small configured transaction-memory budget, **when** it
   commits, **then** the final client map is complete, the server source checkpoint does not pass an
   incomplete transaction, `txn_spills_total`/chunk counters move,
   spill files are removed after recovery, and engine RSS stays below a declared budget derived from
   `TXN_MEMORY + append budget + fixed allowance`—not merely “lower than before.” The
   `NATIVE_TXN_ATOMIC` profile also asserts one observer batch.
2. **Given** a wide backfill with a small append budget, **when** a feed is created, **then** the
   result equals PG, backfill chunking is observed, and a per-create memory envelope holds. A
   timeout fails only that create and leaves a later live write observable.
3. **Given** a fixed mixed workload and maximum allowed subscriptions/retained streams, **when** the
   workload reaches steady state, **then** the accepted/rejected counts, connection/FDS, shape count,
   DS bytes, retained WAL, log-segment count, and queue gauges stay under their profile limits; after
   all clients close, they converge to the documented baseline. This needs a finite operation
   count and barrier-based samples, not an elapsed-time condition.
4. **Given** a 10,000,000-operation PG18 corpus at the approved capacity profile, with 100
   executions of each named fault cut, **when** controlled
   restart, DS outage, Postgres reconnect, shape churn, rotations, and a large commit are injected,
   **then** sampled resource slopes remain bounded and all final oracle checks pass. Save workload
   seed, image digests, config, metrics and logs as release evidence.

## Fault-cut matrix

The following are the minimum meaningful cut points. Each must be a named latch in the test proxy
or process controller, never a timing guess.

| Cut point | Inject | Required observable outcome |
| --- | --- | --- |
| After Postgres commit, before replication read | stop engine / sever replication connection | Restart processes the committed change once. |
| After externally held storage response for an oversized source transaction | SIGKILL/SIGTERM | Server source checkpoint stays safe; event-level replay/reset converges. `NATIVE_TXN_ATOMIC` also forbids a partial eligible observer batch. Internal marker cuts stay in the instrumented tier. |
| During snapshot after snapshot gate, before first/last backfill append | table lock + concurrent commit / kill | Backfill + live union equals PG once. |
| Engine → DS append returns false 404, transient 503, or response lost | DS proxy | No acknowledged write is dropped; false 404 reconciles; retry is idempotent. |
| Catalog create/join/left/dropped append accepted but response withheld | DS proxy | No premature success; retry/restart has exactly one lifecycle effect. |
| Shape stream removed externally / stream long poll held | DS control/proxy | Join does not hand out a dead feed; terminal close wakes the poll. |
| Query-back lock/retry exhaustion | PG table lock + terminate blocked backends | Engine degrades/fails closed; no live-looking stale subquery feed. |
| DDL/TRUNCATE/identity change during create and while down | PG DDL + process latch | Affected create/feed safely retries/resyncs; unrelated table stays live. |
| Slot drop, WAL loss, system-id replacement, slot busy | PG administration / replacement volume | Policy-selected reset or refuse path, never silent continuation. |
| Gateway registry transaction / engine call / upstream read / revocation | gateway recorder + hold proxy | Exactly-once public ownership and no post-revocation bytes. |

## Red-test-first implementation order

1. **Make the real acceptance baseline PG18.** Parameterize test Postgres binary selection
   (e.g. explicit `ELECTRIC_CIRCUITS_TEST_PG_BIN` or container image) and make the required CI lane
   use 18; update local/docker defaults and docs at the same time. First red test: PG18 stored
   generated columns + `pubgencols` behavior. Do not remove PG16 coverage until its support policy
   says so.
2. **Tighten the existing external oracle.** Add the PG18 generated-column and client transaction
   visibility tests to `packages/conformance`; extract barrier/raw-audit helpers from current tests.
   These are the highest-value refactor guardrails because they specify only source writes and
   observed state.
3. **Create `packages/acceptance` with one image smoke.** PG18 + file-backed DS + engine + gateway candidate images:
   create one native feed, write through Postgres, barrier, compare to SQL, then kill/restart and
   repeat. Publish logs/metrics on failure. This proves the deployable artifact rather than a Node
   harness topology.
4. **Add recovery/migration/epoch scenarios one failure family at a time.** Port existing conformance
   fixtures as black-box cases: restart/rotation, lifecycle/durability, schema migration, then slot
   loss. Keep existing focused tests; the new tier validates process orchestration, volumes, and
   externally documented responses.
5. **Build the public gateway before treating any client surface as production.** Start with the
   first failing tests in `gateway-authz.test.ts`: rejected AST/path/tenant substitution must result
   in zero internal requests. Then gateway feed proxy/lifecycle/revocation tests. Do not expose raw
   engine IDs, subscriptions, or DS URLs as a temporary production API.
6. **Add budgets and fixed-operation evidence last, after admission policy exists.** Turn the selected capacity
   target into fixed acceptance limits, feed the same templates through loadgen/fleet/gateway polling,
   and run the 10,000,000-operation PG18 corpus with artifacts. A passing load test without an oracle and a resource
   bound is performance observation, not acceptance.

## Release gates

Every result is `pass`, `fail`, `blocked`, or an explicitly approved profile-specific
`not-applicable`; “blocked” never promotes a release. Capture source SHA, dirty state, image digest,
PG18 image/binary version, DS mode, exact config, upstream Electric revision, workload seed, and
test artifacts with each result.

Required for an engine/gateway release candidate:

```sh
pnpm typecheck
pnpm engine:test
ELECTRIC_CIRCUITS_ENGINE_PREBUILT=1 pnpm test       # required PG18 lane
pnpm test:fuzz
./electric-conformance/run.sh oracle
./electric-conformance/run.sh property
./electric-conformance/run.sh subqueries             # tag-only exceptions explicit/profile-gated
pnpm --filter @electric-circuits/acceptance test     # exact PG18/server/gateway candidate digests
```

For production promotion, add the fixed capacity profile and the long fixed-operation PG18 corpus. The existing demo
browser run remains a useful visual smoke for engine/live-path changes, but it is not a replacement
for a black-box oracle or a gateway authorization test.
