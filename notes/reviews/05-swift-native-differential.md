# Swift-native differential review

Date: 2026-08-22

Scope: independent review of the native Swift lane in
`notes/16-production-readiness-and-swift-migration-spec.md`, checked against `AGENTS.md`, notes 07,
08, 09, and 13, the current TypeScript/protocol/engine sources, and the sibling
`../electric-sync-swift` package. This review changes neither implementation nor execution spec.

## Verdict

The recommended separate Swift package is the right architectural direction, but the work packets
are not safe to execute as written. The proposed public value API contradicts the launch-blocking
server-owned-template policy; durable acknowledgement is assigned to an `AsyncSequence` that cannot
provide it; lifecycle state is required to persist before any persistence protocol exists; subset
pages omit the opaque keys needed to join them to live events; and the dependency graph both cycles
and makes explicitly gated features mandatory for the first native release.

The current TypeScript client is useful behavioral evidence, not a protocol oracle. It has the
correct named-lease, renewal-drain, generation, and absolute-upsert/delete patterns, but it also
contains gaps a native client must not copy: page rows are keyed with `String(row[pk])`, renewal only
rebinds when shape ID/path changes, final release failure is swallowed, and consumer delivery is not
a durable acknowledgement. The existing Swift package likewise demonstrates valuable injection and
session seams, but its default-buffered `AsyncThrowingStream`, unbounded SSE parser state,
`Task.detached` lifecycle, and `Any?` transaction handle are explicitly unsuitable foundations for
the new package.

## Severity-ranked findings and exact task changes

### P0 — The proposed Swift API bypasses the production authorization model

`SEC-003` requires named, versioned, server-owned templates and says never to trust arbitrary client
SQL/AST. In contrast, `SWF-002` and `SWF-006` require public table, predicate, projection, and
aggregate definitions, and the API sketch in note 09 exposes `ShapeDefinition`, `Predicate`,
`SubsetDefinition`, and `AggregateDefinition`. A mobile caller that can choose these fields either
violates G2 or presents a false API whose values the gateway silently ignores. It also leaves Swift
without authoritative result schema: PK columns, scalar types, nullability, projection presence,
schema generation, and codec version are currently app-supplied assumptions.

**Required spec edits:**

- Amend `PROTO-001` so every production create/query request is
  `(templateID, templateVersion, typed parameters)`. Its response must include an immutable
  `ResultSchema` containing canonical table alias, ordered fields, wire scalar kinds, nullability,
  field-presence policy, ordered PK fields, key-codec version, schema fingerprint/generation, and
  template version.
- Amend `SWF-002`, `SWF-006`, `SWF-008`, and gated `SWF-009` to expose `TemplateID` plus generated or
  schema-validated parameter/result descriptors. Move raw `TableRef`/predicate/projection definitions
  to an explicitly insecure self-hosted/admin SPI in a separate product which is excluded from G2 and
  never enabled in the production sample.
- Make `SEC-003`, not the Swift package, own the mapping from template parameters and principal to
  table/predicate/projection/aggregate. Add the result-schema response to the gateway contract rather
  than compiling app schema into the networking library.

**Fixed, executable acceptance:** a checked-in manifest containing exactly 32 template fixtures is
run by Rust, TypeScript, and Swift. For each fixture, execute 16 valid parameter boundary cases and
128 single-field adversarial mutations (4,096 negative requests total); every negative request must
produce zero engine/DS operations. A compile-time API test must contain exactly 8 forbidden snippets
(raw table, raw predicate, raw columns, raw tenant, raw SQL, raw subquery, raw aggregate column, raw
DS path) and require all 8 to fail outside the insecure SPI.

### P0 — Persistence and delivery acknowledgement have no implementable owner

`SWF-004` says the actor owns a durable offset and persists the subscription before tailing, but no
control-state storage abstraction exists until optional `SWF-007`. `SWF-005` then advances an offset
after “sink/consumer acknowledgement,” while `SWF-006` exposes observation without an acknowledgement
operation. Yielding a value from `AsyncSequence` only transfers it to a caller; it does not prove the
caller applied it, much less committed it atomically with the cursor. Persist-before-apply loses an
effect after crash, and persist-after-yield replays arbitrary caller side effects.

**Required new task:** add `SWF-000 — Define cursor ownership, delivery, and acknowledgement`, before
`SWF-004`–`SWF-007`.

It must choose one cursor owner, define batch-level acknowledgement using the response's opaque next
offset, and split two surfaces:

- a materializer/sink path where row effects, feed ownership, generation, and the batch checkpoint
  commit in one typed application transaction; and
- an observer path over already-accepted immutable state/change notifications. Observation is
  explicitly at-least-once or resettable and never advances the durable source cursor merely because
  `next()` returned.

Add a minimal `SubscriptionStore`/`CheckpointStore` protocol to `SWF-004`; it must exist even when
the optional row sink does not. Make `SWF-005` depend on `SWF-000`. Re-scope `SWF-007` to a richer
row-materialization transaction that extends the same checkpoint contract instead of inventing a
second persistence model.

**Fixed, executable acceptance:** process exactly 10,000 envelopes in 100-envelope response batches.
For each of 5 crash cuts (before delivery, after delivery/before apply, during sink transaction,
after sink commit/before return, after acknowledgement) and 4 outcomes (normal, cancellation,
replacement, reset), execute 100 repetitions: 2,000 runs. The reference sink must have zero lost
effects, one committed cursor per committed batch, and zero duplicate row effects; the observer-only
surface must document and test replay rather than claiming exactly-once side effects. A second cursor
owner for the same local materialization identity must be rejected in 100/100 attempts.

### P0 — Renewal, capability refresh, and close cannot guarantee release after an ambiguous response

A renewal is a create. If a replacement create succeeds but its response is lost, the client still
knows only the old shape ID; deleting that ID does not release the newly created claim. Retrying the
same named create can recover the authoritative handle, but `SWF-004` never requires close to resolve
an ambiguous renewal before release. The problem is sharper with signed reads: `SEC-004` permits a
new capability while durable stream identity stays unchanged, whereas the TypeScript client treats
equal shape ID/path as “no change” and would retain stale access material. A signed stream URL is a
credential, not durable handle state.

**Required spec edits:**

- Extend `PROTO-002` with an explicit ambiguous-renewal close algorithm: either add a
  principal-scoped idempotent release-by-subscription endpoint, or require retrying the same create
  to an authoritative response and then releasing the returned `(shapeID, subscription)`. A client
  may not declare close complete while this ambiguity is unresolved.
- Amend `PROTO-001`/`SEC-004` to separate stable persisted identity (`template`, subscription,
  shape ID, stream identity, opaque offset) from ephemeral access material (signed URL/token and
  expiry). Every successful renewal refreshes access material and lease policy even when shape
  ID/path are unchanged. Access material is never persisted or logged.
- Amend `SWF-004` to make concurrent `close()` calls await one client-owned close task. Cancellation
  of one waiter must not cancel cleanup. Define a typed `CloseResult`: `released`,
  `deferredUntilLeaseExpiry(deadline)`, or a thrown permanent protocol/auth failure. Remove the
  unspecified best-effort behavior inherited from TypeScript.

**Fixed, executable acceptance:** run 100 repetitions at each of 12 response-loss cuts covering
initial create, renewal, replacement create, capability refresh, release, and their response-body
boundaries (1,200 runs). After each close outcome, the gateway's subscription lookup must report zero
active claim or the exact persisted deferred-expiry deadline. Run 1,000 renewals with unchanged
shape ID/path and a different capability every time; requests using the previous capability must fail
and all 1,000 current capabilities must resume the same opaque offset.

### P0 — `AsyncThrowingStream` is not a no-drop backpressure design, and transaction buffering is impossible as scoped

`SWF-005` leaves “single-consumer or explicitly multicast” undecided and asks only for bounded
envelope count/bytes. Standard `AsyncThrowingStream` buffering policies drop oldest/newest elements;
its default is unbounded. The sibling package demonstrates the hazard: its completed `SyncBatch`
stream uses default buffering, while its 50,000-message limit bounds only the unfinished protocol
batch. Neither bounds a stalled application consumer. Multicast adds a cursor and lag policy per
subscriber, so it cannot be an implementation detail.

The same task conditionally promises transaction atomicity by buffering/spilling through
`headers.last`, but no spill format, file protection, cleanup, quota, or crash behavior is owned.
Transactions are deliberately unbounded by the engine; “memory below the client buffer bound for the
maximum supported transaction” is impossible without an actual spill implementation. Event-level
delivery should not wait for transaction output work that the first release explicitly excludes.

**Required spec edits:**

- Split `SWF-005A — Pull-based event stream and recovery` from `SWF-005B — Optional transaction-atomic
  delivery`. `SWF-005A` chooses one internal consumer and uses direct pull or a custom suspending
  bounded channel. It never uses a dropping continuation policy. Observer fan-out consumes accepted
  materialized state and has a documented lag-to-reset policy; it does not own the source offset.
- Measure raw response/decompressed bytes, decoded resident bytes, envelope count, and queued
  notification bytes separately. Overflow cancels the current generation and emits one reset; no
  arbitrary item is dropped.
- Make `SWF-005B` conditional on `PROTO-003` and `ENG-002`. It must implement an authenticated,
  file-protected, quota-accounted spill with atomic finalize, startup cleanup, and cancellation. If
  this task is not enabled, transaction markers are decoded but observers explicitly receive
  event/response-level atomicity.

**Fixed, executable acceptance:** for both count and byte budgets, test exactly `limit-1`, `limit`,
and `limit+1` under 3 consumer rates and 2 reconnect positions, 100 repetitions each (3,600 runs).
At `limit+1`, there must be exactly one reset and zero silent drops; at or below the limit, all keys
must be observed in order. For `SWF-005B`, replay 32 payload patterns at each of 7 sizes: 1 byte below,
exactly at, and 1 byte above both memory and spill thresholds, plus one 2 GiB synthetic size (224
transactions), with 20 crash cuts each (4,480 runs). Peak resident decoded storage must stay below
the configured byte bound plus one decoded envelope, and startup must remove 100/100 orphaned spill
files without exposing plaintext after the configured protection check.

### P0 — Native subset identity cannot be correct with the present page protocol

The live feed correctly supplies `env.key`, including the engine's composite-key escaping. The
one-shot subset response contains only `rows` and `lsn`, however, and the TypeScript client seeds its
set with `String(row[pk])`. That already diverges for composite keys and can diverge for encoded
single-key values. Swift cannot obey `SWF-006`/`SWF-009`'s opaque-key rule by reverse-engineering a
page key from row fields. The current shared TypeScript schema also exposes only one primary-key
column, while the engine supports ordered composite PKs.

**Required spec edits:**

- Add `PROTO-001A — Add keyed query pages and result schema`, then
  `ENG-001A — Emit canonical keyed query pages`, depending on `PROTO-001A`. A page row is
  `{ key: opaque string, value: object }`, not a bare object; the cursor is opaque and binds template
  version, ordered values, direction, schema generation, and visibility fence. The server supplies
  the same key function used by shape envelopes. Make `SWF-009` depend on both `ENG-001` and
  `ENG-001A`.
- Amend `SWF-009` to use only page-provided opaque keys for `present`, tombstones, deletes, and sink
  ownership. Key decoding is diagnostic/convenience functionality and is never used for identity.
- Add the keyed-page change to the TypeScript reference/client before declaring the shared subset
  corpus authoritative.

**Fixed, executable acceptance:** generate 100,000 one- through four-component PK tuples with a fixed
seed, including empty strings, backslashes, U+001F, non-ASCII scalars, booleans, floats, and both
`Int64` extremes. For every tuple, assert page key equals snapshot-envelope key equals live-envelope
key and that all 100,000 delete targets are exact. Add 12 adversarial rows whose projected PK-looking
fields stringify to the same text under a naive client; the keyed implementation must preserve 12
distinct identities and a `String(row[pk])` mutation must fail the corpus.

### P0 — The Swift dependency graph blocks its own supported release profile

There is a direct cycle: `PROTO-003` depends on `ENG-002`, and `ENG-002` depends on `PROTO-003`.
`SWF-002` depends on all of `PROTO-001`–`PROTO-004`; because `PROTO-004` depends on `PROTO-003`, even
event-level models wait for transaction-atomic output. `SWF-005` repeats that dependency. Separately,
`SWF-013` depends on numeric ranges containing optional sink, inventory-dependent aggregates, and
gated subsets, and G7 requires every `SWF-*`; the first native release therefore cannot ship until
`ENG-001` and native subsets finish despite section 2 excluding them. Finally, `SWF-003` requires the
implemented gateway even though section 4 says native implementation may begin against fixtures.

**Required dependency edits:**

| Task | New dependency rule |
| --- | --- |
| `PROTO-003` | Depends on `PROTO-001`; owns the normative framing design/fixtures. |
| `ENG-002` | Depends on `PROTO-003`; implements and proves those fixtures. |
| `PROTO-004A` (split) | Base event-level negotiation; depends on `PROTO-001`, `PROTO-002`. |
| `PROTO-004B` (split) | Transaction capability/version negotiation; depends on `PROTO-003`, `ENG-002`. |
| `SWF-002` | Depends on `SWF-001`, `PROTO-001`, `PROTO-002`, `PROTO-004A`, and the shared base corpus. |
| `SWF-003A` (split) | Fixture transport; depends on `SWF-002`; no live gateway dependency. |
| `SWF-003B` (split) | Authenticated gateway integration; depends on `SWF-003A`, `SEC-002`, `SEC-004`. |
| `SWF-005A` | Event-level only; depends on `SWF-000`, `SWF-003A/B`, `SWF-004`, `ENG-003`. |
| `SWF-005B` | Optional transaction atomicity; depends on `SWF-005A`, `PROTO-004B`. |
| `SWF-013` / G7 | Depend on the explicit core set plus only features selected by `GOV-002`; never an ID range. `SWF-009` is conditional on `ENG-001`. |
| `TST-006` | Run once per selected compatibility/native feature profile; do not require disabled `SWF-007/008/009`. |

**Fixed, executable acceptance:** check in a machine-readable adjacency/profile manifest and a
`spec-dag-check` command. It must report exactly zero cycles, zero undefined IDs, zero numeric-range
dependencies, and zero disabled tasks reachable from each release profile. Four fixed fixtures must
pass: core event shapes, core+aggregates, core+sink, and core+subsets. Four one-edge mutations—restore
the current protocol cycle and make each optional feature mandatory—must all fail.

### P1 — Generic `Decodable` cannot implement the declared scalar contract

Rows are schema-directed: an `int` arrives as a JSON number inside the safe range and as a decimal
string beyond it; text values may also look numeric; missing projection and SQL NULL differ. A normal
`Row: Decodable` with an `Int64` property does not accept both encodings, and decoding every number as
`Double` loses precision. `CircuitsValue` in `SWF-002` does not automatically fix the generic API in
the note 09 sketch. Aggregate `value` has the same union, and aggregate `n` is currently emitted by
Rust as an unrestricted `i64` JSON number, unlike lossless integer rows/sums.

**Required spec edits:** amend `PROTO-001` to make every scalar representation schema-directed and
make aggregate `n` use the same lossless integer rule. Amend `SWF-002` to provide a lossless raw row
model and `ResultSchema`. Amend `SWF-006` to require an injected `RowDecoder<Row>` or a
`CircuitsRowDecodable` conformance that receives raw fields plus schema; do not promise arbitrary
`Decodable`. Field absence must be represented before typed decoding, and unknown scalar text must
survive round-trip.

**Fixed, executable acceptance:** a 96-case scalar corpus must include both `Int64` extremes,
the boundaries around positive and negative 2^53, negative zero and finite `Double` boundaries, numeric-looking text, booleans,
empty/non-ASCII text, UUID/timestamp text, NULL, and missing. Run all 96 through row, predicate
parameter, aggregate `value`, aggregate `n`, and projection paths in all three languages (480 path
assertions). Add 32 generated typed Swift models: 16 using provided lossless wrappers and 16 custom
decoders; all 32 must decode both integer wire forms without coercing numeric-looking text.

### P1 — Actor reentrancy and task ownership requirements stop short of the dangerous boundaries

The generation recheck is correct but incomplete. Setup can succeed remotely and fail during HEAD,
preload, persistence, or tail installation; each path needs compensating named release. A tail must be
cancelled before it is awaited or close can hang. Renewal cadence must be recomputed when the server
changes `leaseSeconds`; zero disables timers; scheduling needs a monotonic clock and jitter that
never crosses the safe deadline. `CircuitsClient` needs an actor registry so duplicate opens cannot
create competing cursor owners and `client.close()` can snapshot and await every one-shot close.
Caller cancellation must not abandon remote compensation or release once a mutation might have
landed.

**Required spec edits:** expand `SWF-004` with an explicit transition table for `idle`, `creating`,
`active`, `replacing`, `resetting`, `closing`, `closed`, and `deferredRelease`. Name every suspension
point in a checked-in cut-point manifest. Require a client-owned cleanup task, cancel-then-await task
order, monotonic lease deadline, server-updated cadence, client-wide registry, duplicate local-owner
rejection, and compensation for every partially completed setup. Prohibit `Task.detached` except for
a documented executor-independent task with an owned cancellation/join handle.

**Fixed, executable acceptance:** define exactly 24 suspension cut points and inject cancellation
before and after each, 100 times (4,800 runs), plus 10,000 model-generated transitions. Run lease
windows `0`, `1`, `3`, `60`, and `1,800` seconds with renewal responses that change the next window at
each of 4 attempts. Assertions are: at most one reader, renewal, close task, cursor owner, and active
claim; all spawned tasks joined; no state mutation from a stale generation; and 100/100 create-then-
setup failures leave no active claim.

### P1 — Sink ownership is underspecified for replacement, projection, and tenant reset

`SWF-007` names `(feedID, opaqueKey)` but never defines `feedID`. If it is the server shape ID,
replacement loses ownership continuity; it must be the stable local subscription/materialization
identity across server replacements. Two feeds projecting different columns cannot be merged by
ordinary whole-row upsert without per-feed versions or column ownership. Close, reset, and tenant
switch also need distinct semantics: close may stop future delivery without deleting shared rows,
reset must replace one generation atomically, and tenant change must make prior data inaccessible
before new work begins. The sibling package's `Any?` transaction proves why the new API needs a typed,
actor-confined transaction token.

**Required spec edits:** make `SWF-007` define stable local `MaterializationID`, generation-scoped
ownership, typed non-`Sendable` transaction lifetime, batch checkpoint atomicity, and explicit
`close`, `reset`, `purge`, replacement, and tenant-transition operations. For v0, reject overlapping
feeds with different projection/schema versions unless an application adapter supplies a declared
merge policy; a generic library must not invent one. Move app-specific delete/merge decisions to an
adapter selected by the call-site ownership inventory.

**Fixed, executable acceptance:** test exactly 8 ownership topologies (identical feed, overlapping
predicate, disjoint predicate, replacement ID, same projection, disjoint projection, conflicting
projection, tenant change) against 6 operations (upsert, move-in, move-out, delete, reset, close), 100
orders each: 4,800 schedules. Every schedule must assert row value, per-feed ownership, generation,
and cursor in one transaction. The two unsupported projection topologies must reject before opening a
network request; the other six must converge with zero delete of a still-owned row.

### P1 — App lifecycle, credential, and protected-storage contracts need host-visible ordering

`SWF-010` correctly says not to trust suspended timers/sockets, but “every lifecycle state and network
transition” is not an executable matrix. Resume must renew/validate/rebind before publishing stale
cache as current. Logout ordering is security-critical: stop readers and renewal, attempt named
release while credentials remain valid, atomically hide/purge tenant cache and control state, then
clear credentials. A background task is only a finite checkpoint opportunity; background
`URLSession` must not be used to pretend an endless long-poll can survive suspension.

The library should request a fresh authorization header for every request and never persist the
token. The host owns Keychain policy. If background access is required it must deliberately select an
AfterFirstUnlock accessibility class; otherwise a WhenUnlocked class is safer. Regenerable cache and
subscription metadata belong under Application Support with an explicit file-protection/backup
policy. Signed URLs, authorization, tenant IDs, raw template parameters, and rows must not enter
state restoration, logs, metrics, or crash breadcrumbs.

**Required spec edits:** split `SWF-010A — Host lifecycle/session contract` from simulator/device
qualification, and expand `SWF-012` with a persisted-field allowlist and data-protection table. Define
the exact logout/tenant-switch sequence, resume readiness state, expiration handler, foreground
renewal, offline backoff, and credential-refresh generation fence. The host injects lifecycle and
background opportunities; the package owns no global notification observer. `CredentialProvider`
returns fresh request headers, not a persisted bearer value.

**Fixed, executable acceptance:** check in a 7-state by 8-event transition table (56 cells) and run
each cell 100 times under virtual time (5,600 runs). Run 12 XCUITest host scenarios—foreground,
background expiration, suspension past lease, terminate during apply, relaunch, offline launch,
online recovery, 401 refresh, refresh cancellation, logout, tenant switch, and protected-data
unavailable—25 times on each of 2 pinned iOS runtime versions (600 runs). Automated filesystem,
`URLProtocol`, log, metric, and crash-report scans must inspect exactly 7 forbidden data classes and
find zero occurrences; 100/100 tenant switches must make old rows and checkpoints unreadable before
the new principal opens a request.

### P1 — Wire hardening and forward-compatibility behavior are not precise enough

“Preserve unknown additive fields where forward compatibility requires it” does not decide what may
be ignored. Unknown optional metadata can be retained or ignored, but an unknown operation, control,
required capability, key codec, reset reason, or transaction marker cannot be skipped without risking
permanent divergence. Body limits must apply to compressed bytes, decompressed bytes, JSON nesting,
field count, string length, and partial-frame storage. Redirect handling must strip/reacquire
credentials and reject cross-origin destinations. Long-poll should be the single v0 transport; the
unbounded sibling `SSEParser` is not a reusable implementation unless SSE becomes a separately gated
feature.

**Required spec edits:** add a normative unknown-field/control matrix to `PROTO-004A` and exact parser
limits to `PROTO-001`. Amend `SWF-002/003A` to fail with a typed reset-required/protocol-incompatible
outcome for unknown semantic values, enforce all limits while bytes arrive, disable implicit
cross-origin redirects, and treat the server response offset as one batch checkpoint. Move SSE out of
the core deliverable until it has independent framing and memory acceptance.

**Fixed, executable acceptance:** construct 20 parser partitions with 256 seeded cases each (5,120
cases): truncated JSON, malformed UTF-8, compression expansion, depth, field count, field length,
array length, numeric length, unknown additive field, unknown operation, unknown control, unknown
codec, unknown version, missing required field, duplicate field, wrong content type, 204, partial
body, same-origin redirect, and cross-origin redirect. All 5,120 must produce the table-specified
typed result, never a silent event drop. A 10,000-input byte fuzzer with a fixed seed must stay under
the configured compressed + decompressed + decoded bounds and terminate within the per-case deadline.

### P1 — Package and test gates do not prove the claimed Apple platform surface

`SWF-001` says Linux/macOS tests “where feasible,” while `SWF-013` claims an iOS/macOS release. The
sibling package currently runs only `swift test` on a macOS runner plus a dependency-boundary script;
that does not compile the iOS product, exercise `URLSession` cancellation under an iOS runtime, or
test application suspension/protected-data behavior. Conversely, Linux should be either a supported
contract-test host with `FoundationNetworking` or explicitly non-supported, not an optional green
badge. The native package repository, pinned Swift/Xcode toolchain, release tag coordination with the
server contract, and API baseline owner are also not named.

**Required spec edits:** amend `SWF-001` to name the package repository and starting SHA, pin one Swift
6.1 and one Xcode version, and define a CI matrix rather than “where feasible.” Require macOS unit
tests, iOS simulator build/tests, Linux protocol/codec tests if Linux is claimed, strict-concurrency
warnings-as-errors, dependency-boundary enforcement, and one sample-app build. Amend `SWF-013` to own
the symbol-graph/API baseline, signed tag/artifact provenance, privacy manifest decision, and exact
current/previous server compatibility matrix. Optional sink/aggregate/subset products are emitted and
qualified only when enabled by the release profile.

**Fixed, executable acceptance:** for every release candidate, run exactly 6 clean-checkout jobs:
macOS debug tests, macOS release tests, iOS simulator build/tests, sample-app build/tests, Linux
protocol tests (or a profile assertion that Linux is unsupported), and API/dependency/privacy audit.
Run the core package against exactly 2 server protocol versions (current and previous) and 2 client
fixture versions, once for create/read and once for renew/release: 8 compatibility jobs. CI must fail
6/6 injected mutations:
Protocol imports networking, Client imports a database, a mandatory runtime dependency appears,
strict-concurrency warning appears, public symbol breaks without major version, or privacy-required
API lacks declared evidence.

## Required task graph after correction

The minimum Swift-native order should be:

```text
GOV-002 + SEC-003 policy decision
  -> PROTO-001/001A/002/004A + shared base fixtures
  -> SWF-001 -> SWF-002 -> SWF-003A
  -> SWF-000 -> SWF-004 -> SWF-005A -> SWF-006
  -> SWF-010A/011/012 -> core SWF-013

SEC-002/004 -> SWF-003B ------------------------^
SWF-006 + inventory -> optional SWF-007 ----------^
SWF-004/005A + inventory -> optional SWF-008 -----^
ENG-001 + ENG-001A + PROTO-001A + SWF-007 -> optional SWF-009
PROTO-003 -> ENG-002 -> PROTO-004B -> optional SWF-005B
```

This preserves the intended first milestone: an event-level native shape client can be implemented
against fixtures, integrated through the authenticated gateway, and released without pretending
subsets, a database sink, or transaction-atomic observers are complete.

## Evidence anchors

- `notes/16-production-readiness-and-swift-migration-spec.md:37-51, 68-91, 195-269, 293-348,
  1052-1301, 1393-1411, 1535-1547`
- `notes/09-swift-library-strategy.md:95-104, 127-187, 204-228`
- `notes/13-typescript-client-reference.md:46-119, 125-165, 171-300, 315-337`
- `packages/client/src/index.ts:208-317, 346-431`
- `packages/client/src/subset.ts:176-270, 311-348, 362-386, 483-529`
- `packages/protocol/src/types.ts:6-41, 145-170, 190-207`
- `apps/engine/src/http.rs:148-176, 186-207, 249-299, 540-581`
- `apps/engine/src/schema.rs:255-276`; `apps/engine/src/value.rs:40-117`;
  `apps/engine/src/engine/output.rs:195-211`
- `../electric-sync-swift/Package.swift:1-32`; `.github/workflows/ci.yml:1-22`;
  `Sources/ElectricSync/SSEParser.swift:7-71`;
  `Sources/ElectricSync/ElectricCollection+KeepSynced.swift:64-110`;
  `Sources/ElectricSync/ElectricSyncClient.swift:2656-2785`;
  `Sources/ElectricSync/Providers.swift:3-142, 332-425`

Until these changes are made, the safe implementation boundary is narrower than G7 currently says:
fixture-driven, event-level, authenticated template shapes with an explicit control-state store and
one internal cursor owner. Native subsets, transaction-atomic observers, generic overlapping-feed
persistence, and arbitrary client predicate APIs must remain outside the supported release.
