# Swift library strategy

## Decision

**Recommendation — build a small, Circuits-native Swift package, while first shipping an
opt-in Electric-protocol adapter in `electric-sync-swift`.**  The adapter is the low-risk
way to validate Circuits against an existing iOS application; the new package is the only
option that gives Circuits a coherent, supportable API for shapes, subsets, and aggregates
without importing a much larger app-replica framework.

This is deliberately reversible: neither package needs to replace the other, and the native
package initially has no dependency on `electric-sync-swift` or an application's database.
Existing ElectricSync users can remain pinned and point their existing provider at Circuits.

## Evidence and constraints

- Circuits already exposes an Electric-compatible `GET /v1/shape`; the adapter owns snapshot,
  live long-poll, `must-refetch`, and Electric headers.  This is implemented by
  `apps/engine/src/electric.rs::shape` / `shape_inner` and documented in
  `apps/engine/README.md` (“HTTP endpoints”).
- Circuits' differentiating surface is not the Electric protocol: it has one-shot subsets and
  live aggregates (`Engine::query_subset`, `Engine::create_aggregate` in
  `apps/engine/src/engine/mod.rs` and `engine/lifecycle.rs`).  The current TypeScript client
  exposes `shape`, `query`, `subset`, and `aggregate` in
  `packages/client/src/index.ts::ElectricIvmClient`.
- The current extended client depends on a tRPC control plane plus direct durable-stream reads
  (`packages/client/src/index.ts::createClient`); its subscriptions additionally own identified
  lease renewal and idempotent release (`startLeaseRenewal`, `deleteShapeWithRetry`).  A Swift
  SDK should not reverse-engineer or couple its public API to those TypeScript implementation
  libraries.
- `electric-sync-swift` is a storage- and transport-injected Electric client.  Its public
  `HTTPClientProvider` / `HTTPStreamClientProvider`, `DataCacheProvider`, and
  `MetadataProvider` contracts are in
  `/Users/bozilabs/labs/electric-sync-swift/Sources/ElectricSync/Providers.swift`; the central
  owner is `ElectricSyncClientImpl` (an actor) in `ElectricSyncClient.swift`.
- It already models the relevant Electric request/response continuity fields:
  `ElectricShapeRequest`, `ElectricMessage`, and `SyncState` in
  `electric-sync-swift/Sources/ElectricSync/Models.swift`.  Its collection API owns local
  replica semantics, DNF tracking, row ownership, bootstrap/recovery, and application session
  fences (`ElectricCollection`, `ElectricShapeReplica`, and
  `ElectricCollectionModel` under `Sources/ElectricSync/`).
- That breadth is material: the audited package has exactly 13,203 production lines and 23,220
  test lines (36,423 total; `wc -l Sources/ElectricSync/*.swift Tests/ElectricSyncTests/*.swift`),
  while its SwiftPM
  product has only `ElectricSync` and targets iOS 18/macOS 15
  (`electric-sync-swift/Package.swift`).  It intentionally has no library-target SwiftPM
  dependencies; GRDB is test-only (same manifest and README).
- Circuits treats subscriptions as named leases: repeated create is renewal and a named delete is
  idempotent.  A Swift implementation must preserve that lifecycle rather than treating a stream
  read as proof of liveness.  See `docs/ARCHITECTURE.md` and
  `apps/engine/src/engine/lifecycle.rs::create_shape_as` / `create_aggregate_as`.
- A subset is expressly a one-shot page plus an independently followed base-predicate feed, not
  a range-shaped live subscription.  The required page/live seam uses an LSN watermark and delete
  tombstones in `packages/client/src/subset.ts::{createSubset,mergeFeedDelta}`; the engine side is
  `Engine::query_subset`.

## Viable options

### A. Compatibility-only adapter in `electric-sync-swift`

First configure the application's existing `HTTPClientProvider` and
`HTTPStreamClientProvider` to target `/v1/shape`, if those providers already translate
`ElectricShapeRequest` and decode the Electric wire response.  No ElectricSync core or product
change is needed in that case.  Only if that concrete transport is missing should the package
provide `CircuitsElectricHTTPProvider` and `CircuitsElectricSSEProvider` (preferably as a separate
product); they translate `ElectricShapeRequest` into `/v1/shape` query parameters, decode the
Electric wire messages/headers into `ElectricMessage`, and use the existing collection/replica
machinery unchanged.

This is feasible because the existing provider boundary is exactly one-shot fetch plus an
`AsyncThrowingStream` (`Providers.swift::{HTTPClientProvider,HTTPStreamClientProvider}`), and
Circuits provides the compatible endpoint (`apps/engine/src/electric.rs::shape`).  It provides
shapes only.  Do **not** pretend that `orderBy`/`limit` are live shapes: Circuits reserves them
for subsets (`docs/live-queries-guide.md`, §1 and §6).

Benefits: fastest route to a real iOS app, preserves cache/GRDB integration and existing cursor
state, and gives conformance coverage for the Electric surface.  Costs: it imports all of
ElectricSync's app-local persistence and recovery concepts, cannot naturally expose native
aggregates, and leaves Swift with a non-native API.

### B. Refactor `electric-sync-swift` into dual Electric/Circuits backends

Extract its transport, stream decoder, lifecycle, and storage-independent pieces into products;
then add a native Circuits backend for shapes, subsets, and aggregates.  Existing
`ElectricCollection` remains a compatibility façade.

Benefits: one repository and one shared set of retry/session/SSE primitives.  Costs: the current
actor/replica logic assumes `ElectricShapeRequest` and `ElectricMessage`
(`ElectricSyncClientImpl`, `ElectricShapeReplica`, and `SyncBatch` under `Sources/ElectricSync/`),
so the extraction changes the behavioral core on which pinned users rely.  It couples Circuits'
independent client model to DNF/ownership and forces each new Circuits feature through a large
compatibility abstraction.  This is a viable later consolidation only after both products have
stable conformance suites.

### C. New focused package: `ElectricCircuitsSwift`

Create a new SwiftPM repository/package with a value-typed Circuits protocol model and one actor
that owns each subscription's mutable cursor, renewal task, and close state.  It is a streaming
client, not an ORM or a database: applications apply changes to their own GRDB/SwiftData/Core
Data store through an optional sink protocol.

Benefits: mirrors Circuits' actual primitives, has a small dependency and semver surface, and
does not threaten existing ElectricSync users.  Costs: it needs a new transport contract and its
own tests for page/live positioning, leases, and retry/cancellation.  This is the recommended
end state.

## Weighted decision matrix

Scores are 1 (weak) to 5 (strong); weighted total is score × weight, maximum 500.  “Existing-user
safety” weights both source compatibility and avoiding a behavioral regression in the pinned
0.1.x package.

| Criterion | Weight | A: adapter | B: dual backend | C: native package |
|---|---:|---:|---:|---:|
| Existing-user safety | 25 | 5 / 125 | 2 / 50 | 5 / 125 |
| Time to first production validation | 20 | 5 / 100 | 2 / 40 | 3 / 60 |
| Full Circuits feature fit | 20 | 2 / 40 | 5 / 100 | 5 / 100 |
| Long-term maintenance isolation | 15 | 2 / 30 | 2 / 30 | 5 / 75 |
| Protocol/versioning control | 10 | 3 / 30 | 4 / 40 | 5 / 50 |
| Testability and rollback | 10 | 4 / 40 | 2 / 20 | 5 / 50 |
| **Total** | **100** | **365** | **280** | **460** |

**Recommendation — choose C as the product decision, and execute A as its first compatibility
milestone.**  A's fast validation is not a reason to let it become the permanent native API;
the 20k-line implementation surface and incompatible feature shape make that expensive.

## Proposed public API and module boundaries

Before a native beta, define and version a small JSON/HTTP contract owned by Circuits.  It must
cover shape/aggregate create-renew-release, subset page responses, durable-stream read/HEAD, and
typed errors.  Treat the current tRPC routes in `apps/api/src/router.ts` and the control endpoints
in `apps/engine/README.md` as implementation evidence, not as a Swift public contract.

```swift
// ElectricCircuitsProtocol
public struct ShapeDefinition: Sendable, Codable, Hashable { /* table, predicate, columns */ }
public indirect enum Predicate: Sendable, Codable, Hashable { /* Circuits AST */ }
public struct SubsetDefinition: Sendable, Codable, Hashable { /* order, limit, projection */ }
public struct AggregateDefinition: Sendable, Codable, Hashable { /* fn, column, predicate */ }
public enum Change<Value: Sendable>: Sendable { case upsert(Value, key: String, lsn: String?); case delete(key: String, lsn: String?) }

// ElectricCircuitsClient
public actor CircuitsClient {
  public func shape<Row: Decodable & Sendable>(_ definition: ShapeDefinition,
    as type: Row.Type, subscription: String = UUID().uuidString) async throws -> ShapeSubscription<Row>
  public func subset<Row: Decodable & Sendable>(_ definition: SubsetDefinition,
    as type: Row.Type) async throws -> SubsetPage<Row>
  public func aggregate(_ definition: AggregateDefinition,
    subscription: String = UUID().uuidString) async throws -> AggregateSubscription
}

public actor ShapeSubscription<Row: Decodable & Sendable> {
  public func changes() -> AsyncThrowingStream<Change<Row>, Error>
  public func renew() async throws
  public func close() async
}
```

`ElectricCircuitsProtocol` is only stable `Codable` models, predicate validation, and envelope
codecs.  `ElectricCircuitsTransport` contains `URLSession` request, SSE/long-poll decoding,
HTTP status/error normalization, and a test URL protocol.  `ElectricCircuitsClient` contains
create/renew/delete and stream lifecycle.  An optional `ElectricCircuitsReplica` product defines
an application-supplied transactional `ChangeSink`; it must never select a persistence framework.
`ElectricCircuitsElectricCompatibility` is a separately versioned bridge product/package and
may depend on `ElectricSync`, never the reverse.

**Recommendation — use `URLSession` for HTTP and SSE/long-poll, not Network.framework.**  The
networking guidance selects URLSession for HTTP(S), and the core is already HTTP-stream shaped.
Each subscription actor must re-check cancellation/closed state after awaits, serialize renewal
before release, and surface a bounded/reconnectable `AsyncThrowingStream`; do not use
`Task.detached` as the normal lifecycle mechanism.  This matches Swift's actor/reentrancy and
structured-concurrency guidance and directly protects the engine's “stop renewal before delete”
lease invariant evidenced by `packages/client/src/index.ts::startLeaseRenewal`.

## Dependency, platform, and maintenance policy

**Recommendation — v0 has no mandatory third-party dependencies.**  Use Foundation/
FoundationNetworking as necessary and URLSession injection for tests.  Do not depend on GRDB,
SwiftData, tRPC, JavaScript, or a durable-streams Swift client.  Add a dependency only for a
concrete platform gap, behind a separate product.  This preserves the existing package's useful
dependency posture (`electric-sync-swift/Package.swift`, README) and keeps transport/storage
choices app-owned.

**Recommendation — first supported targets: Swift 6.1+, iOS 18+, macOS 15+.**  This matches the
known production package's target set and keeps strict-concurrency and CI scope contained.  Do
not claim watchOS/tvOS/visionOS support in v0; add each only after physical-device/CI streaming
and background-lifecycle tests.  Lower deployment targets can be a post-1.0 compatibility project,
not an untested promise.

Expected maintenance: A is low initial/medium ongoing (wire-parity tests follow Electric
changes); B is high ongoing (a shared internal refactor couples two products); C is medium initial
and low-to-medium ongoing if the JSON contract is explicitly versioned.  The C team owns API
conformance fixtures and simulated transport tests; engine maintainers own contract compatibility
and an end-to-end fixture server.

## Transition, semver, migration, and rollback

1. **Compatibility spike (no user migration).**  First point the app's existing HTTP providers at
   `/v1/shape`; do not alter `ElectricSync` defaults or persisted `SyncState`.  Add an adapter as
   a new product or separate `0.1.x` additive release only when that concrete transport is absent.
   Exercise snapshots, resumed offsets, `must-refetch`, SSE, long-poll, reconnect, and
   cancellation against the Circuits conformance stack.  Existing apps opt in by selecting the
   Circuits provider/base URL.
2. **Contract gate.**  Publish a versioned native JSON contract and golden fixtures shared by the
   Rust engine and Swift decoder.  The contract must state numeric representation (Circuits sends
   oversized integers as decimal strings: `packages/protocol/src/types.ts::Value` and
   `apps/engine/src/value.rs::Value::to_json`), canonical table names, errors, stream offset
   semantics, and lease responses.
3. **Native alpha (`ElectricCircuitsSwift 0.1`).**  Ship shapes first, explicitly experimental;
   independent app-side sinks make adoption additive.  Add subsets only once tests cover the
   snapshot-LSN/tombstone seam, then aggregates.  Never silently map a shape to a subset.
4. **Dual run and migration.**  An app can run ElectricSync against `/v1/shape` and native
   Circuits shapes side by side only on distinct local tables/ownership domains.  Compare
   canonical key/value output in a test or shadow store; cut over one feature/table at a time.
   Do not share persisted cursors across libraries: their identities and retention assumptions
   differ.
5. **1.0 gate.**  Require stable contract versioning, semver-locked fixtures, reconnect/lease/
   `must-refetch`/schema-retirement tests, and at least one production migration before 1.0.
   From 1.0, add protocol fields compatibly and reserve major versions for required behavioral or
   source breaks.  Keep ElectricSync's public API independent; deprecate no existing API merely
   because Circuits-native exists.

Rollback is a configuration change during the compatibility phase: point the existing provider
back at the prior Electric service and retain its own cursor/cache.  During native rollout, close
the native subscription (identified close is idempotent), discard only the new library's local
replica data, and re-enable the ElectricSync owner.  Server-side shape retention and lease expiry
make an abandoned native claim eventually recoverable, but the client must still make its best
effort named release.  This preserves both existing users and a clean escape hatch if native
semantics or transport behavior prove incomplete.

## Principal risks and mitigations

| Risk | Mitigation |
|---|---|
| Native Swift binds to unversioned tRPC/internal stream details | Contract gate and protocol fixtures before public beta; no tRPC dependency. |
| A lease renewal races `close()` and pins a shape | One subscription actor; stop and await renewal before identified delete, matching `startLeaseRenewal`. |
| Subset page/live overlap resurrects deletes | Port the LSN watermark and tombstone invariants from `mergeFeedDelta`; test delayed page versus live delete. |
| Convenience ORM abstraction recreates circuit-per-page behavior | Keep `subset` one-shot and tail only the base predicate, as `Engine::query_subset` documents. |
| Refactor destabilizes pinned ElectricSync users | Do not refactor its core for v0; bridge in a one-way optional product and maintain independent CI. |
| Strict-concurrency escape hatches hide stream races | Use `Sendable` values and actors; audit any `@unchecked Sendable` or detached task as an exception with a test. |
