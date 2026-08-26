# ElectricCircuitsSwift production-readiness punchlist

Status: **active execution punchlist**, 2026-08-24.

This is the production-readiness contract for the reusable Swift library and its direct native Axum
integration. It is not a claim that the LinearLite example is production-ready, and it does not
require a public gateway, tenant registry, or production iOS application in this phase.

## Implementation status — 2026-08-24

This status records executable evidence, not completion by checklist wording alone.

- **P0-1 implemented and reviewed:** the package now publishes a versioned native-v1 prose and
  machine contract for every public control route it consumes, including aggregate creation, plus
  the separate durable-stream HEAD/GET contract. A dependency-free gate checks the live Rust
  `/v1/openapi.json`; the Swift corpus covers scalar fidelity, NULL versus missing fields, unknown
  members, malformed/truncated envelopes, terminal statuses, empty cursor advancement and escaped
  paths. The fresh PostgreSQL 18.4 top-10 run exercised the live gate before client use.
- **P0-2 implemented for the current provider API:** `ShapeMaterializer.apply` is a cursor CAS;
  rows and cursor commit atomically; stale writers, rollback, replay, reopen, migration failure, and
  empty cursor-advancing responses are covered by the in-memory and GRDB suites. Reviewed immutable
  admission limits now reject oversized successful response bodies before JSON decode and overfull
  event batches before provider apply, with typed bounded diagnostics and no row/cursor movement.
  The built-in URLSession transport enforces the byte limit while receiving successful bodies;
  terminal and retryable HTTP status semantics remain authoritative.
- **P0-3 implemented and reviewed:** stable named claims, create/renew/release retries, bounded
  jitter, cancellation joins, terminal/reset and replacement reseed outcomes, durable-cursor resume,
  and release compensation are covered. The final terminal-vs-renew regression uses causal test
  gates and passed 10/10 timeout-bounded repetitions.
- **P0-4 implemented and reviewed for recent-10 and simultaneous filtered windows:** scripted
  native-route tests and real PostgreSQL 18.4 qualifications cover feed-before-snapshot overlap,
  deterministic ordering, newest insert/eviction, outside-row promotion, in-window
  demotion/promotion, delete/promotion, replay, cancellation, and fresh subscription after a
  closed stream. A separate Ada/Bob assignee run proves that one principal and canonical GRDB
  table can serve two live predicates with isolated memberships, cursors and releases. Every real
  phase compares the public `LinearLiteSession` plus file-backed GRDB with independent predicate
  SQL ordered by `modified DESC, id DESC LIMIT 10`.
- **P0-5 implemented in the GRDB example/provider:** canonical rows are principal-partitioned;
  memberships/cursors/overlays are scope-isolated; logout/principal purge and migrations are tested.
- **P0-6 implemented at the provider/example seam:** UUIDv4 `client_id`, optimistic overlay
  persistence, insert reconciliation to a server ID, delete reconciliation, rejection, and sibling
  scope isolation are tested.
- **P0-7 implemented and independently reviewed for baseline + top-10:** the opt-in Layer-B harness boots a real
  PostgreSQL 18.4 cluster, logical replication, Rust durable-streams, native Axum, and the actual
  Swift control/stream client. It compares the atomically persisted Swift key map with an
  independently authored SQL oracle, replays one real three-event mutation batch from a saved
  pre-terminal cursor, verifies custom headers, binds evidence to runtime source/binary digests,
  bounds a stuck Swift child, and proves listener/PG-root cleanup. The separate top-10 run adds the
  actual recent-subset session, file-backed GRDB, four window-transition phases, provider reopen,
  named release, and exact 10-row membership. A separate reviewed failover run now covers the
  supported unsynchronized-slot epoch-boundary profile.
- **P0-8 and P1-6 initial profile implemented and reviewed:** Foundation-only bounded OTLP/HTTP export,
  write-only endpoint/auth configuration, W3C trace propagation, redaction/cardinality rules,
  sampling, exporter health, shutdown, backpressure, finite per-export timeout, and no-op behavior
  have deterministic tests. A live disposable collector verifies endpoint/auth arrival, parseable
  OTLP JSON, 401/403/429/500 containment, blackhole timeout, bounded drops, causal materialization
  and cursor advancement, and exact listener cleanup. Runtime endpoint/header rotation is explicitly
  outside the initial immutable-configuration profile.
- **P1-1 implemented and independently reviewed for the unsynchronized-slot profile:** a real
  PostgreSQL 18.4 physical standby catches the fenced baseline before primary isolation, is promoted
  with `sync_replication_slots = off`, and serves the engine again at the same Axum URL. Swift sees
  the old generation's typed terminal, keeps the complete old scope isolated, creates a distinct
  named claim/handle/generation, atomically switches visibility only after the new snapshot commits,
  and reopens file-backed GRDB exactly equal to the promoted-primary SQL oracle. Scenario and cleanup
  must both pass; the reviewer reran the full qualification from a byte-identical copy.
- **P1-2 implemented and reviewed for engine/stream/network/schema recovery:** a real PostgreSQL
  18.4 run covers engine restart and durable-stream outage/recovery, with an independent SQL oracle
  and durable cursor receipt after each phase. Package tests cover caller-owned 401/403 credential
  refresh, typed 408/425/429/5xx retryability, bounded `Retry-After`, transient recovery, retry
  cancellation, in-flight response cancellation and provider-apply failure. A real Foundation
  URLSession request to a reserved `.invalid` host produces a typed, redacted and retry-bounded DNS
  failure with no provider/cursor mutation and explicit session cleanup. Caller-owned path gates
  prove no retry churn while unavailable, same-claim resume from the durable cursor, and
  cancellation-safe exact-once waiter cleanup. The native 409 + `stream-closed` generation contract
  releases before publishing reseed, never auto-creates, stages the fresh isolated GRDB snapshot
  while the old scope remains visible, atomically switches selection, then purges the old scope.
  Nonretryable response bodies and provider diagnostics cannot enter public errors, state or
  telemetry. Full PostgreSQL schema-drift topology remains server qualification, not a missing Swift
  recovery seam.
- **P1-3 reusable lifecycle/privacy seams implemented and reviewed:** a natural terminal/failed
  state is published only after the old generation detaches its handle and completes its release
  attempt. Immediate restart is proven with a held-release causal gate and 10/10 repetitions;
  concurrent stop/new generation suppresses stale publication. Provider durable state is now
  preflighted before the first create POST, with coarse redacted protected-data/database
  availability states and no claim on failure. Causal GRDB tests cover joined background release,
  foreground durable-cursor resume and release → principal purge → new-account visibility with no
  same-ID leakage. The reusable Foundation lifecycle is now wired through the reference UIKit host:
  protected-data and scene notifications produce truthful joined start/stop receipts, account
  transitions serialize release → throwing purge → new start, failed releases retain retry
  ownership, and stale start generations cannot publish after an inactive fence. The exact reviewed
  host sources regenerate byte-identical Xcode project output and build unsigned for generic iOS and
  the newest available iOS 26.5 simulator.
- **P1-4 provider migrations implemented and reviewed:** the production GRDB migrator is exercised
  from every supported schema cut point through v8, including populated rows, memberships, cursors
  and overlays. The v8 rebuild preserves narrowly compatible caller-owned nullable/defaulted columns
  and ordinary indexes, while triggers, generated/constraint-bearing columns and unsafe index forms
  fail closed transactionally without damaging the prior database. WAL, foreign-key policy,
  concurrent snapshot readers and a shared in-memory/GRDB materializer contract are covered.
- **P1-5 resource bounds implemented and reviewed:** callers can opt into a
  shared Foundation-only hard cap across coordinators. Rejection is typed and occurs before provider
  preflight or a create POST; each admitted coordinator holds one opaque permit across create,
  streaming, retry and release, and a permit returns only after any landed claim is released. There
  is no generic work queue or scheduler. Deterministic 1/10/100/1,000 qualification records exact
  active, rejected, create, release and final-active counts; cancellation, joined callers,
  preflight failure, terminal reseed and failed-release ownership have causal tests. The previously
  scheduler-dependent epoch-reset assertion was replaced with named terminal-response,
  release-observed and public-state gates. The built-in URLSession transport now accumulates at most
  the configured successful-response limit in library-owned `Data`, observes at most `limit + 1`,
  cancels chunked/unknown-length overflow, preflights oversized known lengths, and maps overflow to
  typed redacted client/stream failures before provider apply or cursor movement. Foundation,
  URLSession and socket-owned buffers remain outside this library-owned allocation guarantee.
- **P1-7 package-quality baseline implemented and reviewed:** the package now has a warnings-as-errors
  DocC catalog for transport, storage/materialization, cursor/lifecycle/errors and OTLP/redaction;
  explicit Swift 6.0, iOS 16+ and macOS 13+ support plus source-package SemVer rules for public API,
  native wire and provider-schema evolution; and one reproducible local/CI gate for strict root and
  clean LinearLite builds/tests, formatting, dependency closure, DocC, XcodeGen project identity and
  an unsigned generic iOS host build. External checkout/tool inputs are SHA/checksum pinned. The
  integrated current tree passes the full gate at 117/12 and 92/8. Stable binary ABI/module
  compatibility is explicitly not promised.

Canonical Swift evidence after integration: strict format, complete-concurrency and
warnings-as-errors builds pass; the combined Foundation-only package passes **117 tests in 12 suites**,
LinearLite/GRDB passes **92 tests in 8 suites**, and the core dependency list is empty. The server
PG18 promotion case passes under its ordinary unprefixed command through the repository's PG18 tool
resolver. The canonical Swift
`Scripts/qualify-real-stack-pg18.sh` run also passes `SWF-P0-7-v2`: SQL and Swift converge on rows
`[(1, after), (3, created)]`, replay advances from the saved pre-terminal cursor to the final cursor
by applying one batch/three events, and all PostgreSQL/engine/durable-stream listeners are gone.
The reviewed `Scripts/qualify-real-top10-pg18.sh` evidence additionally proves exact recent-10
convergence through snapshot overlap and four boundary-changing live phases, with no surviving
owned listeners or PostgreSQL data root. The reviewed `Scripts/real-pg18-failover.ts` evidence
additionally records all 11 promotion/reseed phases, distinct `s1`/`s2` generations, two successful
named releases, exact old/new visible scopes, and complete listener/process/data-root cleanup. The
real OTLP qualification passes six live-collector tests and ten repeated focused runs under an
external deadline.
The reviewed filtered-window evidence additionally records exact Ada/Bob SQL equality at baseline,
A-only insert, two-window reassignment, B-only update, A release with continued B progress and
provider reopen. The reviewed engine/durable-stream outage evidence records exact SQL/Swift/cursor
equality after baseline and each outage recovery. The native transport matrix preserves typed
status and bounded retry guidance without retaining response bodies, credentials or provider text.

## Next merge/release milestones

The punchlist no longer identifies another speculative library-hardening slice. The next work should
turn the reviewed implementation into a consumable release:

1. Split the native Axum API and its reviewed qualification changes from the server repository's
   mixed working tree, then run the native-v1 contract corpus and PostgreSQL 18 real-stack
   qualification from that clean branch in CI.
2. Rotate the exposed machine credential/rewrite described below, then publish the clean, green
   Swift `main` as an initial SemVer `0.x` package release with the supported profile and limitations
   above.
3. Integrate that release into one real application. New implementation work should be driven by
   concrete integration failures or an explicit expansion of the supported profile.

### Merge-readiness snapshot

- `electric-circuits-swift` 0.1.0 is merged and published. PR
  [`indexedlabs/electric-circuits-swift#1`](https://github.com/indexedlabs/electric-circuits-swift/pull/1)
  merged reviewed head `227a3169b1f791180bcc5ea21065bd2435412b64` as exact `main` commit
  `176d296e3aabc061a7b73b6bb09166ed47ea94e6`; tag `0.1.0` dereferences to that commit and the
  [GitHub release](https://github.com/indexedlabs/electric-circuits-swift/releases/tag/0.1.0) is
  published.
- The package carries both `LICENSE-MIT` and `LICENSE-APACHE`, matching the server's dual-license
  policy.
- The complete local `Scripts/quality.sh` gate passes: 118 root tests (108 ordinary plus 10 isolated
  capacity tests) in 12 suites, 92 LinearLite/GRDB tests in 8 suites, strict builds, formatting,
  dependency closure, DocC, XcodeGen identity and the unsigned iOS host build.
- GitHub Actions PR run
  [`32816969536`](https://github.com/indexedlabs/electric-circuits-swift/actions/runs/32816969536)
  passed on Xcode 26.3 for exact head `227a316`; the post-merge `main` run
  [`32817527557`](https://github.com/indexedlabs/electric-circuits-swift/actions/runs/32817527557)
  passed for exact release commit `176d296` before the tag was published.
- The local Swift remote is stored without credentials, but the machine's global Git URL rewrite
  injects an embedded token. Rotate that token and replace the rewrite with a credential helper or
  other non-URL authentication before any push.
- The server's native-Axum branch already contains the core API commit; its remaining dirty work
  spans reviewed server qualification, PostgreSQL 18 support and documentation. Split that work by
  behavior/evidence before opening the final server PR rather than adding another feature wave.

### Active release execution (2026-08-24)

- Server candidate: branch `codex/native-api-pg18-release`, isolated worktree
  `/private/tmp/electric-circuits-native-api-pg18-release`, pinned to native-API base `e53cf40`.
  Its implementation pass may import only reviewed PostgreSQL 18/native-contract qualification
  changes from the mixed primary checkout; lifecycle/catalog/retention work is out of scope.
- Swift candidate: branch `codex/swift-0.1.0-release`, isolated worktree
  `/private/tmp/electric-circuits-swift-0.1.0-release`, pinned to green base `6d92806`. Its release
  pass must prove that the actual LinearLite iOS host resolves and builds against an exact `0.1.0`
  source-control tag in an isolated local release fixture before any remote tag is published.
- Both candidates use the lightweight freeze/review protocol: implementer writes `DONE.md` last;
  an independent reviewer works in the same worktree and writes `REVIEWED.md`; a failed review
  removes the stale marker before the next implementation pass.
- The unsafe global tokenized Git URL rewrite was removed and GitHub CLI was configured as the
  credential helper. Remote token revocation/rotation remains user-owned because local cleanup
  cannot invalidate a credential already exposed outside Git configuration.

### Release execution update (2026-08-25)

- The first Swift PR CI attempt (`32810770823`) timed out after the ordinary 108-test lane passed:
  the capacity fixture counted a stream request before its checked continuation was installed, so
  cancellation in that interval was lost and `swiftpm-testing` stayed alive. TDD added a latched
  cancellation regression and a 120-second isolated capacity supervisor.
- Two independent reviews then found real process-ownership holes in the first supervisor drafts:
  external TERM/INT/HUP could orphan the `setsid()` child, and cancellation could arrive between
  `fork` and group readiness. The final protocol masks cancellation signals before `fork`, uses
  child-to-parent `setsid` readiness plus parent-to-child execution authorization, targets only the
  exact direct PID before readiness and exact negative PGID afterwards, escalates TERM to KILL on a
  bounded deadline, and reaps the direct child. Nine deterministic supervisor cases and unchanged
  1/10/100/1,000 capacity receipts passed independent review.
- Server PR [`mwildehahn/electric-circuits#1`](https://github.com/mwildehahn/electric-circuits/pull/1)
  ultimately passed at reviewed head `5b5bca8d79506dc9cad1f192748c5ce0edb6b288`: clean Ubuntu CI
  [`32820013740`](https://github.com/mwildehahn/electric-circuits/actions/runs/32820013740) ran
  rustfmt, 389 engine tests, PostgreSQL 18 setup/resolution, typecheck, Node 7/7 and Vitest 58/58
  files with 293/293 tests; Docker run
  [`32820013663`](https://github.com/mwildehahn/electric-circuits/actions/runs/32820013663) built all
  three images. It merged as `04c026da5d5636bddfffd2523a728004224ae0d5`.
- That merge's first main CI run
  [`32821421233`](https://github.com/mwildehahn/electric-circuits/actions/runs/32821421233) exposed a
  real nondeterministic FakeDs test-harness race: every other Rust test finished, but
  `internal_purge_waits_for_stream_retirement_before_returning` lost a `notify_waiters` release
  between DELETE arrival and waiter registration, then held the job until its 30-minute cap. The
  correction was not a timeout increase: per-seam release generations now release only requests
  that observed the prior generation, retaining an early release while keeping a queued retirement
  retry blocked for its own later release. Two implementation/review retries caught and removed an
  initially weakened multi-delete assertion and then a test-side re-arm race.
- Follow-up PR [`mwildehahn/electric-circuits#2`](https://github.com/mwildehahn/electric-circuits/pull/2)
  passed independent review at `657bb7d8ea98528808f6ddad7eb6a88f7d6d344d`. Both race contracts
  passed 100 repetitions each; purge tests were 10/10; engine tests were 389; Node was 7/7; and the
  local full gate was 58/58 files and 293/293 tests. Clean PR CI
  [`32827483543`](https://github.com/mwildehahn/electric-circuits/actions/runs/32827483543) and Docker
  [`32827483575`](https://github.com/mwildehahn/electric-circuits/actions/runs/32827483575) passed.
  It merged as final server `main` commit `e33164bf414fa2abbf7ac2284dda1da1247fbdc4`; post-merge CI
  [`32829004051`](https://github.com/mwildehahn/electric-circuits/actions/runs/32829004051) and Docker
  [`32829004174`](https://github.com/mwildehahn/electric-circuits/actions/runs/32829004174) both passed,
  including the formerly hanging Rust gate and the full conformance suite.
- The external Elixir Electric oracle was not rerun for this candidate because `mix`/`elixir` are
  absent on the host; vendored oracle input hashes and the direct adapter cleanup contracts pass.
  This limitation remains explicit rather than being represented as green evidence.

## Scope and first profile

The first supported library profile is deliberately narrow:

- `ElectricCircuitsSwift` talks directly to the native Electric Circuits Axum HTTP API.
- Authentication is supplied by an injected `HTTPTransport`/`URLSession` (cookies, headers, or a
  caller-owned credential refresh policy). The library does not own account auth.
- The core package depends only on Foundation and its public storage/materialization protocols.
- GRDB is an example/provider integration, not a core dependency.
- The initial client profile supports ordinary shapes and bounded subset snapshot + live-feed
  materialization. Aggregates, cross-stream transaction atomicity, and arbitrary SQL are separate
  profiles.
- PostgreSQL primary/replica failover is a server epoch/re-hydration event. The client must recover
  without mixing generations; seamless failover is not promised by the library.
- Observability is injectable and OTLP-compatible. An application can configure an OTLP endpoint URL
  and authorization headers without making OpenTelemetry SDKs a mandatory core dependency.

## P0 — required before calling the library deployable

### P0-1. Freeze the native HTTP contract

- Version and document the direct Axum endpoints used by the package: shape/subset creation,
  subset snapshot, stream cursor, stream reads, release, and server error responses.
- Define stable status semantics: retryable 5xx/429, auth 401/403, terminal 404/410/closed streams,
  malformed 4xx, and epoch/generation reset errors.
- Define the snapshot/live fence precisely: the snapshot rows and the stream cursor must describe
  one source frontier, and a cursor is acknowledged only after the provider commits the associated
  rows.
- Define whether a reset returns a new epoch/generation in the response or requires the client to
  recreate the subscription. The Swift API must expose enough information to prevent old/new feed
  mixing.
- Add a protocol corpus for Int64, Decimal, NULL, missing fields, empty batches, deletes, malformed
  envelopes, unknown fields, and URL/path escaping.

### P0-2. Make materialization and checkpointing crash-safe

- Require every provider to commit row changes and the stream cursor in one durable transaction.
- Prove that a provider failure leaves both rows and cursor at the previous committed state.
- Prove that a process crash after commit and before acknowledgement is safe to replay.
- Prove duplicate batches and duplicate envelopes are idempotent at the provider boundary.
- Define behavior for an empty response that advances the stream cursor.
- Define bounded transaction/batch limits and failure behavior when a batch exceeds provider memory.

### P0-3. Finish subscription lifecycle semantics

- Start, stop, cancel, and restart are idempotent.
- A named subscription claim is reused for renewal; release is safe to retry.
- A cancelled create/feed task cannot leave a leaked server claim or a partially installed local
  materializer.
- A terminal stream closes the reader and transitions to a typed recoverable state; it must not spin
  or silently discard the cursor.
- Reconnect resumes from the last durably committed cursor, not an in-memory cursor.
- Backoff has bounded jitter, honors cancellation, and distinguishes terminal from transient errors.
- The library exposes enough state for an app to show connecting, streaming, retrying, terminal, and
  failed states without parsing error strings.

### P0-4. Prove subset snapshot + live-window correctness

For an ordered, limited view such as “most recent 10 issues,” add end-to-end tests for:

- initial snapshot has exactly the requested limit and deterministic tie-breaking;
- insert of a newest row enters and evicts the old boundary row;
- update of an outside row moves it into the window;
- update of an in-window row moves it out and promotes the next row;
- deletion of an in-window row promotes the next row;
- updates that do not affect predicate/order do not cause unnecessary reseeds;
- filtered windows (assignee/project/status) do not leak rows across scopes;
- a snapshot response and overlapping live events converge exactly once;
- repeated reseeds do not advance beyond an uncommitted cursor;
- a server-side feed reset causes a fresh snapshot/live fence rather than mixing generations.

### P0-5. Define storage topology and scope isolation

- Store subscription metadata, cursor, epoch/generation, and materialized rows under an explicit
  scope key (principal + template/query identity + subscription/generation).
- A shared canonical table may back multiple views, but each view's cursor and membership/window
  state must be isolated.
- Closing one view must not delete rows or cursors still owned by another view.
- Account switch/logout must invalidate old scopes before exposing the new account's rows.
- The core must not assume one table, one subscription, or one database implementation.

### P0-6. Optimistic write reconciliation

- Client-generated UUIDv4 `client_id` is persisted in the pending write and authoritative row.
- A feed row retires the overlay only when its client identity matches; numeric server IDs are not
  sufficient for matching optimistic inserts.
- A rejected/failed write rolls back or marks the overlay failed without corrupting the authoritative
  row.
- Retried writes do not create duplicate overlays.
- Server-side updates, deletes, and conflicts arriving before acknowledgement have deterministic
  precedence and tests.
- This remains an application/provider seam; the core should expose identity and event ordering but
  not prescribe a LinearLite schema.

### P0-7. Real-stack integration harness

Build a disposable test stack:

```text
PostgreSQL 18 → logical replication → Electric Circuits Axum/engine → native HTTP → Swift client → store
```

The harness must use an independent SQL oracle and causal markers, not only internal engine state.
Every scenario records the source commit, server cursor/generation, client materialization commit,
and final SQL equality.

### P0-8. Telemetry and trace context contract

- Expose a telemetry configuration with an OTLP endpoint URL, authorization headers, service name,
  deployment/environment attributes, and sampling policy. Header values are write-only configuration
  and are never included in logs, diagnostics, or span attributes.
- Support injecting `traceparent`/correlation context into native HTTP requests while preserving the
  caller's existing auth and cookie headers.
- Emit spans for subscription creation/renewal/release, snapshot fencing, stream poll, reconnect,
  retry/backoff, terminal/reset handling, provider materialization, cursor commit, and reseed.
- Emit metrics for active subscriptions, stream lag, poll duration, bytes/events per batch, snapshot
  duration, materialization transaction duration, retries, reconnects, reseeds, terminal resets,
  provider failures, and telemetry export failures/dropped records.
- Use bounded-cardinality attributes: template/query IDs and hashed scope identifiers are allowed;
  raw subscription IDs, predicates, primary keys, row values, cookies, bearer tokens, and signed URLs
  are not.
- Telemetry export is best-effort and must never delay, fail, reorder, or roll back a data or cursor
  transaction. Exporter outage, timeout, 401/403, 429, and malformed collector responses must be
  observable locally while sync continues.
- Keep the exporter queue bounded and define behavior under memory pressure, app suspension, offline
  operation, and process termination. A diagnostic metric may count dropped telemetry, but it must
  not become an unbounded local cache.
- Support disabling telemetry without changing sync behavior, and support deterministic test sinks so
  package tests do not require a collector.

## P1 — required for broad library adoption

### P1-1. Primary outage and replica promotion

Use a real PostgreSQL 18 primary/standby fixture with WAL/physical replication:

1. Subscribe and materialize a known baseline.
2. Commit marker rows before the outage and wait for the client receipt.
3. Stop or isolate the primary.
4. Promote the standby and redirect the server.
5. Exercise both supported outcomes:
   - the old logical slot is unavailable/unsafe, so the server reports an epoch break and the client
     receives a typed terminal/reset condition;
   - a separately qualified synchronized failover-slot profile is usable, if that profile is ever
     enabled.
6. Recreate/reseed the subscription and verify the new-primary SQL state, with no mixed generations,
   duplicates, or missing rows.

The Swift library does not implement PostgreSQL failover. It must correctly stop the old reader,
discard or quarantine the old generation, recreate the subscription, and resume from the new fence.

### P1-2. Server and transport fault matrix

Test direct native HTTP behavior for:

- engine restart while the client is streaming;
- durable-stream outage and recovery;
- connection timeout, DNS failure, offline mode, and network path change;
- 401/403 credential expiry and caller-supplied credential refresh;
- 429/5xx with retry-after and bounded backoff;
- stream 404, 410, and `stream-closed` terminal responses;
- response cancellation during snapshot, page, and event-batch application;
- malformed JSON, truncated body, missing cursor, and unknown envelope fields;
- server schema drift or explicit unsupported-schema response.

### P1-3. Mobile lifecycle and privacy

- background suspension and foreground restart;
- task cancellation from scene/account teardown;
- protected-data-unavailable at launch and becoming available later;
- process termination during network response, provider transaction, and cursor commit;
- logout/account switch purges private materialized data before new-account reads;
- local database unavailable, read-only, corrupt, full, or migration-failing;
- no credentials or private row data appear in logs, diagnostics, or crash payloads.

### P1-4. Migrations and provider compatibility

- Versioned provider schema migration from an empty database and every supported prior version.
- Migration failure leaves the prior database recoverable and refuses to start sync against an
  ambiguous schema.
- A provider can add indexes/columns without resetting the server cursor incorrectly.
- GRDB tests cover transaction behavior, WAL mode, foreign keys, indexes, and concurrent readers.
- An in-memory provider runs the same materializer contract suite as GRDB.

### P1-5. Resource and observability contract

- Bound decoded response size, pending batch size, retry count, and concurrent subscriptions.
- Measure feed lag, batch sizes, reconnects, retries, reseeds, terminal resets, provider commit
  duration, and last committed cursor.
- Expose redacted diagnostics suitable for support without exposing cookies or row payloads.
- Add performance tests for 1/10/100/1,000 subscriptions, large rows, large batches, and long idle
  streams.
- Define battery/network policy for polling timeout, backoff, and foreground/background transitions.

### P1-6. OTLP collector qualification

- Run a local disposable OTLP/HTTP collector in integration tests and verify configured endpoint and
  authorization headers arrive exactly as configured.
- Verify trace spans carry a stable sync operation ID, source cursor/generation, and outcome without
  exposing row data or credentials.
- Verify retry attempts, server resets, provider failures, and account-scope transitions produce the
  expected span/metric relationships.
- Stop or blackhole the collector during active sync and prove rows/cursors still converge, with a
  bounded dropped-export count.
- Exercise collector 401/403/429/5xx responses, endpoint changes, and auth-header rotation without
  restarting the materializer.
- Test telemetry disabled, sampling zero, and sampling one as separate configurations; the data
  path must remain identical.

### P1-7. Package quality and compatibility

- Strict Swift concurrency and `Sendable` checking with no unchecked data-race escape hatches.
- Supported iOS/Swift version matrix and CI builds for package, example provider, and host app.
- DocC/API documentation for transport, store, materializer, cursor, lifecycle, and error semantics.
- DocC/API documentation for telemetry configuration, OTLP headers, span names, metric names,
  redaction rules, sampling, and exporter-failure behavior.
- Semantic versioning policy for protocol, storage, and public API changes.
- No GRDB, tRPC, durable-streams, or app-specific model dependency in the core target.

## Integration test suite layout

### Layer A — package contract tests (fast)

Pure Swift Testing tests for JSON/scalar/key codecs, cursor rules, retry classification, stream
state machines, overlay identity, subset window transitions, and provider transaction contracts.

### Layer B — direct native server integration (required)

Run against real PostgreSQL 18 and real engine/stream processes. Use an independent SQL oracle and
test the full native HTTP path. These tests own snapshot/live fencing, reconnect, restart, reset,
filtering, and failover behavior.

### Layer C — provider integration

Run the same event corpus through the in-memory provider and GRDB provider. Compare rows, overlays,
cursor, scope metadata, and error behavior after every committed batch.

### Layer D — app-host lifecycle tests

Use the host app to exercise background/foreground, cancellation, protected data, account switching,
and UI-observable state. Do not use UI tests to prove row-level correctness already covered by A–C.

### Layer E — device/network qualification

Run bounded scenarios on a simulator and at least one physical device with offline transitions,
network changes, process termination, storage pressure, and background suspension. This validates
OS behavior; it is not a substitute for the real-stack oracle tests.

## Explicitly out of scope for this first library profile

- Public gateway, tenant registry, or multi-gateway authorization.
- Seamless PostgreSQL failover implementation in Swift.
- Arbitrary SQL or DNF compatibility with `electric-sync-swift`.
- Cross-stream transaction atomicity.
- Aggregates and generic replica-sink semantics.
- CDN/direct durable-stream capabilities.
- GRDB as a mandatory core dependency.

## Suggested implementation order

1. Freeze the native HTTP/cursor/epoch contract and add the Layer A state-machine tests.
2. Add the Layer B PG18 baseline, subset-window, restart, reconnect, and reset tests red-first.
3. Finish the provider transaction/cursor contract and run the corpus against in-memory and GRDB.
4. Add optimistic identity/retry/conflict tests.
5. Add mobile lifecycle/privacy and migration tests.
6. Add primary/standby promotion and resource/network qualification.
7. Publish the package API documentation and a versioned release candidate.
