# Electric Circuits production-readiness and Swift migration execution spec

Status: **superseded first synthesis draft**, 2026-08-22. It is retained for review traceability;
use [`18-production-readiness-spec-reviewed.md`](18-production-readiness-spec-reviewed.md) for execution.

## 1. Outcome

The target is a production deployment in which iOS clients can consume Electric Circuits without
direct access to Postgres, the engine control plane, or durable-streams administration. Postgres
remains the source of record. The first supported topology is one active engine per replication
slot with an explicit active/passive recovery procedure; this spec does not claim active/active HA.

The recommended Swift migration has two lanes:

1. **Compatibility pilot:** point the application's existing `electric-sync-swift` transport at
   Circuits' Electric-compatible `GET /v1/shape`. Keep this opt-in, use a new local cache generation,
   use long polling rather than SSE, and initially permit only collection modes whose correctness
   does not depend on Electric tags, `changes_only`, ordering, or limits.
2. **Native product:** build a small, independent `ElectricCircuitsSwift` package for native
   shapes, then aggregates. Add live subsets only after the server exposes a correct page/live
   visibility fence. Do not refactor the existing ElectricSync core into a dual backend first.

This is a migration to a separately owned fork, not an implementation of a publicly documented
Electric deprecation path. Public primary sources show current Electric is still released and do
not call Circuits its successor; see [the lifecycle research](10-deprecation-and-successor-context.md).
The production decision must therefore include explicit ownership of this fork and its support
burden.

## 2. Production target and exclusions

### Supported at the first production release

- PostgreSQL 16 with logical replication, a dedicated publication and slot, and required replica
  identities.
- File-backed durable-streams with persistent volumes, backup and restore.
- One active Circuits engine for a slot, fenced against a second active engine.
- A public TLS gateway that authenticates the app and derives tenant policy server-side.
- Electric-compatible shapes for the migration pilot.
- Native materialized shapes and live aggregates through a versioned, language-neutral contract.
- iOS 18+/macOS 15+, Swift 6.1+, Foundation `URLSession`, and no mandatory third-party Swift runtime
  dependencies.
- Application-owned local persistence through a narrow optional sink protocol.

### Not claimed by the first production release

- Direct Internet/mobile access to the engine, tRPC API, Postgres, or durable-streams.
- Arbitrary client-provided SQL/predicates in a multi-tenant deployment.
- Active/active engines sharing one replication slot.
- General SQL joins or ordered/limited live shapes.
- Native live subsets until `ENG-001` is complete.
- Observer-visible source-transaction atomicity until `ENG-002` is complete.
- SSE on the Circuits Electric adapter.
- Compatibility with Electric tags, DNF move-out ownership, or legacy `changes_only` semantics unless
  a task below explicitly proves it.
- A claim that public upstream supports, maintains, or will merge this fork.

## 3. Non-negotiable launch gates

Every gate requires committed evidence, not a declaration that work is “done.”

| Gate | Requirement | Closed by |
| --- | --- | --- |
| **G0 — ownership** | A recorded decision names the fork owner, supported topology/features, release policy, and response path if upstream never adopts the fork. | `GOV-001`–`GOV-004` |
| **G1 — isolation** | Only the authenticated TLS gateway is public. Postgres, engine, API, DS, metrics, and debug/admin routes are private under default-deny policy. | `SEC-001`–`SEC-008`, `OPS-001` |
| **G2 — tenant safety** | The server derives tenant predicates/projections from identity and approved templates; tests prove no cross-tenant read, subquery, stream, subscription, or admin access. | `SEC-002`–`SEC-005`, `TST-004` |
| **G3 — durability** | DS is file-backed; catalog/change/shape storage survives restore; slot/catalog epochs cannot silently skip WAL; upgrades and rollback preserve or intentionally retire every acknowledged shape. | `OPS-002`, `OPS-005`, `TST-003` |
| **G4 — bounded failure** | Queue, creation, snapshot, transaction, connection, and disk limits have explicit accounting, admission behavior, metrics, and overload tests. | `ENG-007`–`ENG-011`, `CAP-001`–`CAP-004` |
| **G5 — protocol** | Native HTTP/stream behavior is versioned, has stable errors and golden fixtures, and is not coupled to tRPC encoding. | `PROTO-001`–`PROTO-004` |
| **G6 — correctness** | The existing suites plus external Electric conformance, fault matrices, randomized oracle comparisons, and the subset fence tests are green. | `ENG-001`–`ENG-006`, `TST-001`–`TST-005` |
| **G7 — Swift** | The selected app collections pass compatibility pilot tests; native shape/aggregate lifecycle, cancellation, replacement, lease, precision, and reset tests are green. | `CMP-001`–`CMP-006`, `SWF-001`–`SWF-013` |
| **G8 — operations** | Reproducible production manifests, least privilege, observability, backup/restore, failover, schema migration, upgrade, and incident commands are executed successfully. | `OPS-001`–`OPS-009` |
| **G9 — capacity** | A versioned target envelope and reproducible Linux results establish the supported subscriptions, shapes, rows, write rate, fan-out, connections, disk, and recovery limits. | `CAP-001`–`CAP-004` |
| **G10 — migration** | Shadow comparison, cutover, and rollback complete against every production query template and cache ownership domain with zero unexplained divergence. | `MIG-001`–`MIG-005` |

## 4. Dependency order

```text
GOV-001/002 ──┬── PROTO-001 ──┬── SEC-002/003/004 ──┬── CMP compatibility pilot
              │               │                      └── SWF native shapes/aggregates
              │               ├── ENG-001 ────────────── SWF-009 native subsets
              │               └── PROTO-002/003/004 ─── TST contract suites
              ├── OPS-001/002/003 ── OPS-004/005/006/007/008
              └── ENG boundedness ── CAP instrumentation ── capacity evidence

All implementation lanes ── TST release matrix ── MIG shadow/cutover/rollback ── release
```

The compatibility pilot may begin after `CMP-001`, `CMP-002`, and a protected staging gateway,
but it cannot be called production before gates G0–G10 close. Native Swift API implementation may
begin against golden fixtures before the gateway is complete. `SWF-009` must not begin its public
API implementation until `ENG-001` and the corresponding fixtures are merged.

## 5. Completion contract for every work packet

Each task below is intended to fit one primary subagent/PR. The assignee must:

- read `AGENTS.md` and the cited research note/source areas;
- add or update the implementation, tests, operations documentation, and user documentation affected
  by the change;
- keep a short evidence note under `notes/execution/<task-id>.md` containing the starting SHA,
  decisions, commands, results, remaining risks, and links to artifacts;
- use stable, machine-checkable assertions; “observe for N days” is not an acceptance criterion;
- run the repository's required gates for engine-touching changes: `cargo fmt --check`,
  `pnpm typecheck`, `pnpm engine:test`, `ELECTRIC_CIRCUITS_ENGINE_PREBUILT=1 pnpm test`, and the
  external Electric conformance command, or record the exact environmental blocker;
- add focused regression tests before claiming a bug closed;
- avoid changing public protocol behavior without updating the versioned contract and fixtures; and
- leave unrelated user changes untouched and make no commit/push unless explicitly authorized.

For load/fault work, completion is based on a fixed operation count and declared assertions. The
default evidence floor is three independent repetitions, 10,000,000 committed row mutations for the
long correctness run, 10,000 lifecycle cycles per client implementation, and 100 injections for each
restart/failure cut point. A task may raise these counts. Lowering them requires an explicit ADR with
the statistical or resource rationale.

## 6. Governance and product definition

### GOV-001 — Record fork ownership and upstream posture

**Priority:** blocker. **Depends on:** none. **Suggested owner:** product/architecture agent.

Deliverables:

- Add an ADR that states whether this team owns the fork indefinitely, who approves releases and
  security fixes, and whether upstream merge is an optimization rather than a launch dependency.
- Record the verified distinction between old ElectricSQL, current Electric, and Circuits.
- List the exact maintainer questions about succession, roadmap, compatibility, and support. If those
  answers are unavailable, select the self-owned-fork branch in the ADR instead of waiting.
- Define how upstream/fork security advisories and dependency updates enter the local backlog.

Acceptance:

- The ADR contains one selected decision, named owners/roles, and explicit consequences; it has no
  “pending upstream response” state that blocks execution indefinitely.
- README and migration language no longer describe Circuits as an official successor without a dated
  primary-source statement.

### GOV-002 — Commit the production feature and capacity envelope

**Priority:** blocker. **Depends on:** `GOV-001`. **Suggested owner:** product + SRE agent.

Deliverables:

- Add `docs/production/support-matrix.md` with supported Postgres, Swift/platform, topology, API,
  query, type, schema-change, and recovery behavior.
- Add a machine-readable `docs/production/capacity-target.yaml` containing planned peak writes/s,
  row width, transaction size, active clients, feeds/client, distinct/shared shapes, rows/shape,
  aggregate groups, subset page size, reconnect wave, and RPO/RTO.
- Give every field a nonzero value or explicitly mark the feature unsupported.

Acceptance:

- Config validation and benchmark tools can load the file.
- Every later load test derives its dimensions from the same file; no task invents a different
  “production-like” workload.

### GOV-003 — Reconcile trackers and fork delta

**Priority:** required. **Depends on:** `GOV-001`. **Suggested owner:** release-management agent.

Deliverables:

- Convert [the upstream inventory](02-upstream-open-issues.md) and
  [fork inventory](15-fork-open-issues.md) into local tracked issues with links, local disposition,
  task ID, and regression-test evidence.
- Close/supersede obsolete parent red-test PRs and issues only when authorized; otherwise prepare the
  exact comments/patches for a human owner.
- Define the upstream rebase/cherry-pick policy and code areas where the 30-commit fork is expected to
  conflict.

Acceptance:

- Every public and parent-fork open issue maps to “fixed with test,” a task in this spec, a documented
  product exclusion, or an upstream-only decision.
- A script or release checklist reports unmapped issues and fork divergence.

### GOV-004 — Define versioning, compatibility, and release artifacts

**Priority:** blocker. **Depends on:** `GOV-001`, `PROTO-001`. **Suggested owner:** release agent.

Deliverables:

- Define semver for engine images, native protocol, TypeScript packages, and Swift package.
- Define catalog/change-log migration compatibility and the minimum rollback version.
- Produce immutable image tags/digests, changelog, compatibility matrix, SBOM/provenance links, and a
  release evidence manifest for each candidate.

Acceptance:

- A release-candidate workflow builds from a tag in a clean checkout and emits all artifacts.
- An automated test rejects an incompatible client/server protocol pair with a stable typed error.

## 7. Public protocol and gateway

### PROTO-001 — Specify a versioned native wire contract

**Priority:** blocker. **Depends on:** `GOV-002`. **Suggested owner:** protocol agent.

Deliverables:

- Specify language-neutral JSON/HTTP endpoints for shape and aggregate create/renew/release, shape
  inspection needed by clients, subset query, and durable stream read/HEAD.
- Specify table names, predicates, projections, composite keys, oversized integers as decimal strings,
  Postgres text values, nulls, aggregate `{value,n}`, leases, replacement handles, terminal stream
  states, pagination, offsets, LSNs, and transaction markers.
- Reserve a protocol version and content type/header. Do not expose tRPC batch encoding as the public
  Swift contract.
- State which fields are opaque, which may be persisted, and which may be logged.

Acceptance:

- OpenAPI/JSON Schema validates all checked-in examples and rejects malformed subscriptions,
  predicates, values, and unknown required-version combinations.
- Rust, TypeScript, and Swift consume the same fixture corpus; there are no hand-copied divergent
  examples.

### PROTO-002 — Stabilize errors, retry classes, and idempotency

**Priority:** blocker. **Depends on:** `PROTO-001`. **Suggested owner:** protocol/engine agent.

Deliverables:

- Define a versioned error envelope with machine code, retryability, safe message, request ID, and
  optional retry delay.
- Cover invalid input, unauthorized/forbidden, quota, subscription conflict, schema drift, epoch
  reset/degraded, stream closed/gone, replacement required, and incompatible protocol.
- Mark every mutation as idempotent or non-retryable. Public clients must use named subscriptions;
  anonymous decrement and `purge=true` are not client operations.

Acceptance:

- A table-driven engine/gateway suite asserts status, code, and retry class for every case.
- Lost-response tests repeat each public mutation and produce exactly one claim/release effect.

### PROTO-003 — Specify stream framing and transaction completion

**Priority:** blocker for transaction-aware native clients. **Depends on:** `PROTO-001`, `ENG-002`.
**Suggested owner:** protocol/engine agent.

Deliverables:

- Specify response headers, body framing, long-poll timeout, next offset, caught-up state, closed/gone
  signals, reconnection, and maximum page bytes.
- Specify `headers.last` on the final output envelope for each source transaction that emits to a
  stream, including fan-out, chunking, reconnect, and transactions producing no event for a shape.
- Specify whether client observers receive event or transaction atomicity.

Acceptance:

- Golden streams split every possible byte/page boundary and reconstruct the same ordered transaction
  sequence.
- A client disconnected after every envelope/page resumes without duplicate effect or partial
  committed transaction visibility.

### PROTO-004 — Add contract negotiation and deprecation tests

**Priority:** required. **Depends on:** `PROTO-001`–`PROTO-003`. **Suggested owner:** protocol agent.

Deliverables:

- Implement supported-version discovery/negotiation and minimum/maximum client version responses.
- Add fixture compatibility tests for current and previous supported protocol versions.
- Define additive-field, behavior-change, deprecation, and removal rules.

Acceptance:

- Previous supported Swift/TS clients pass against the new server fixture; unsupported clients fail
  before creating a shape.
- CI detects an unreviewed breaking schema/fixture diff.

## 8. Security and tenant isolation

### SEC-001 — Threat model and route/data classification

**Priority:** blocker. **Depends on:** `GOV-002`. **Suggested owner:** security agent.

Deliverables:

- Enumerate engine, API, DS, Postgres, metrics, graph/state/trace/memory, epoch reset, purge, and table
  mutation routes and classify each public/data-plane/admin/debug/internal.
- Trace tenant data through source WAL, `changes/*`, catalog, `shape/*`, logs, metrics, backups, and
  mobile caches.
- Model guessed stream paths, stolen subscriptions, arbitrary predicates/subqueries, projection
  widening, denial-of-service, replay, URL leakage, and confused-deputy attacks.

Acceptance:

- Every listening socket and route has an owner, required identity, authorization rule, data class,
  retention rule, and negative test ID.
- No “protected by trusted network” assertion exists without a corresponding manifest/network-policy
  control.

### SEC-002 — Build the authenticated public gateway

**Priority:** blocker. **Depends on:** `PROTO-001`, `SEC-001`. **Suggested owner:** gateway agent.

Deliverables:

- Expose only versioned client operations through one TLS origin.
- Authenticate app sessions using the product's identity provider; pass a normalized principal to
  authorization logic and refresh credentials without embedding secrets in stream URLs.
- Propagate request IDs and stable errors; redact authorization, cookies, query secrets, DS paths,
  predicates, and row data from normal logs.
- Keep engine, tRPC API, DS, and Postgres endpoints private.

Acceptance:

- Unauthenticated, expired, malformed, and wrong-audience credentials fail before any shape/catalog
  mutation.
- Gateway restart and credential refresh do not duplicate claims or lose the persisted stream offset.
- A network scan from the public/client network can reach only the gateway.

### SEC-003 — Enforce server-owned query templates and tenant predicates

**Priority:** blocker. **Depends on:** `SEC-001`, `SEC-002`, `CMP-001`. **Suggested owner:** policy agent.

Deliverables:

- Convert application call sites into named, versioned templates with allowed table, projection,
  predicate parameters, aggregate, ordering, and maximum result/page size.
- Derive tenant/access-cohort predicates from the authenticated principal; never trust a client
  tenant ID or arbitrary SQL/AST.
- Validate subquery inner tables and projections under the same policy.

Acceptance:

- Property tests mutate every client-controlled field and cannot broaden table, tenant, columns,
  subquery membership, page size, or aggregate scope.
- Revoking access prevents new reads/renewals and forces existing stream capability expiry or proxy
  denial according to the documented revocation bound.

### SEC-004 — Broker or capability-protect durable stream reads

**Priority:** blocker. **Depends on:** `SEC-002`, `SEC-003`. **Suggested owner:** gateway/DS agent.

Deliverables:

- Choose and implement either gateway-proxied stream reads or signed, path-bound, read-only,
  short-lived capabilities.
- Ensure a mobile credential can never append, create, close, or delete a stream and can never read
  `changes/*`, `meta/catalog`, or another tenant's `shape/*`.
- Define capability renewal/revocation without changing the durable offset identity.

Acceptance:

- Cross-shape, cross-tenant, path traversal, guessed ID, expired token, verb substitution, and direct
  DS-origin tests all fail.
- A valid client resumes the same stream after capability refresh without data loss or replay effect.

### SEC-005 — Split and protect admin/debug surfaces

**Priority:** blocker. **Depends on:** `SEC-001`. **Suggested owner:** engine/security agent.

Deliverables:

- Move epoch reset, purge, table mutation, metrics reset, graph/state/trace/memory, raw shape rows/logs,
  and schema definition behind a separate admin listener or remove them from production builds.
- Require operator identity and operation-specific authorization; add audit records for destructive
  calls.
- Disable trace and row-bearing introspection by default in production.

Acceptance:

- Public gateway credentials receive 404/403 for every admin/debug route.
- Production config fails validation if a row-bearing debug route is enabled on a public bind.
- Audit records identify actor, operation, target, outcome, and request ID without recording row data
  or credentials.

### SEC-006 — Encrypt every network hop and rotate secrets

**Priority:** blocker. **Depends on:** `OPS-001`. **Suggested owner:** platform-security agent.

Deliverables:

- Enable verified TLS for public traffic and mTLS/workload identity or equivalent private transport
  for gateway→engine/API/DS and engine→DS/Postgres.
- Add CA, certificate, hostname, and client-certificate configuration to the Rust DS client; remove the
  current plain-HTTP-only contradiction.
- Document automated rotation for identity-provider keys, DB credentials, DS identity, and TLS certs.

Acceptance:

- Tests reject untrusted CA, wrong hostname, expired cert, missing client identity, and plaintext on a
  TLS-required listener.
- Rotation succeeds with in-flight long polls, and old credentials fail after the documented overlap.

### SEC-007 — Add quotas, rate limits, and request budgets

**Priority:** blocker. **Depends on:** `GOV-002`, `ENG-007`–`ENG-010`. **Suggested owner:** gateway/engine agent.

Deliverables:

- Enforce per-principal/tenant and global limits for active subscriptions, distinct shapes, renewals,
  create concurrency, long polls, snapshot rows/bytes, predicate complexity, subquery cardinality,
  page size, and aggregate groups.
- Return stable quota errors before allocating large buffers or Postgres snapshots.
- Export current use, wait, reject, and shed counters without tenant-sensitive labels.

Acceptance:

- Boundary tests cover limit−1, limit, and limit+1; rejected work leaves no stream, catalog record,
  lease, PG transaction, or queue entry.
- An abusive tenant at every quota cannot violate another tenant's declared capacity/error budget in
  the fixed mixed-load test.

### SEC-008 — Harden images, dependencies, and client data handling

**Priority:** required. **Depends on:** `OPS-001`, `SWF-012`. **Suggested owner:** supply-chain agent.

Deliverables:

- Run containers as non-root with read-only root filesystems, dropped capabilities, resource limits,
  pinned base image digests, SBOMs, vulnerability scans, signatures, and provenance.
- Define dependency update and vulnerability-exception policy for Cargo, npm, SwiftPM, and base images.
- Define iOS cache protection, Keychain credential storage, privacy-safe logging, and cache deletion on
  logout/tenant switch.

Acceptance:

- Admission policy rejects unsigned/unscanned images and root/capability regressions.
- A clean mobile install, logout, tenant switch, backup/restore, and device-lock test proves credentials
  are not in URLs/UserDefaults/logs and tenant cache data cannot cross sessions.

## 9. Engine correctness and boundedness

### ENG-001 — Replace the subset LSN seam with a visibility-correct fence

**Priority:** blocker for native subsets. **Depends on:** `PROTO-001`. **Suggested owner:** replication/
query correctness agent.

Problem: the current subset algorithm creates a changes-only feed, captures a stream offset, queries
Postgres, and merges using an LSN. A WAL commit record can exist before the transaction is visible to
the query snapshot. LSN comparison alone can therefore omit or resurrect a row at the page/live seam.
The engine already solves the analogous shape-backfill problem with `SnapshotGate`; `/query` does not
expose an equivalent fence.

Deliverables:

- Write an ADR choosing either (a) a server-owned atomic page+feed operation or (b) an explicit query
  snapshot visibility token containing enough xid information to classify feed changes.
- Implement the fence across query response, feed envelopes, protocol fixtures, TypeScript client,
  and eventual Swift client. Do not expose a raw Postgres snapshot string unless its lifecycle and
  security properties are specified.
- Preserve tombstone and keyset pagination behavior, NULL ordering, and `COLLATE "C"` semantics.

Acceptance:

- A deterministic test holds a transaction in the WAL-recorded/not-yet-snapshot-visible window and
  proves exact convergence for insert, update-in, update-out, delete, and key reorder.
- At least 100,000 randomized page/feed interleavings compare every emitted window with a
  repeatable-read SQL oracle; zero unexplained divergence is allowed.
- Client implementations cannot accidentally substitute a bare LSN for the new fence type.

### ENG-002 — Propagate transaction completion to output streams

**Priority:** blocker for transaction-atomic native delivery. **Depends on:** `PROTO-003`.
**Suggested owner:** sequencer/output agent.

Deliverables:

- Preserve source transaction identity and stamp `headers.last=true` on the final envelope emitted to
  each affected shape/aggregate stream.
- Define chunking when a transaction's output exceeds an append budget and define a no-output
  transaction without emitting fake row changes.
- Keep the sequencer invariant: no source transaction is published/advanced before all of its shape
  appends land or the affected shape is retired.

Acceptance:

- Unit/integration tests cover one/many shapes, one/many chunks, zero output, reconnect at each chunk,
  duplicate ingest, and retirement during append.
- A Swift/TS fixture can buffer until `last`, apply once, crash after each envelope, resume, and produce
  exactly one local transaction effect.

### ENG-003 — Make terminal stream outcomes uniformly recoverable

**Priority:** blocker. **Depends on:** `PROTO-002`. **Suggested owner:** engine + TypeScript client agent.

Deliverables:

- Map a gone/closed backing stream on `/v1/shape` to `409 must-refetch`, never generic 500
  (parent-fork issue #18).
- Teach the native TypeScript client to classify `stream-closed`, 404, and 410; recreate with the same
  named subscription, accept a replacement handle, reset its fold/page state, and resume
  (parent-fork issue #17).
- Ensure recreation does not release a new claim through an old reader's cleanup path.

Acceptance:

- Focused tests cover retention eviction, schema drift, purge, epoch reset, engine restart, DS restart,
  and false proxy 404 for both surfaces.
- Each test either resumes with exact state or emits one typed terminal configuration error; no tail
  hangs and no registered shape becomes silently stale.

### ENG-004 — Fence replayed TRUNCATE retirement

**Priority:** required correctness/availability. **Depends on:** existing `SnapshotGate` and catalog.
**Suggested owner:** schema-drift agent.

Deliverables:

- Persist/recover sufficient seed visibility for a restored shape and ignore a replayed TRUNCATE whose
  xid was already visible to that shape's seed.
- Retain fail-closed retirement for a TRUNCATE not reflected in the shape.

Acceptance:

- Crash after shape creation but before replication-slot acknowledgement, restart, and replay the same
  TRUNCATE; a post-TRUNCATE shape remains live while a pre-TRUNCATE shape is retired.
- Repeat across circuit, routing, fallback, aggregate, and subquery dependents.

### ENG-005 — Discover new and re-created selected tables safely

**Priority:** required if dynamic selectors are supported; otherwise make restart an explicit product
exclusion. **Depends on:** schema generations/resolve locks. **Suggested owner:** schema agent.

Deliverables:

- Re-resolve `ELECTRIC_CIRCUITS_PG_TABLES` on reconciliation or add an authenticated table-reload
  operation.
- Install introspection, publication/identity validation, schema holders, decoder, and circuit input
  without admitting a create against partial state.
- Detect drop/re-create identity and retire old dependents before accepting the new relation.

Acceptance:

- Conformance covers matching-table create, drop/re-create, selector expansion/contraction, concurrent
  client create, and DML during resolution.
- If excluded, config/docs/tests reject or clearly require restart rather than silently ignoring a new
  table.

### ENG-006 — Bound replication connection and classify startup failures

**Priority:** required. **Depends on:** none. **Suggested owner:** Postgres connectivity agent.

Deliverables:

- Apply a configurable deadline to replication connect and first receive; shutdown cancellation must
  interrupt both (parent-fork issue #14).
- Distinguish permanent authentication/configuration/TLS failures from retryable network failures,
  including errors without SQLSTATE (parent-fork issue #16).
- Emit stable readiness reasons and exit codes for permanent failures.

Acceptance:

- Blackholed address, refused port, bad password, missing TLS CA, wrong database, non-logical WAL,
  busy slot, and shutdown-during-connect each meet the documented deadline/action.
- Repeated identical permanent failures exit rather than retry forever; a transient outage recovers
  without shape loss.

### ENG-007 — Bound flip, emission, and fan-out queues by bytes and count

**Priority:** blocker. **Depends on:** `GOV-002`. **Suggested owner:** sequencer/backpressure agent.

Deliverables:

- Replace unbounded flip and ordered-emission channels with byte/count-accounted queues.
- Bound `txn_pending` fan-out and define how backpressure reaches the sequencer/ingestor without
  acknowledging WAL early or dropping an affected shape's output.
- Export queued items/bytes, oldest age, wait time, capacity, rejections, and degradation reasons.

Acceptance:

- Slow/failed DS, hot membership flip, maximum fan-out, and a transaction over the spill threshold
  remain within the configured memory envelope and preserve exact oracle state.
- Shutdown drains or safely checkpoints bounded queues; no task blocks cancellation indefinitely.

### ENG-008 — Bound concurrent creates and pending backfill deltas

**Priority:** blocker. **Depends on:** `GOV-002`, `SEC-007`. **Suggested owner:** lifecycle agent.

Deliverables:

- Add global/per-tenant create concurrency and pending-delta byte budgets.
- Backpressure, spill, or reject before a pending shape buffers an unbounded live-change set during
  backfill.
- Define cleanup for cancellation, timeout, quota rejection, schema drift, and joiners waiting on a
  create.

Acceptance:

- A create storm at the target reconnect wave plus continuous writes never exceeds its declared
  budget; accepted shapes converge and rejected shapes leave no stream/catalog/share/registry state.
- Cancellation is injected at every create phase and all invariants remain true.

### ENG-009 — Bound snapshot, inner-set, and diagnostic materialization

**Priority:** blocker. **Depends on:** `SEC-007`. **Suggested owner:** query/backfill agent.

Deliverables:

- Add row/byte/time budgets for Electric snapshots, `/query`, inner subquery seeds, raw rows/logs, and
  expensive memory/state diagnostics.
- Stream/page or spill materialization where the protocol permits; reject before allocation otherwise.
- Rate-limit/admin-protect diagnostics and cancel work when clients disconnect.

Acceptance:

- Narrow/wide rows at limit−1/limit/limit+1 have deterministic results and stable quota errors.
- Disconnect and timeout release the PG snapshot, buffer, permit, and response task.
- Peak RSS for the maximum accepted request stays within the capacity manifest's request budget.

### ENG-010 — Make output disk accounting durable and enforceable

**Priority:** blocker. **Depends on:** `OPS-002`, `GOV-002`. **Suggested owner:** storage/retention agent.

Deliverables:

- Persist or reconstruct per-stream/tenant/global shape-output bytes across restart.
- Enforce configured budgets with a documented eviction/refusal policy that never silently drops a
  registered shape's committed event.
- Account for active streams, dormant pins, segmented input log, catalog, pending retirement, and
  temporary spill space.

Acceptance:

- Restart does not reset accounting; limit crossing has the same result before and after restart.
- Disk-full injection at each append/rotation/catalog/retirement phase either recovers or fails closed
  without advancing the slot/checkpoint past unlanded data.

### ENG-011 — Complete orphan stream reconciliation

**Priority:** required. **Depends on:** DS list/index capability, `OPS-002`. **Suggested owner:** DS/
retirement agent.

Deliverables:

- Add an authenticated DS list/index primitive or another durable inventory so the engine can find
  `shape/*` streams absent from the catalog, not only catalog-known pending retirements.
- Reconcile with a creation/retirement fence so GC cannot delete a newly minted live stream.
- Bound enumeration and expose planned/deleted/skipped/error metrics.

Acceptance:

- Seed unknown, partially-created, pending-retirement, reused-looking, and live streams; only true
  orphans are closed then deleted.
- Crash after each GC phase and rerun to the same final state.

### ENG-012 — Validate every production configuration before serving

**Priority:** blocker. **Depends on:** `OPS-001`, `SEC-006`. **Suggested owner:** configuration agent.

Deliverables:

- Parse/validate/redact DS URLs; reject memory DS, public debug binds, unlimited shapes/budgets,
  plaintext-required hops, missing spill directory, invalid retention relationships, and no-op/unknown
  environment variables in production mode.
- Either implement the dedicated Prometheus listener or reject `ELECTRIC_PROMETHEUS_PORT`; align docs
  and tests (parent-fork issue #15).
- Emit a sanitized effective-config summary and stable preflight failures.

Acceptance:

- Table-driven tests cover every configuration variable and unsafe combination.
- The production manifest passes; the current demo defaults fail when relabelled as production.

### ENG-013 — Add a database-level single-writer fence

**Priority:** blocker for automated failover. **Depends on:** `OPS-004`. **Suggested owner:** epoch/
leadership agent.

Deliverables:

- Acquire and continuously validate a Postgres advisory lock or equivalent lease bound to the slot,
  database/system identifier, stack ID, and catalog epoch.
- Fence old leaders before a promoted engine can write catalog/shape output or acknowledge WAL.
- Treat a busy legitimate walsender as waiting, but never allow two local control planes to mutate the
  same shape namespace.

Acceptance:

- Start two engines simultaneously and at every failover cut point; exactly one reaches readiness and
  produces output.
- Partition, pause, kill, and resume the old leader in 100 deterministic runs; it cannot append after
  losing the fence.

## 10. Production deployment and operations

### OPS-001 — Ship a protected production deployment package

**Priority:** blocker. **Depends on:** `GOV-002`, `SEC-001`. **Suggested owner:** platform agent.

Deliverables:

- Separate demo/dev Compose from production Helm/Kustomize or equivalent manifests.
- Give Postgres, DS, engine, gateway, and metrics explicit private services, default-deny network
  policies, service identities, resource requests/limits, persistent volumes, disruption policy, and
  ordered startup/shutdown.
- Run one active engine; prevent a generic replica count greater than one unless `ENG-013` is enabled.
- Set termination grace above the engine drain budget and test forced termination behavior.

Acceptance:

- Policy tests prove only intended service-to-service paths and the public gateway ingress.
- A clean environment deploys from immutable digests, passes preflight/readiness, serves a shape, and
  drains on rollout without a lost committed change.

### OPS-002 — Prove durable-stream backup, restore, and disk recovery

**Priority:** blocker. **Depends on:** `OPS-001`, `ENG-010`. **Suggested owner:** storage SRE agent.

Deliverables:

- Document and automate consistent backup/restore of DS WAL/data plus required catalog metadata.
- Define encryption, retention, integrity verification, free-space reserve, expansion, and disk-full
  recovery.
- Record the relationship between DS restore point and Postgres slot/catalog epoch; refuse an unsafe
  mismatched restore instead of serving stale shapes.

Acceptance:

- Restore onto an empty host from each supported backup type and compare catalog, segment positions,
  shape streams, leases, and SQL oracle state.
- Corrupt/truncate one artifact and prove integrity checks fail closed with a named recovery path.
- Run 100 crash points across backup/restore/startup; no acknowledged shape is silently omitted.

### OPS-003 — Automate least-privilege Postgres setup and validation

**Priority:** blocker. **Depends on:** `GOV-002`, `SEC-006`. **Suggested owner:** database agent.

Deliverables:

- Provide idempotent setup for roles, grants, publication, replication slot, `wal_level`, WAL retention,
  replica identity, schema allowlist, TLS, and connection limits.
- Separate replication/read-query credentials from migration/admin credentials where feasible.
- Add preflight reporting for unsupported types, missing PK/identity, column-list publications, and
  projected sensitive columns.

Acceptance:

- Setup works on a blank supported PG16 instance, repeating it changes nothing, and an intentionally
  underprivileged engine fails with an actionable preflight result.
- Tests cover non-logical WAL, hand-made column-list publication, missing slot, lost slot, and foreign
  plugin/cluster identity.

### OPS-004 — Implement and execute active/passive failover

**Priority:** blocker for the supported topology. **Depends on:** `ENG-013`, `OPS-001`–`OPS-003`.
**Suggested owner:** reliability agent.

Deliverables:

- Define promotion, fencing, slot verification, readiness routing, DNS/service switch, and rollback.
- Define explicit RPO/RTO behavior for same-cluster engine loss, DS loss, Postgres primary failover,
  slot loss, and cluster identity change.
- Automate the non-destructive path and require a distinct authorized operation for epoch reset.

Acceptance:

- Execute 100 deterministic failovers at enumerated cut points with continuous writes and readers.
- Exactly one leader serves; every committed row either appears once in the logical client state or the
  protocol orders a full refetch. Results converge with the SQL oracle inside the declared operation/
  latency bounds.

### OPS-005 — Version and test catalog/change-log upgrades and rollback

**Priority:** blocker. **Depends on:** `GOV-004`, `OPS-002`. **Suggested owner:** storage migration agent.

Deliverables:

- Replace boot-fatal “reset storage” format changes with versioned readers/migrations or an explicit
  export/retire/reseed operation.
- Define forward and rollback compatibility for catalog events, offsets/highwater, segment controls,
  epoch binding, leases, and pending retirement.
- Make migration resumable and idempotent.

Acceptance:

- Upgrade from every supported prior fixture, crash after every migration write, resume, and compare
  the folded catalog.
- Roll back to the declared minimum version or fail before mutation; no half-migrated engine serves.

### OPS-006 — Complete low-cost production observability

**Priority:** blocker. **Depends on:** `ENG-007`–`ENG-012`. **Suggested owner:** observability agent.

Deliverables:

- Export authenticated Prometheus/OpenTelemetry metrics for commit-to-append latency, confirmed-flush
  and retained WAL, sequencer/transaction bytes+age, all queues, PG pool waits, flip/query-back work,
  DS append/read/fsync/FD/disk, backfills, long polls, reconnects, catalog/retirement retries, quotas,
  and leadership.
- Add structured, redacted logs and traces with request/shape correlation but no row/predicate/secret
  data by default.
- Check in dashboards and alert rules derived from `capacity-target.yaml`.

Acceptance:

- A fixture triggers every alert condition and validates metric existence/labels without unbounded
  cardinality.
- The metrics path remains responsive under maximum accepted data-plane load and does not perform a
  heap walk.

### OPS-007 — Write executable incident and maintenance runbooks

**Priority:** required. **Depends on:** `OPS-002`–`OPS-006`. **Suggested owner:** SRE runbook agent.

Deliverables:

- Add exact diagnosis/recovery commands for DS unavailable/full/corrupt, catalog retry, WAL growth,
  slot lost/busy, epoch broken, schema drift/unresolved, TRUNCATE, flip degradation, backfill storm,
  queue saturation, bad release, certificate rotation, tenant revocation, and client resync storm.
- Label destructive actions, prechecks, authorization, expected output, rollback, and verification.
- Turn safe checks and rehearsals into scripts rather than prose-only shell fragments.

Acceptance:

- In an isolated environment, an agent unfamiliar with the implementation executes every scenario
  from the runbook and reaches its asserted final state using only the documented inputs.
- Destructive scripts require an explicit target/confirmation and reject broad/unresolved targets.

### OPS-008 — Make schema/application deployment coordinated and fail-closed

**Priority:** blocker. **Depends on:** `ENG-004`, `ENG-005`, `OPS-003`. **Suggested owner:** database
migration agent.

Deliverables:

- Define expand/migrate/contract rules for tracked tables, PKs, replica identity, types, projections,
  circuits layout fingerprints, and query templates.
- Add pre-deploy compatibility checks and a controlled retire/reseed/recreate path.
- Prevent application rollout from relying on a new template/circuit layout before the engine supports
  it.

Acceptance:

- Execute additive column, type change, rename, PK/identity change, table replace, TRUNCATE, template
  add/remove, and rollback fixtures; each either stays live safely or forces an explicit typed resync.

### OPS-009 — Publish release qualification and rollback automation

**Priority:** blocker. **Depends on:** all other OPS tasks and `TST-001`–`TST-006`.
**Suggested owner:** release-engineering agent.

Deliverables:

- Build one command/workflow that validates source cleanliness, versions, migrations, images, SBOM,
  signatures, protocol fixtures, test evidence, benchmark manifest, deployment diff, and rollback
  compatibility.
- Produce a machine-readable release evidence bundle keyed by source and image digest.

Acceptance:

- A candidate with a deliberately missing/failed artifact cannot be promoted.
- Deploy and roll back the qualified artifacts in a clean environment while continuous oracle traffic
  remains convergent or receives the documented reset signal.

## 11. Capacity and performance evidence

### CAP-001 — Instrument every bounded resource

**Priority:** blocker. **Depends on:** `GOV-002`, `ENG-007`–`ENG-011`, `OPS-006`.
**Suggested owner:** performance instrumentation agent.

Deliverables:

- Add byte/count/age/high-water metrics for the sequencer transaction, flush waves, pending creates,
  live-delta buffers, flip/emission queues, query-back pool, snapshots, DS operations, stream backlog,
  spill, retained input segments, output streams, and client long polls.
- Add a workload manifest and raw JSON/CSV output schema that records SHA, images, kernel/hardware,
  PG/DS config, durability, seed, workload dimensions, and errors.

Acceptance:

- Every configured limit has current, limit, rejection, and peak evidence.
- A synthetic test drives each resource above 80% and proves its metric/alert changes without high-
  cardinality labels or material throughput collapse from instrumentation.

### CAP-002 — Produce paired Electric/Circuits protocol benchmarks

**Priority:** required for migration choice. **Depends on:** `CAP-001`, `GOV-002`.
**Suggested owner:** benchmark agent.

Deliverables:

- Run the unmodified Electric benchmarking fleet against this exact Circuits candidate and a pinned
  current Electric image on identical isolated Linux resources, durable storage, DB config, seed, and
  fleet revision.
- Run every fleet workload at scale 1 and at the declared target scale, three independent repetitions.
- Commit report generation and durable raw artifacts; distinguish correctness, latency, throughput,
  CPU/RSS/IO/disk, and operational differences.

Acceptance:

- The report can be regenerated from its manifest and contains p50/p95/p99/p999/max plus confidence/
  run-to-run spread.
- No “faster/slower” claim is made from incomparable durability, platform, or missing raw samples.

### CAP-003 — Establish the supported capacity table

**Priority:** blocker. **Depends on:** `CAP-001`, engine boundedness tasks.
**Suggested owner:** load/capacity agent.

Deliverables:

- Exercise distinct versus shared shapes, active/dormant mix, native versus `/v1` snapshots, narrow/
  wide rows, circuit/routing/fallback, subquery cardinality, aggregate group cardinality, connection
  scale, and create/reconnect storms.
- For each supported deployment size, publish sustainable writes/s, append bytes/s, active polls,
  distinct shapes, subscriptions, result rows, queue peaks, PG/engine/DS/client CPU/RSS/IO/FD/disk,
  and limiting component.
- Run a fixed 10,000,000-mutation steady workload at 70% of the declared peak and a fixed
  1,000,000-mutation burst at 2× peak. Fit/report post-warm-up RSS, FD, disk, and lag slopes rather
  than relying on a calendar soak.

Acceptance:

- Zero oracle divergence or lost committed changes; no unbounded positive RSS/FD/lag slope after
  accounting for retained data; all peaks remain inside declared headroom.
- At 70% load, the candidate meets the latency/RPO/RTO values selected in
  `capacity-target.yaml`; after the fixed 2× burst it drains to the steady band within that file's
  recovery bound.
- The production admission defaults do not exceed measured capacity.

### CAP-004 — Characterize failure and recovery capacity

**Priority:** blocker. **Depends on:** `CAP-003`, `OPS-004`. **Suggested owner:** chaos/performance agent.

Deliverables:

- At declared load, inject DS latency/outage/disk-full, PG disconnect/failover, gateway restart,
  engine termination, slot busy/lost, large transaction (64 MiB, 128 MiB, 2× spill cap, declared max),
  high-fan-out commit, flip storm, schema drift, and client reconnect wave.
- Execute 100 deterministic injections per cut point and preserve seed/event logs for replay.
- Measure peak memory/scratch/disk/WAL, head-of-line latency, backlog drain operations/time, resync
  count, and success/reset outcome.

Acceptance:

- Every accepted transaction either lands exactly once in logical client state or triggers the
  documented full-refetch boundary; the slot/checkpoint never advances past unlanded output.
- Resource limits shed/reject before process OOM/disk corruption; recovery meets the declared bounds
  without manual state editing.

## 12. Existing `electric-sync-swift` compatibility lane

### CMP-001 — Inventory real app call sites and classify compatibility

**Priority:** blocker for migration. **Depends on:** `GOV-002`. **Suggested owner:** Swift/app analysis agent.

Deliverables:

- Enumerate every production collection, predicate/parameter, projection, table, sync mode, transport,
  `changes_only`, order/limit/subset use, tags/DNF ownership dependency, cache table, background use,
  and observer transaction requirement.
- Collapse call sites into server query templates and mark each `v1-compatible`, `native-shape`,
  `native-aggregate`, `native-subset-after-ENG-001`, `requires-redesign`, or `remove`.
- Record result cardinality/row width/write fan-out in `capacity-target.yaml`.

Acceptance:

- Every sync call site has exactly one disposition, owner, test fixture, cache ownership domain, and
  cutover/rollback flag.
- No wildcard “all other queries” category remains.

### CMP-002 — Characterize `/v1/shape` against the Swift protocol assumptions

**Priority:** blocker. **Depends on:** `CMP-001`, `ENG-003`. **Suggested owner:** Swift compatibility agent.

Deliverables:

- Add an integration matrix for bootstrap `offset=-1`, headers/body decoding, 204 idle long poll,
  resume, 409 must-refetch, 400/503/5xx, handle TTL/restart, values/schema, composite keys, delete/
  predicate move-out, projection, parameters, cancellation, and duplicate pages.
- Explicitly test that unsupported `log=changes_only`, SSE, order, limit, subset, tags, removed tags,
  `activeConditions`, and move-in/out are rejected or never selected.
- Compare the 13 passing and two tag-related Electric external conformance cases with the app's actual
  semantics.

Acceptance:

- Each `v1-compatible` template passes snapshot+live+reset against a real PG/engine/DS stack.
- A compile/runtime capability guard prevents an unsupported ElectricSync collection mode from being
  configured with the Circuits endpoint.

### CMP-003 — Implement or configure the compatibility HTTP provider

**Priority:** required. **Depends on:** `CMP-002`, `SEC-002`. **Suggested owner:** Swift networking agent.

Deliverables:

- First determine whether the app's existing injected `HTTPClientProvider` can simply target the new
  base URL. If so, keep `electric-sync-swift` unchanged and add app configuration/tests only.
- If a transport is missing, add an opt-in product/provider using `URLSession`: encode `where`/params/
  columns and handle/offset, parse Electric headers and messages, turn 204 into an idle/up-to-date
  boundary the existing batch loop accepts, and repeat long polls with cancellation.
- Inject a `Sendable` credential provider; do not put durable secrets in persisted URLs or logs.

Acceptance:

- One poll per handle; cancellation ends the request promptly; retry honors `Retry-After` and existing
  circuit-breaker policy.
- URL-protocol unit tests plus real-stack tests cover every outcome in `CMP-002`.
- Existing Electric service behavior and public API remain unchanged when the provider is not selected.

### CMP-004 — Isolate Circuits cache and resume generations

**Priority:** blocker. **Depends on:** `CMP-001`, `CMP-003`. **Suggested owner:** Swift persistence agent.

Deliverables:

- Give Circuits a distinct semantic/cache generation and metadata namespace. Never reuse old Electric
  handles, offsets, cursors, tag ownership, or native stream positions.
- Define an atomic full reset: invalidate old generation rows/ownership and persist new bootstrap state
  in the same local DB transaction or use separate shadow tables.
- Preserve the old Electric generation for rollback until cutover evidence closes.

Acceptance:

- Crash/cancel at every reset/bootstrap persistence step and reopen; the app exposes either the old
  complete generation or new complete generation, never a mixed cache.
- Account/tenant switch cannot surface a previous generation's rows.

### CMP-005 — Run a compatibility shadow pilot by operation count

**Priority:** blocker. **Depends on:** `CMP-001`–`CMP-004`, `MIG-001`. **Suggested owner:** app integration agent.

Deliverables:

- For every `v1-compatible` template, run current Electric and Circuits into isolated shadow stores
  from the same canonical write trace.
- Include bootstrap during writes, reconnect, 204s, handle expiry, engine/DS/gateway restart, schema
  retirement, network loss, process cancellation, and app relaunch.
- Compare normalized row maps and observer events; retain seeds and first divergence diagnostics.

Acceptance:

- At least 1,000,000 committed mutations and 10,000 reconnect/lifecycle cycles produce zero unexplained
  final-state divergence for every template.
- Expected tag/control representation differences are normalized by an explicit comparator rule, not
  discarded generically.

### CMP-006 — Ship the compatibility path as an opt-in, reversible release

**Priority:** required. **Depends on:** `CMP-005`, production server gates. **Suggested owner:** Swift release agent.

Deliverables:

- Add feature/config flags per query template, independent cache identity, telemetry, user-facing error
  recovery, and one-operation rollback to the old service/cache owner.
- Document supported/unsupported ElectricSync modes and the Circuits endpoint contract.
- Keep native package naming/API separate.

Acceptance:

- Automated cutover and rollback fixtures switch each template independently without cursor reuse,
  double ownership, or deletion of the rollback cache.
- The release artifact contains no default behavior change for existing ElectricSync users.

## 13. New `ElectricCircuitsSwift` package

### SWF-001 — Scaffold independent modules and CI

**Priority:** required. **Depends on:** `PROTO-001`, `GOV-004`. **Suggested owner:** Swift package agent.

Deliverables:

- Create `ElectricCircuitsProtocol`, `ElectricCircuitsTransport`, and `ElectricCircuitsClient` products;
  add optional `ElectricCircuitsReplica` only when `SWF-007` starts.
- Target Swift 6.1, iOS 18, macOS 15; enable strict concurrency. Runtime targets have no mandatory
  third-party dependency.
- Add format/lint policy, dependency-boundary check, Linux/macOS protocol tests where feasible, and
  iOS simulator lifecycle tests.

Acceptance:

- A clean clone builds/tests all products and verifies Protocol does not import networking/storage and
  Transport/Client do not import an ORM/database.

### SWF-002 — Implement exact protocol models and codecs

**Priority:** blocker for native Swift. **Depends on:** `SWF-001`, `PROTO-001`–`PROTO-004`.
**Suggested owner:** Swift protocol agent.

Deliverables:

- Implement `TableRef`, predicate AST, shape/aggregate definitions, handles, leases, offsets, LSN/
  snapshot fences, stream envelope, stable errors, and reset/replacement events as `Sendable` values.
- Decode declared integers from JSON number or decimal string without precision loss; preserve unknown
  scalar text; model NULL and field presence separately.
- Keep composite/single `key` opaque for identity. Put optional key decoding behind a separately tested
  codec matching backslash/U+001F escaping.
- Preserve unknown additive fields where forward compatibility requires it and fail with decoding
  context rather than `try?`.

Acceptance:

- All shared golden fixtures round-trip/canonicalize as specified, including `Int64` boundaries,
  out-of-range numeric strings, composite escaping, timestamps, UUID text, null/missing, aggregate
  nulls, and unknown fields.
- Fuzz decoders with malformed/truncated/oversized inputs and enforce body/depth limits.

### SWF-003 — Build the authenticated URLSession transport

**Priority:** blocker. **Depends on:** `SWF-002`, `SEC-002`, `SEC-004`. **Suggested owner:** Swift networking agent.

Deliverables:

- Implement create/renew/release/query and stream HEAD/read/long-poll through the gateway with injected
  `URLSession`, clock, backoff, randomness, and `Sendable` credential provider.
- Classify HTTP, protocol, cancellation, offline, TLS, decoding, quota, terminal stream, and replacement
  outcomes. Retry only idempotent operations and honor server delay.
- Apply response/body/page limits, TLS trust policy from the app, request IDs, redacted diagnostics,
  and prompt cancellation. Do not use reachability preflight or Network.framework for HTTP.

Acceptance:

- Deterministic virtual-clock tests enumerate all status/error classes, credential refresh, partial
  body, idle timeout, connection loss, redirect rejection, and cancellation after every await.
- Named create/release lost-response tests have exactly one server-side effect.

### SWF-004 — Implement the subscription lifecycle actor

**Priority:** blocker. **Depends on:** `SWF-003`. **Suggested owner:** Swift concurrency agent.

Deliverables:

- One actor owns definition, named subscription, handle, durable offset, generation, tail task, renewal
  task, close state, and replacement transition.
- Persist the subscription identity before tailing; renew at a bounded fraction of `leaseSeconds` with
  jitter; serialize renewal and one-shot close; stop and await renewal/tail before identified delete.
- Recheck generation/closed state after every suspension so stale responses cannot resurrect a closed
  or replaced subscription.

Acceptance:

- A deterministic state-machine test executes 10,000 create/renew/read/replace/suspend/resume/close
  cycles, injecting cancellation at every await; at most one active claim/tailer remains and close is
  idempotent.
- Lease lapse/replacement performs a full documented rehydrate, never continues an old stream offset
  on the new handle.

### SWF-005 — Implement bounded stream reading and recovery

**Priority:** blocker. **Depends on:** `SWF-003`, `SWF-004`, `PROTO-003`, `ENG-003`.
**Suggested owner:** Swift streaming agent.

Deliverables:

- Expose a single-consumer or explicitly multicast `AsyncSequence` with documented ordering,
  cancellation, caught-up, reset, and terminal behavior.
- Bound buffered envelopes/bytes. On overflow, cancel the reader and recreate/resnapshot rather than
  dropping an arbitrary event.
- Advance/persist offsets only after the sink/consumer acknowledgement defined by the API. Recover from
  closed/404/410 and replacement handles without an old task winning a race.
- If `ENG-002` is enabled, buffer/spill through `headers.last`; otherwise explicitly expose event-level
  atomicity.

Acceptance:

- Split fixtures at every byte/envelope/page boundary, duplicate responses, stall consumers, overflow
  buffers, and disconnect after every event. Final materialized state is exact or one reset is emitted.
- Memory remains below the configured client buffer bound for the maximum supported transaction.

### SWF-006 — Expose native materialized shapes

**Priority:** blocker for native release. **Depends on:** `SWF-002`–`SWF-005`.
**Suggested owner:** Swift API agent.

Deliverables:

- Provide a value-typed shape definition and subscription API over absolute `upsert`/`delete` events.
- Define initial snapshot/caught-up readiness, current-state snapshot semantics, decoder failures,
  reset/replacement, and close.
- Require all PK columns in projections and use opaque key identity independent of decoded row fields.

Acceptance:

- Real-stack tests cover insert, update, predicate move-in/out, delete, NULL, composite PK, projection,
  schema retirement, large values, restart, slow consumer, and shared identical shapes.
- The public API never exposes Electric tags/DNF, tRPC types, raw DS admin operations, or anonymous
  release.

### SWF-007 — Add an optional transactional sink with ownership semantics

**Priority:** required for app migration if materializing locally. **Depends on:** `SWF-006`, `CMP-001`.
**Suggested owner:** Swift data-layer agent.

Deliverables:

- Define a narrow application-supplied transaction protocol for apply upserts/deletes, metadata/
  offset, reset generation, and commit acknowledgement. Do not depend on GRDB/SwiftData/Core Data.
- If multiple shapes share a destination table, store membership/ownership by `(feedID, opaqueKey)` or
  a proven equivalent so one shape's delete cannot remove a row still owned by another.
- Define merge/conflict rules when two feeds project different columns or versions of one row.

Acceptance:

- Reference in-memory sink plus app-database integration tests atomically apply data and resume token.
- Overlapping-shape, close, move-out, reset, crash-before/after-commit, and tenant-switch cases preserve
  exactly the specified ownership.

### SWF-008 — Add live aggregates

**Priority:** required if app inventory needs aggregates. **Depends on:** `SWF-004`, `SWF-005`.
**Suggested owner:** Swift aggregate agent.

Deliverables:

- Add count/sum/avg/min/max definitions with column validation, scalar precision, `{value,n}` state,
  caught-up/reset/replacement, lease, and close behavior.
- Represent integer sums without loss and define floating/decimal/null behavior from the server
  contract.

Acceptance:

- Golden and real-stack tests cover empty set, NULLs, insert/update/delete/retraction, bigint beyond
  2^53, group cardinality limits, restart, and replacement.

### SWF-009 — Add visibility-correct live subsets

**Priority:** gated feature. **Depends on:** `ENG-001`, `SWF-002`–`SWF-007`.
**Suggested owner:** Swift subset agent.

Deliverables:

- Implement one-shot page plus base-predicate changes feed using the versioned snapshot visibility
  fence, durable position, tombstones, and guarded seed.
- Implement keyset pagination with PK tie-break, PostgreSQL NULL ordering, C/unicode-scalar text order,
  opaque page cursor, and replacement reload.
- Reject unsupported multi-order, predicate, or page-size requests before network work.

Acceptance:

- Share the `ENG-001` randomized 100,000-interleaving corpus with TypeScript and Swift; both equal the
  SQL oracle.
- Cover delayed page, live delete/update/reorder, load-more overlap, NULL cursor, Unicode scalar order,
  feed lapse, crash/resume, and buffer overflow.

### SWF-010 — Handle app lifecycle and network transitions

**Priority:** blocker for iOS production. **Depends on:** `SWF-004`–`SWF-006`.
**Suggested owner:** Swift lifecycle agent.

Deliverables:

- Define foreground activation, suspension, finite background checkpoint opportunity, offline retry,
  credential refresh, logout/tenant switch, process termination, and relaunch.
- Treat lease expiry and v1/native replacement as routine reset paths; do not assume timers or sockets
  run while suspended.
- Integrate with the host app through injected lifecycle/background hooks, not global notification
  ownership hidden inside the package.

Acceptance:

- Simulator/device tests suspend/resume or terminate at every lifecycle state and network transition;
  relaunch converges without leaked task/claim or cross-session data.
- Background expiration cancels finite work promptly and leaves a recoverable persisted state.

### SWF-011 — Add client observability without data leakage

**Priority:** required. **Depends on:** `SWF-003`–`SWF-010`. **Suggested owner:** Swift observability agent.

Deliverables:

- Add opt-in structured events/metrics for lifecycle state, caught-up latency, reconnect reason,
  replacement/reset, buffer use/overflow, renewal, sink apply, decode error, and close result.
- Hash or application-alias table/template/subscription identifiers; never emit row values, predicates,
  authorization, signed URLs, raw query parameters, or tenant identifiers by default.

Acceptance:

- Snapshot tests inspect every emitted diagnostic under success/error paths for forbidden data.
- App telemetry can correlate gateway request ID with an anonymous client operation and diagnose every
  recovery branch.

### SWF-012 — Security and dependency audit

**Priority:** blocker. **Depends on:** `SWF-003`, `SWF-007`, `SWF-010`. **Suggested owner:** Swift security agent.

Deliverables:

- Threat-model credential storage/refresh, signed stream access, redirects, TLS, cache protection,
  backups, logs, jailbroken/debug builds, and tenant switch.
- Audit `Sendable`, actor isolation, continuations, task ownership, `@unchecked Sendable`, and any
  detached task.
- Produce privacy manifest/API usage evidence if required by the final implementation.

Acceptance:

- Static checks and focused tests find no credential in URL persistence/logs, no cross-tenant cache,
  no unchecked concurrency escape without documented proof/test, and no runtime dependency outside
  the approved manifest.

### SWF-013 — Package, document, and qualify the native release

**Priority:** blocker for native release. **Depends on:** `SWF-001`–`SWF-012`, `TST-002`–`TST-006`.
**Suggested owner:** Swift release agent.

Deliverables:

- Publish semver package/tag, API docs, getting started, gateway setup, supported matrix, examples,
  migration/rollback guide, error/recovery guide, and changelog.
- Generate symbol graph/API diff and protocol-compatibility report in CI.
- Qualify against pinned current/previous supported server versions and iOS/macOS CI.

Acceptance:

- A clean sample app authenticates, opens a shape/aggregate, materializes through the sample sink,
  recovers from a forced retirement/restart, and closes with no leaked claim.
- The release workflow rejects breaking API/protocol or missing documentation/evidence.

## 14. Cross-system verification

### TST-001 — Make all existing correctness gates release-blocking

**Priority:** blocker. **Depends on:** none. **Suggested owner:** CI agent.

Deliverables:

- Run `cargo fmt --check`, `pnpm typecheck`, `pnpm engine:test`, full Vitest/conformance/fuzz, and
  Electric's separate oracle/property/integration suites on every release candidate.
- Pin Rust, Node, pnpm, Elixir/Erlang, Postgres, DS, and Electric fixtures/images.
- Preserve failure seed/log/database artifacts and make every randomized failure replayable.
- Track the two tag assertions as an explicit compatibility deviation until fixed or declared
  unsupported; do not hide them in a green aggregate status.

Acceptance:

- A deliberately broken fixture in each suite blocks promotion and uploads actionable artifacts.
- CI can reproduce the local command matrix from a clean checkout without ambient sibling state.

### TST-002 — Share one protocol conformance corpus across Rust, TypeScript, and Swift

**Priority:** blocker. **Depends on:** `PROTO-001`–`PROTO-004`, `SWF-002`.
**Suggested owner:** conformance agent.

Deliverables:

- Generate valid/invalid fixtures for requests, handles, values, envelopes, errors, headers, pages,
  leases, replacement, transaction boundaries, and subset fences.
- Run encoder/decoder and live mock-server conformance in all three languages.
- Include unknown additive fields and previous supported protocol versions.

Acceptance:

- The same fixture identifiers have identical semantic outcomes in all implementations.
- CI fails if a contract file changes without regenerated fixtures and compatibility approval.

### TST-003 — Build deterministic crash/failure cut-point suites

**Priority:** blocker. **Depends on:** engine/OPS durability tasks. **Suggested owner:** fault-injection agent.

Deliverables:

- Add named cut points around catalog create/join/left/drop/retired, stream ensure/append/close/delete,
  source transaction chunks, checkpoint/highwater, segment rotation/deletion, snapshot gate, schema
  retirement, epoch reset, backup/restore, leader fence, and migration.
- Execute 100 failures at every cut point with seeded writes/readers and automatic oracle comparison.

Acceptance:

- On restart, each run restores exactly the last acknowledged contract: no shape is acknowledged then
  forgotten, no committed change is advanced past without landing/retirement, and no ID is reused.
- Every non-convergent seed is replayable by one documented command.

### TST-004 — Prove security and tenant isolation end to end

**Priority:** blocker. **Depends on:** `SEC-002`–`SEC-007`. **Suggested owner:** security test agent.

Deliverables:

- Create at least two tenants, roles, revoked users, templates, overlapping row identifiers, shapes,
  subsets, aggregates, and streams.
- Attack authentication, predicate/parameter substitution, projections, subqueries, subscriptions,
  stream paths/capabilities, HTTP verbs, redirects, CORS, admin/debug routes, quotas, logs, and direct
  private services.
- Add dependency/container/IaC scanning and a targeted manual protocol review checklist.

Acceptance:

- Zero unauthorized row/metadata/control access in the automated corpus; forbidden attempts make no
  catalog/stream/PG mutation.
- Logs/traces/metrics/artifacts contain no secret, signed URL, raw row, or unapproved tenant label.

### TST-005 — Extend oracle coverage to unresolved engine gaps

**Priority:** blocker. **Depends on:** `ENG-001`–`ENG-006`. **Suggested owner:** conformance/fuzz agent.

Deliverables:

- Add the subset visibility race, replayed TRUNCATE, dynamic table create/re-create, counts-tier drift
  restart, column-list publication refusal, non-logical WAL, blackholed replication connect, and
  no-SQLSTATE permanent error lanes.
- Extend fuzzing across NULL/negation/subquery flips, composite keys, large transactions, segment
  boundaries, and engine restarts.

Acceptance:

- Run at least 10,000 seeds per new pure-property family and 100 deterministic external failure runs;
  all results equal the oracle or the documented typed retirement/reset behavior.

### TST-006 — Qualify client lifecycle under adversarial scheduling

**Priority:** blocker. **Depends on:** `CMP-003`, `SWF-004`–`SWF-010`.
**Suggested owner:** Swift testing agent.

Deliverables:

- Use injected clocks/transports/sinks and a model-based reference state machine for create, tail,
  renew, replace, reset, sink commit, suspend, credential refresh, and close.
- Schedule completion/cancellation at every await boundary and duplicate/reorder only outcomes the
  network can actually produce.
- Add simulator/device tests for suspension/background expiration/network transitions and real-stack
  tests for engine/DS/gateway failures.

Acceptance:

- 10,000 model-generated lifecycle traces per seed class have no leaked task/claim, stale generation
  write, double close, dropped effect, or forbidden retry.
- Swift concurrency sanitizer/race diagnostics (where available) and strict-concurrency build are clean.

## 15. Migration, cutover, and rollback

### MIG-001 — Build a canonical differential shadow harness

**Priority:** blocker. **Depends on:** `CMP-001`, `TST-001`. **Suggested owner:** migration tooling agent.

Deliverables:

- Feed the same deterministic Postgres transactions to current Electric and Circuits, and optionally
  to the native Swift mock/real client.
- Normalize only documented representational differences: value typing, opaque key form, control
  messages, and absence of tags where the selected app semantics do not require them.
- Compare row membership, projected values, ordering/window, aggregate value/count, reset boundary,
  and caught-up point per query template.

Acceptance:

- The harness reports the first causal divergence with transaction, LSN/xid/seq, offsets, handles,
  template, and both materialized states; seeds replay in one command.
- A deliberately injected membership/value/delete divergence is detected.

### MIG-002 — Define isolated cache ownership and data migration

**Priority:** blocker. **Depends on:** `CMP-004`, `SWF-007`. **Suggested owner:** app data-migration agent.

Deliverables:

- Map every old cache/table/metadata key to its compatibility/native generation and owning feed(s).
- Choose shadow tables, generation columns, or `(feedID,key)` membership tables for overlapping data.
- Add an atomic promote/discard operation; never translate old Electric cursor/tag state into native
  Circuits state.

Acceptance:

- Upgrade, interrupted upgrade, downgrade, logout, and tenant-switch fixtures preserve one coherent
  visible generation and the rollback copy.
- Deleting/closing one feed cannot remove a row still owned by another selected feed.

### MIG-003 — Add per-template cutover and kill switches

**Priority:** blocker. **Depends on:** `MIG-001`, `MIG-002`, `CMP-005`, native tasks used by the app.
**Suggested owner:** app integration agent.

Deliverables:

- Select old Electric, Circuits `/v1`, or native Circuits independently for every inventory template.
- Keep writes on the existing application API; sync remains read-side only.
- Add bounded resubscribe jitter/admission, reset UX, telemetry, and a server/client kill switch that
  does not require an app binary release.

Acceptance:

- An automated scenario cuts over and rolls back each template during continuous writes, offline/
  online transition, app restart, and server retirement; visible state stays coherent.
- A kill switch stops new Circuits subscriptions, closes/relinquishes existing claims best-effort, and
  restores the prior owner/cache without cursor reuse.

### MIG-004 — Execute operation-count-based launch qualification

**Priority:** blocker. **Depends on:** gates G0–G9, `MIG-001`–`MIG-003`.
**Suggested owner:** release qualification agent.

Deliverables:

- Run every production query template through 1,000,000 shadow mutations, 10,000 client lifecycle
  cycles, 100 engine/DS/gateway restarts, all supported schema migrations, one backup/restore, one
  active/passive promotion/rollback, and the declared capacity workload.
- Record canonical divergence, latency, reset/replacement, resource, quota, and recovery results in the
  release evidence bundle.

Acceptance:

- Zero unexplained final-state divergence and zero lost committed changes.
- Every expected reset is typed, counted, and followed by a complete rehydrate; every threshold in the
  support/capacity matrix passes.

### MIG-005 — Prove rollback and close the migration

**Priority:** blocker. **Depends on:** `MIG-004`. **Suggested owner:** migration/release agent.

Deliverables:

- Roll back from each selected Circuits mode to the prior Electric service/cache using only published
  automation and runbooks.
- Verify named native releases, lease expiry cleanup, no orphan local ownership, and no deletion of the
  rollback generation.
- Produce the final per-template decision: remain old, `/v1`, native shape, native aggregate, native
  subset, or redesigned.

Acceptance:

- Rollback meets the declared RPO/RTO and final state equals the SQL oracle.
- The release checklist contains no unowned query, stream, cache, alert, runbook, or unsupported
  dependency.

## 16. Suggested subagent execution waves

Tasks within a row can run in parallel after their dependencies. Give each subagent one task ID and
its principal file boundary; avoid multiple agents editing the same contract/spec file simultaneously.

| Wave | Parallel work packets | Integration checkpoint |
| --- | --- | --- |
| **0 — decisions/evidence** | `GOV-001`, `GOV-002`, `GOV-003`, `CMP-001`, `SEC-001`, baseline `TST-001` | Support matrix, ownership ADR, app query inventory, route/data map |
| **1 — contracts/foundations** | `PROTO-001`, `OPS-001`, `OPS-002`, `OPS-003`, `ENG-006`, `ENG-012`, `CMP-002` | Versioned contract draft and protected staging topology |
| **2 — correctness/security** | `PROTO-002`, `ENG-001`, `ENG-003`–`ENG-005`, `ENG-007`–`ENG-011`, `SEC-002`–`SEC-007`, `OPS-005` | Native contract RC, bounded engine, authenticated gateway |
| **3 — leadership/clients** | `ENG-002`, `ENG-013`, `PROTO-003/004`, `CMP-003/004`, `SWF-001`–`SWF-005`, `OPS-004/006/008` | Transactional stream fixtures, compatibility app build, native transport/lifecycle |
| **4 — product surfaces** | `CMP-005`, `SWF-006`–`SWF-012`, `CAP-001`, `TST-002`–`TST-006`, `OPS-007/008` | Shape/aggregate RC; subset only after `ENG-001`; complete failure/security corpus |
| **5 — evidence/cutover** | `CAP-002`–`CAP-004`, `MIG-001`–`MIG-003`, `GOV-004`, `SEC-008` | Reproducible capacity, shadow parity, signed candidate |
| **6 — qualification** | `OPS-009`, `SWF-013`, `MIG-004`, then `MIG-005` | All gates closed; release or explicit no-go |

Recommended integration ownership:

- one protocol maintainer owns contract schema/fixtures and serializes `PROTO-*` merges;
- one engine maintainer reviews all changes touching sequencer/catalog/epoch/retirement invariants;
- one security maintainer owns gateway policy and negative-test coverage;
- one Swift maintainer owns public API/concurrency review while task agents own isolated modules/tests;
- one release maintainer owns the evidence manifest and refuses incomplete gates.

## 17. Explicit product decisions embedded in this spec

These decisions prevent subagents from solving different products:

- **New native Swift library:** yes. It is a transport/materialization client, not an ORM or shared
  database.
- **Change `electric-sync-swift` core first:** no. Use its existing injection seam or add a separate
  opt-in provider only if the app lacks one.
- **Native Swift transport:** versioned REST/stream protocol through an authenticated gateway, not tRPC.
- **Direct DS from mobile:** no, unless reads use audited, expiring, path-bound capabilities and the DS
  origin is otherwise unreachable; proxying is the simpler first target.
- **Native subsets:** not production-supported until `ENG-001` proves an xid/snapshot visibility fence.
- **Source transaction atomic observers:** not promised until `ENG-002`/`PROTO-003` land end to end.
- **Shape semantics:** absolute upsert/delete. Electric tags/DNF ownership are not synthesized.
- **Keys:** opaque identity. Composite decoding is optional and contract-tested.
- **Storage:** application-owned via optional transactional sink; overlapping feeds require explicit
  ownership accounting.
- **HA:** single active with fencing and tested active/passive recovery; active/active is out of scope.
- **Upstream dependency:** launch does not wait for upstream adoption; the team either owns the fork or
  chooses not to ship it in `GOV-001`.

## 18. Current evidence and known status

The research supporting this spec is indexed in [notes/README.md](README.md). In particular:

- the fork's correctness/durability core is substantially stronger than public upstream, with
  `SnapshotGate`, exactly-once-effect highwater, transaction-end input markers, durable catalog,
  retirement completion, schema/epoch safety, segmented change log, spillable ingest, and leases;
- public upstream still has no tags/releases or stable package/API contract, and the local fork is a
  material unreleased divergence;
- the present blockers are chiefly public exposure/tenant authorization, direct DS access, deployment
  durability/TLS, bounded work, recovery packaging, protocol versioning, capacity evidence, the subset
  visibility seam, and client terminal-stream recovery;
- `electric-sync-swift` is a mature but broad Electric-specific replica framework (13,203 production
  LOC and 23,220 test LOC), so making its core dual-protocol would create more risk than a focused
  native package; and
- this document deliberately replaces calendar “monitor/soak for N days” gates with fixed workloads,
  repetitions, failure injections, resource-slope assertions, and oracle convergence.

This draft is not yet the final release plan. Independent differential reviewers must try to find
missing tasks, incorrect dependencies, unverifiable acceptance criteria, and contradictions; their
accepted findings will be incorporated into this file before it is marked reviewed.
