# PostgreSQL 18 and black-box E2E/TDD addendum

Status: reviewed and integrated into the canonical execution specification, 2026-08-23.

This addendum answers two concrete questions:

1. PostgreSQL 18 should be the only first-production database profile.
2. Production work should be driven by high-level, real-stack E2E contracts whose oracle is
   PostgreSQL state at a named source commit, leaving engine, gateway, client, and cache internals
   free to change.

The detailed research inputs are
[`21-postgres18-support.md`](21-postgres18-support.md),
[`22-server-e2e-tdd-map.md`](22-server-e2e-tdd-map.md), and
[`23-swift-app-e2e-tdd-map.md`](23-swift-app-e2e-tdd-map.md). Its scenario rationale and proposed
contract content are reflected in the canonical tasks in
[`18-production-readiness-spec-reviewed.md`](18-production-readiness-spec-reviewed.md).

## 1. Decision: PostgreSQL 18, with explicit launch blockers

Use this exact first-production database contract:

> PostgreSQL 18.x on one writable primary; logical WAL; one dedicated `pgoutput` logical slot;
> one active engine for that slot; SCRAM credentials; verified TLS on query, snapshot/query-back,
> and replication connections; an explicit fully validated publication; stored generated columns
> only when the publication publishes them; virtual generated columns rejected. Promotion is an
> epoch break followed by feed reset and client rehydration, not seamless failover.

PG image evidence identifies the OCI index digest, declared OS/architecture platform tuple, and
resolved platform-manifest digest. The fixture verifies that exact resolution before startup; an OCI
index digest by itself is not evidence of the platform bytes exercised.

PostgreSQL 18 does not require a pgoutput protocol upgrade: protocol v1 remains supported. PG18
does, however, change or expose behavior that must be handled deliberately:

- PG18 supports publishing stored generated columns when configured, while virtual generated
  columns are not logically replicated. PG18 also made virtual generated columns the default.
  [Generated-column replication](https://www.postgresql.org/docs/18/logical-replication-gencols.html),
  [PG18 release notes](https://www.postgresql.org/docs/18/release-18.html).
- PG18 can invalidate an inactive logical slot for `idle_timeout`; the reason is exposed through
  `pg_replication_slots.invalidation_reason`. Any non-null invalidation reason means the old epoch
  cannot safely continue. [Replication settings](https://www.postgresql.org/docs/18/runtime-config-replication.html),
  [slot view](https://www.postgresql.org/docs/18/view-pg-replication-slots.html).
- PG18 failover slots require explicit primary/standby configuration and downstream fencing. They
  are a future profile. Merely running PG18 does not make promotion seamless.
  [Slot synchronization](https://www.postgresql.org/docs/18/logicaldecoding-explanation.html),
  [failover slots](https://www.postgresql.org/docs/18/logical-replication-failover.html).

### Unverified exploratory PG18.6 observation and blocker hypothesis

An exploratory local run reportedly used the official PostgreSQL 18 image
`postgres@sha256:06cad38a5d9f5d24b4d83d86def30795d5e4b757fedbf5281172b576dedcd941`,
reporting PostgreSQL 18.6 (Debian 18.6-1.pgdg13+2), with the real local engine and durable-streams
process. It is an unverified research observation, not inherited or qualification evidence, until a
replay bundle records the exact command, OCI index, platform tuple and resolved platform-manifest
digest, fixture SQL, source/engine/DS SHAs, and redacted raw snapshot/live payloads:

- ordinary snapshot/live insert, update, and delete passed;
- a stored generated column passed snapshot and live replication when the publication used
  `publish_generated_columns = stored`;
- a virtual generated column failed: snapshot/backfill returned the computed value, while a live
  insert emitted that field as `null` even though the PostgreSQL `SELECT` returned the computed
  value.

If reproduced, that is a silent snapshot/live divergence and a P0 blocker. The current schema paths
include the virtual field during introspection/backfill, but PG18 does not include its value on the
logical wire.
Changing only the fingerprint would not fix the bug: publication admission, table schema,
fingerprint, backfill projection, tuple decoding, and replica identity must all use the same
publishable-column set.

The differential reviews and this unverified observation establish the following blocker hypotheses;
they are tasks, not a reason to fall back to PostgreSQL 16:

1. use one canonical publishable-column/admission result on boot, create/join/reactivation, live and
   down-time drift, restore, backfill, filtering, tuple decode, fingerprint and replica identity;
2. capture any non-null slot `invalidation_reason`, including `idle_timeout`, and fail closed before
   old-epoch traffic; integrate reset only through the separately authorized reset workflow;
3. prove slot continuity from the durable landed source frontier and slot properties—same slot name,
   plugin and cluster do not prove the slot was not dropped/recreated ahead;
4. treat every promotion/timeline change as an epoch break even when the promoted primary has a
   synchronized, usable same-name slot;
5. make the publication bootstrap-owned and immutable while ready, fingerprint its full effective
   definition, reject tracked-table RLS, and set the walsender to fail on row-security filtering;
6. implement and independently test verified TLS/SCRAM identity on setup/admin, pool/backfill/
   query-back and walsender connectors, with no downgrade; and
7. qualify exact PG18 minor maintenance plus provider backup/PITR/restore and the resulting
   resume-versus-whole-generation-reset decision.

## 2. Refactor-safe acceptance boundary

The high-level contract is:

> Given committed writes to PostgreSQL 18, an authorized client reaches exactly the rows and values
> selected by its declared server-owned template at a named source-transaction fence. A recoverable
> interruption converges to that state without losing an acknowledged effect. An unrecoverable
> continuity break becomes a typed reset/refetch and never a live-looking stale continuation.

```text
named PG18 transaction
        |
        +----> SQL/journal oracle at SourceCommitID
        |
        v
logical slot -> engine -> durable streams -> authenticated gateway -> real client fold/cache -> app reader
                      \________________ externally controlled fault cuts _________________/
```

Tests may control real processes, network proxies, PostgreSQL transactions, and a test-only gateway
issuer. They may inspect public responses, normal app/cache reads, readiness transitions, redacted
gateway audit facts, and bounded resource metrics. They must not assert Rust type names, circuit-node
counts, catalog record layout, private actor state, or an exact internal call sequence.

### What “same source fence” means

Each mutating test transaction writes its data changes and one harness-allocated marker row as the
last statement of the same PostgreSQL commit. The marker relation is harness-only: it belongs to the
fixture's immutable explicit test publication, is excluded from public templates and client-visible
results, and is not a separate sentinel feed. `SourceCommitID` is not an Electric offset, Circuits
offset, shape handle, or LSN.

The marker is decoded only after the source transaction's terminal envelope. The causal fence has
three receipts: `source.committed(id)`; adapter-specific `server.drainedThrough(id)` only after
every causally preceding direct and deferred action has completed; then
`client.appliedTailAfter(id, principal, template, generation)` after a target read started following
the server receipt and the actual fold/cache transaction committed through its returned tail. Private
LSN/changelog/flip observations may implement the middle adapter receipt, but remain diagnostics and
never become the public/data oracle. A compatibility lane that cannot carry an in-lane receipt uses
quiesced writes plus an explicit per-template caught-up/cache-commit receipt.

The test either prevents later source writes until its SQL comparison finishes or folds the checked-in
operation journal only through `SourceCommitID`; querying current SQL after unrelated later commits is
not a valid oracle for an earlier fence. Deferred subquery/query-back work must also reach its named
barrier before the client state is compared.

### Transaction contract is profile-scoped

All profiles require that the server's source-log acknowledgement/checkpoint never advances past an
incomplete PostgreSQL transaction and that final fenced materialization loses no acknowledged applied
effect. Core public resume tokens/checkpoints are event/response-level and may safely replay
duplicates according to their client contract. Only `NATIVE_TXN_ATOMIC` with an eligible negotiated
stream requires one observer batch and a transaction checkpoint after its final marker; it never
claims cross-stream atomicity.

## 3. Harness deliverables

Build these once and reuse them across TypeScript server tests, Swift package tests, and the real app:

| Primitive | Concrete behavior | Failure diagnostics |
| --- | --- | --- |
| `Postgres18Fixture` | Starts an isolated PG18 cluster, asserts `server_version_num`, applies the exact publication/role/TLS profile including the immutable harness-only marker relation, commits named journals, and queries canonical SQL. | Server/config version, transaction journal, slot/publication facts. |
| `CircuitsStackFixture` | Starts built PG18, engine, file-backed DS and isolated storage immediately; accepts optional gateway/client adapters when those products exist; restarts/kills one component. The early direct-engine adapter is isolated test infrastructure, never a production exposure claim. | OCI index, platform tuple, resolved platform-manifest digest, config hashes, logs, exit status, volume inventory. |
| `CausalFence` | Produces the ordered source, server-drained, and target client/cache-application receipts above; rejects an unpublished marker, pre-transaction-end marker observation, deferred-work-skipping receipt, wrong principal/template/generation, or a pre-barrier tail. | Last `SourceCommitID`, receipt tuple/cache transaction, each adapter's redacted phase, pending named stage. |
| `GatewayProbe` | Issues real public requests with test credentials; holds/releases named request phases; records only redacted authority and downstream-call facts. | Principal/template/request IDs, status, allowed headers, internal-call count. |
| `ClientMaterializationProbe` | Folds through the actual TS/Swift client or reads the actual app generation through its normal reader. | Normalized key/value map, effect trace, generation, terminal/reset event. |
| `LifecycleProbe` | Parks a real tail and observes create/renew/release/replacement without exposing a production admin API. | Claim count/identity, tail state, terminal response, task completion. |
| `FaultGate` | Announces arrival at an enumerated network/storage/process/cache cut before the controller releases, cancels, or kills it. | Cut ID, hit count, before/after journal, process state. |
| `ResourceObserver` | Samples RSS, FDs, sockets/connections, queue gauges, DS bytes, WAL retention, segment count, and spill files at named operation barriers. | Raw samples keyed by operation count, declared cap, first crossing. |

Every wait has a diagnostic deadline, but ordering is created by events and gates, never
`sleep(250)`. Every test owns an isolated database, slot, Compose namespace, DS volume, gateway
namespace, and local cache. Randomized cases record seed and first divergent operation.

Proposed server-side files:

```text
packages/acceptance/
  src/stack.ts
  src/barrier.ts
  src/oracle.ts
  src/fault-gate.ts
  src/postgres18-contract.test.ts
  src/server-recovery.test.ts
  src/gateway-security.test.ts
  src/fixed-operation-boundedness.test.ts

packages/conformance/src/
  conformance-pg18-generated-columns.test.ts
  conformance-pg18-slot-invalidation.test.ts
  conformance-client-transaction-visibility.test.ts
```

Proposed Swift/app files and shared fixtures are specified in
[`23-swift-app-e2e-tdd-map.md#proposed-files-and-suites`](23-swift-app-e2e-tdd-map.md#proposed-files-and-suites).

## 4. Stable E2E scenario inventory

The IDs below are the stable public contracts. A refactor may replace every internal component while
retaining the scenario ID, source journal, and oracle. Each case is parameterized over all admitted
templates affected by the behavior, not just a toy table.

### 4.1 PostgreSQL 18 admission and continuity

| ID | Test action | Required result |
| --- | --- | --- |
| `PG18-E2E-001` | On real PG18, snapshot a template; commit insert/update/delete, predicate entry/exit, and a multi-row transaction; restart the engine and repeat. | At every named fence, materialization equals SQL; restart loses no acknowledged applied effect and any replay follows the selected event/response checkpoint contract. |
| `PG18-E2E-002` | Hold shape creation after its repeatable-read snapshot is fixed; commit a concurrent transaction; release creation. | Snapshot plus live state equals SQL exactly, proving the xid visibility fence. |
| `PG18-E2E-003` | Publish stored generated columns; snapshot and live-change source fields so rows enter, change within, and leave a projected/filtering template. | Generated values always equal SQL and never become synthetic `null`. |
| `PG18-E2E-004` | Track a table containing a virtual generated column. | Engine fails before readiness/feed creation with a stable actionable error; a snapshot-only success or live `null` is failure. |
| `PG18-E2E-005` | Track a stored generated field that the effective publication does not publish, including identity-key variants. | Engine fails before readiness with table/publication/column detail; it never serves a half-schema. |
| `PG18-E2E-006` | Produce real primary `idle_timeout` and `wal_removed` invalidations with reset off/on; use real restart for `wal_level_insufficient` when supported and focused policy fixtures for all documented/unknown reasons. | Any non-null reason latches fail-closed; authorized reset terminally retires old feeds and a fresh subscription equals SQL. Synthetic/standby-only coverage is labelled accurately. |
| `PG18-E2E-007` | Exercise setup/admin, pool/backfill/query-back and walsender independently through PG18 `hostssl` with SCRAM/`verify-full`; rotate/wrong-CA/wrong-SAN/reconnect one path after the others are healthy. | `pg_stat_ssl` proves each named connector is encrypted; identity failure fences freshness/readiness without downgrade; restoration converges. |
| `PG18-E2E-008` | Exclude `pgoutput`, omit a table/operation, add row filter/column list/RLS, regress identity, or attempt publication mutation with each runtime role; then use the sanctioned fenced change workflow. | Runtime mutation is denied. Authorized workflow makes readiness/public traffic unavailable before DDL/DML and jointly re-admits or retires affected tables; polling is diagnostics, not the safety fence. |
| `PG18-E2E-009` | Promote a PG18 physical standby without synchronized logical failover-slot support and redirect the engine. | Old epoch never continues; documented reset/fail-closed path occurs; a fresh feed equals the promoted primary. |
| `PG18-E2E-010` | Promote a standby with a synchronized, valid same-name `pgoutput` slot. | The first profile still treats the timeline change as an epoch break and rehydrates; this prevents the missing-slot branch from false-proving promotion policy. |
| `PG18-E2E-011` | Stop at fence A, commit B, drop/recreate the same-name slot on the same cluster, then restart; also test quiet behind/equal/ahead recreation. | Slot name/plugin never prove incarnation. A slot ahead of the durable landed frontier resets/fails closed; gap-free behind/equal cases follow the explicit redelivery decision. |
| `PG18-E2E-012` | Add/toggle virtual/stored generated columns, publication generated setting, RLS/policies and partition-root policy while live and while down. | Every lifecycle reuses joint table/publication admission; affected feeds retire/stay unresolved before service while an unaffected table remains live. |
| `PG18-E2E-013` | Exercise URL/keyword conninfo parity, escaped credentials, selected multi-host policy and channel-binding disposition for each connector. | All connectors interpret the approved identity policy equally; unknown/weaker settings fail preflight. |
| `PG18-E2E-014` | Run approved `18.N → 18.N+1` maintenance on the same data directory with before/after commits and restart cuts; exercise declared minor rollback/reset. | Cluster/slot/publication/frontier continuity is re-proved and the existing feed safely resumes or resets. PG19 and unsupported PG16/17 import fail/use full rehydrate. |

### 4.2 Server lifecycle, durability, and refactor safety

| ID | Test action | Required result |
| --- | --- | --- |
| `SRV-E2E-001` | Commit an operation-ID'd oversized transaction; externally withhold/loss-inject a storage response and restart the server. | Server source checkpoint never passes an incomplete commit; event-level replay may duplicate safely; final fenced materialization equals SQL. `NATIVE_TXN_ATOMIC` separately asserts one eligible per-stream batch. |
| `SRV-E2E-002` | Retry create/renew/release after response loss with two clients sharing one template. | One named claim per client; retry is idempotent; releasing one never interrupts the other. |
| `SRV-E2E-003` | Release the last claim; write through active→dormant→reactivated and dormant→evicted paths. | Reactivation replays to SQL; eviction terminally closes old feed; recreation has a new opaque generation. |
| `SRV-E2E-004` | Park a public tail, then purge, drift-retire, truncate-retire, or epoch-reset it. | Tail wakes with the documented terminal result and cannot continue stale. |
| `SRV-E2E-005` | Hold DS requests at create, join, release, dropped intent, close, delete, and retired completion; lose responses and restart. | No success precedes required durable intent; retry/restart leaves exactly one correct lifecycle outcome. |
| `SRV-E2E-006` | SIGKILL engine with active feeds, commit while absent, then start the same release image. | Readiness fences traffic; native continuation or documented compatibility refetch converges to SQL. |
| `SRV-E2E-007` | SIGTERM with a parked tail and commit pressure. | Readiness becomes unavailable before handoff, tails unblock, accepted work lands or resets by contract, successor converges. |
| `SRV-E2E-008` | Sustain writes with a dormant consumer across declared retention/storage bounds; apply process, external-storage and volume cuts. | Reactivated or reset client reaches fenced SQL, no acknowledged effect is lost, and physical storage stays inside the declared bound. Exact segment/control/checkpoint invariants remain focused same-SHA tests. |
| `SRV-E2E-009` | Apply schema drift during create, while live, and while engine is down, with an unaffected table as control. | Affected generations retire/refetch; unresolved tables refuse work; unaffected feed remains live. |
| `SRV-E2E-010` | Restore empty/corrupt/ahead/behind catalog, DS, and PG combinations, including changed system identifier. | Exact resume or whole-generation reset is selected before readiness; partial/empty catalog never serves as healthy. |
| `SRV-E2E-011` | One transaction changes two tracked tables, including membership/outer rows, across direct/circuit/routed/deferred templates. | Each stream's final fenced map equals SQL and the server source checkpoint is safe; `NATIVE_TXN_ATOMIC` remains per eligible stream and claims no cross-stream observer atomicity. |
| `SRV-E2E-012` | SIGKILL/restart the exact file-backed DS candidate around an accepted-but-unanswered append while a client tail is held. | Exact resume or typed whole-generation reset, never silent loss; final materialization equals SQL. |
| `SRV-E2E-013` | Start a successor while the former engine still owns the slot/volume, then confirm termination and retry handoff. | Successor never becomes gateway-routable or mutates DS until exclusive ownership is proven. |

### 4.3 Gateway authorization and public ownership

| ID | Test action | Required result |
| --- | --- | --- |
| `GW-E2E-001` | Mutate table/predicate/projection, template version, parameter type, duplicate keys, path, handle, or downstream ID. | Gateway rejects or canonicalizes before forbidden engine/DS work; clients never supply authority-bearing ASTs. |
| `GW-E2E-002` | Use two tenants with overlapping PKs; substitute feed/idempotency/claim IDs across them. | Each sees only its SQL rows and cannot enumerate, renew, read, or release the other's object. |
| `GW-E2E-003` | Send malformed, expired, revoked, wrong issuer/audience/signature/session credentials. | Non-enumerating denial, zero downstream data/lifecycle calls, zero cache effect, redacted logs. |
| `GW-E2E-004` | Lose responses at every gateway-registry↔engine lifecycle boundary, restart gateway, and retry the same public idempotency key. | Reconciliation yields exactly one owned internal claim or none according to the last acknowledged outcome. |
| `GW-E2E-005` | Hold a long poll and revoke: stop admission, cancel/join reads, invalidate generation, acknowledge; then release upstream. | Responses whose public headers/body began are recorded pre-barrier; uncommitted responses emit zero bytes, exact claim releases once, later requests deny. |
| `GW-E2E-006` | Scan from the public network for engine, API, DS, PG, metrics, graph/state/trace, catalog, and admin routes. | Only documented gateway methods/routes are reachable; internal identifiers and listeners are not exposed. |
| `GW-E2E-007` | Revoke/policy-change during create, renew and a multi-page/streaming snapshot body. | No stale renewal/cache-generation publication or post-barrier uncommitted body; lifecycle reconciles exactly once. |
| `GW-E2E-008` | Rotate/break public HTTPS and gateway↔engine↔DS TLS identities; attempt plaintext/stripping and reconnect. | Every hop authenticates the selected identity, fences on failure and converges after the dual-version rotation barrier without downgrade. |

### 4.4 Swift package and real-app behavior

The detailed Given/When/Then definitions live in
[`23-swift-app-e2e-tdd-map.md#reusable-contract-scenarios`](23-swift-app-e2e-tdd-map.md#reusable-contract-scenarios).
These IDs are mandatory as selected by profile:

| Contract group | IDs | Principal proof |
| --- | --- | --- |
| Bootstrap/live/reset | `SYNC-001`–`SYNC-005` | Snapshot/live SQL equality, restart/reset/refetch, no stale handle reuse. |
| Compatibility checkpoint | `COMPAT-001` | 199/200/201/400/maximum response chunk/crash matrix; cursor never outruns committed cache effects. |
| Lifecycle/mobile | `LIFE-001`–`LIFE-006` | Idempotent claim/close, 204, reconnect, suspension, cancellation at every external await. |
| Auth/account | `AUTH-001`–`AUTH-003` | Template admission, hard account-generation boundary, logout authority removal. |
| Codec/key/projection | `CODEC-001`–`CODEC-004` | Manifest-selected PG18→gateway→Swift→view/sink scalar fidelity, tagged Electric versus native keys, field presence and fail-closed eligibility. |
| Ownership/migration | `OWN-001`–`OWN-003`, `CUT-001`, `ROLL-001`, `ROLL-002` | Invisible shadow, no cross-owner delete, atomic promotion, fenced warm/cold rollback. |
| Optional native | `TXN-001`, `NATIVE-001`, `AGG-001`, `SUBSET-001` | Per-stream transaction batch, sink acknowledgement, exact aggregates, or page/live seam only in selected profiles. |

`COMPAT_V1` never gains eligibility merely because a happy-path test passes. Every production call
site must first pass the inventory, ownership, codec, and unsupported-mode admission cases.

### 4.5 Fixed-operation boundedness and fault corpus

| ID | Test action | Required result |
| --- | --- | --- |
| `BND-E2E-001` | Commit 64/128/129/512 MiB and production-max transactions with a small scratch budget; run external cuts plus adjacent instrumented invariant cuts. | Final state equals SQL; the server source checkpoint never passes an incomplete transaction; public replay follows the selected profile; memory/scratch follow the declared envelope and reclaim after success/recovery. |
| `BND-E2E-002` | Backfill a maximum admitted wide shape under a small append budget, with a concurrent live journal. | Bounded create resources, exact snapshot/live union, isolated typed rejection above the admission cap. |
| `BND-E2E-003` | Execute the checked-in mixed workload through maximum allowed clients/shapes/retained streams and release all claims. | Accepted/rejected counts match policy; queue, RSS, FD, socket, connection, DS/WAL/disk, and segment bounds hold and return to declared post-release baselines. |
| `BND-E2E-004` | Run the exact attempt/offered budget with minimum 10,000,000 commits and 100 executions of every named process/network/storage/schema/revocation cut. | Zero divergence outside the signed pre-run allowlist; deterministic stop/deadline, minimum counts, typed outcomes, seeds/digests/config/logs/metrics and first-divergence replay all validate. |
| `BND-E2E-005` | At limit-1/limit/limit+1 stall readers, cancel creates/snapshots, churn reconnects and hold one downstream unavailable. | Exact admission/backpressure/reset result, no lost acknowledged application, source/client checkpoint safety, and cleanup at a named lifecycle/clock event within absolute caps. |

There is no calendar-duration acceptance step. Capacity and long-run evidence are finite,
replayable operation corpora. A deadline may bound a single operation or recovery objective, but
calendar duration is never the promotion criterion.

## 5. TDD execution protocol

Every packet declares `proof_kind`: `genuine_red`, `inherited_control`, or `non_behavioral`.
Only `genuine_red` is a failure at the intended semantic assertion and may enter `red_proved` or
authorize the unchanged green behavior pair. `inherited_control` is recorded as characterization,
not red proof; `non_behavioral` needs no red proof. The author/merge direct gates, inherited-baseline
characterization, and non-waivable release qualification are distinct evidence lanes. A named
baseline-repair packet may consume its recorded base failure only at the exact assertion it owns,
then must turn that unchanged assertion green and pass all runnable direct gates. An unavailable
external qualification lane remains `blocked` for promotion without deadlocking the task that
installs it.

For every `genuine_red` behavior-changing packet:

1. Add or select its stable E2E scenario and build the smallest fixture needed to reach the public
   boundary.
2. Capture a red run proving the intended failure, not an environment failure. Include the command,
   scenario/seed, expected oracle, actual public observation, and first divergence.
3. Implement only enough behavior to make that scenario green; keep focused unit/property tests for
   algorithms, codecs, parsers, and state machines.
4. Refactor freely behind the black-box boundary.
5. Run the scenario's adjacent matrix and author/merge direct gates. Attach green evidence with
   unchanged test semantics; final profile/third-party/browser lanes remain release qualification
   requirements and cannot be waived.

A red test is execution evidence, not a release-gate success. Every `genuine_red` production packet
uses a consumer-bound `red_artifact` packet followed by independent review and a separate
implementation packet that works on that exact commit—even when one author performs both phases.
Integration merges only the green stack. Do not merge a skipped, inverted, or permanently
expected-failure version of the contract into the release branch.

Red, green, direct, qualification and reviewer evidence is valid only from the canonical clean
evidence runner: a fresh detached worktree at the exact commit, or a newly empty export of a verified
prepared-patch result tree. Each command retains pre/post source cleanliness or tree-manifest hashes
and the effective-config hash; immutable dependencies/tools/fixture inputs are bound by content and
read-only resolver/mount topology, while writable build/cache/fixture/artifact state uses a unique
initially empty external run root. Evidence from an author/control checkout, an undeclared/writable
overlay, stale dependency, reused run root, mutated source or reviewer reuse of the author's directory/
external state fails provenance even if the assertion is green.

## 6. Task authority and scenario ownership

This note is a scenario and rationale addendum. Task IDs, dependencies, applicability expressions,
principal boundaries, and acceptance text are authoritative only in
[the canonical execution specification](18-production-readiness-spec-reviewed.md) and the generated
task manifest created by `PLAN-001`. Repeating a task definition here is a validation error.

| Scenario family | Canonical contract/implementation/runner owners |
| --- | --- |
| `PG18-E2E-*` | `PG18-001A`/`001B`/`001Q`, `PG18-002A`/`002B`/`002C`, `PG18-003A`/`003Q`, `PG18-004`, `ENG-017`, `OPS-004`, `PGR-001` |
| `SRV-E2E-*` | `E2E-001R` contract registry; existing engine/storage implementation packets; `E2E-001Q` immutable-candidate runner |
| `GW-E2E-*` | `E2E-002R`; security/gateway/`GWR-*` implementation packets; `E2E-002Q` |
| compatibility Swift/app | `E2E-003CR`, `CMP-*`, `APP-OWN-001`, `E2E-003CQ` |
| native Swift/app | `E2E-003NR`, `SWF-*`, selected `APP-NATIVE-*`, `E2E-003NQ`; optional-module runners are profile-conditioned |
| `OWN-*`/`CUT-*`/`ROLL-*` | `E2E-004R`, `MIG-*`, `E2E-004Q` |
| `BND-E2E-*` | `CAP-*`, `TST-010`–`012`, and final runner `E2E-005` |

The scenario registry stores `scenario_id`, semantic contract hash, proof kind, test-owner task,
implementation-owner task(s), integration-runner task, profile expression, external action, public
oracle, and cut tier. An implementation packet owns the stacked red/green test patch. A qualification
runner may add adapters and evidence capture, but cannot change the journal, oracle, expected outcome,
or exclusion hash; changing any contract hash invalidates both implementation and qualification
evidence.

## 7. Scheduler-owned execution order

This note does not define fronts. The canonical bootstrap is `PLAN-001`; its validator then emits
`GOV-001` and `TST-000`, and only after `GOV-001` may it emit `GOV-002`, whose completion can make
`PG18-000` ready. Every later scenario, implementation, integration and qualification packet comes
only from the generated ready-set report for the exact task/profile closure. The numbered scenario
families above describe contracts, never assignment permission.
