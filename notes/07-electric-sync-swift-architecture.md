# electric-sync-swift architecture audit

Audited 2026-08-22 against /Users/bozilabs/labs/electric-sync-swift: all 25 production files and 24 test/support files (13,203 production LOC; 23,220 test LOC). This is an application-facing Electric Shape client, not a general sync engine or a Circuits client. The library target is Foundation/CryptoKit-only; GRDB 7.10.0 is an exact test-target-only dependency (Package.swift:7-30, README.md:3-5, Scripts/check-dependency-boundaries.sh:34-62).

Verification: swift test --skip-build passed all 351 tests in 23 suites; the dependency-boundary script passed. Test code currently warns about deprecated MoveOutPattern aliases and ignored NSLock.withLock results. No files in the Swift checkout were changed.

## Executive assessment

The design is correctness-first: a host transaction combines model and metadata writes; cursor state moves only at protocol boundaries; live/snapshot interleavings are fenced; on-demand DNF tails are lease-gated; malformed tagged input fails closed. The resulting complexity is overwhelmingly tied to Electric's local-client protocol and app-specific storage contract.

For Circuits, use this as reference material for a future Swift consumer of the existing Electric-compatible /v1/shape adapter, not as engine code. Direct code reuse in the current Rust/TypeScript workspace is 0 LOC as-is (language and layer mismatch). A native Swift Circuits client could selectively lift about 1,432 LOC / 10.8%; 11,771 LOC / 89.2% needs an adapter redesign or should remain behind.

## Public API, platforms, and dependency map

Package.swift targets Swift 6.1, iOS 18, and macOS 15 (Package.swift:1-30). Main public surface:

- ElectricSyncClientImpl plus ElectricSyncClientConfiguration, the actor-backed protocol coordinator (ElectricSyncClient.swift:1069-1247).
- Generic ElectricCollection and ElectricCollectionModel for app models, cache reads, and atomic model materialization (Collections.swift:479-570; ElectricCollectionModel.swift:15-81).
- MetadataProvider, DataCacheProvider, HTTPClientProvider, HTTPStreamClientProvider, session/runtime/background/log/tracing providers (Providers.swift:5-142, 332-458).
- Predicate AST, subset SQL compiler, request/message/value models, fetch tracking and deduplicated subset loads.

    App model/database/auth/HTTP/telemetry
           |  model + providers + host transaction
           v
    ElectricCollection -> ElectricShapeReplica -> ElectricSyncClientImpl actor
           |                    |                    |
           |                    |                    +-- Shape request/resume/batch assembly
           |                    +-- owner GC, gates, demand/recovery fences
           v
    host cache transaction <- SyncBatch.apply -> MetadataProvider + MoveOutTagTracker
                                                       |
                                                       v
                                             Electric Shape service

    Stand-alone utilities: Predicate AST -> SubsetSQLCompiler; SSEParser; backoff;
    fetch coverage; runtime/session/tracing injection.

There is intentionally no concrete database, URLSession, authentication, or app-model decoder. HTTPStreamClientProvider receives already-decoded ElectricMessage values (Providers.swift:347-358); applications own URL construction, status/TLS policy, SSE decoding, and database migrations.

## Transport and Electric protocol coupling

ElectricShapeRequest includes immutable wire identity, table/predicate/order/limit, offset, handle, cursor, live flag, log/replica modes, and optional subset SQL; ElectricMessage adds payload/key/resume fields, Postgres snapshot/txids, tags, active conditions, move events, controls, and field presence (Models.swift:412-502, 595-661). This is an Electric Shape/Postgres client contract, not Circuits' durable-stream envelope.

The poll path fetches injected pages to a terminal boundary. The live path obtains injected SSE, batches at up-to-date, immediately projects truncate/must-refetch, preflights tagged input before publication, and caps an individual unfinished batch at 50,000 messages (ElectricSyncClient.swift:1175-1204, 2511-2567, 2571-2766). SSE-with-fallback switches to long polling after three SSE failures (Collections.swift:1282-1368).

SSEParser is deliberately only an event framer: UTF-8 data lines, comments ignored, id/event/retry ignored, and no pending-line/buffer byte limit (SSEParser.swift:7-72). It is not wired into the core client; its host transport must turn the framed payload into ElectricMessage.

## Persistence and collection abstraction

MetadataProvider is the actual persistence integration: coverage/observation records, cursor state, legacy adoption, row ownership/tag/tombstones, working-set cutover, and optimistic-publication retirement all accept a host opaque transaction (Providers.swift:13-142). DataCacheProvider only loads/clears typed rows; ElectricCollectionModel.processMessage performs row writes using the same transaction (Providers.swift:332-345; ElectricCollectionModel.swift:38-81).

That permits GRDB, Core Data, or another store, but atomicity is an app obligation. The Any? transaction handle also prevents Sendable checking from proving a database object never crosses executors. A Circuits Swift integration should instead expose a typed, actor-isolated storage transaction.

SyncState is not Codable, so adapters own stored-schema compatibility. Its new bridgedFromSyncMode attestation must move atomically with state, ownership, and tombstones (Models.swift:128-193). That is an application database migration requirement, not library JSON compatibility.

## Predicate/DNF machinery

SyncPredicateExpression is a recursive Boolean AST with scalar/membership/comparison leaves, normalization, JSON persistence, subset checking, and conservative subtraction. It feeds DeduplicatedLoadSubset and ElectricFetchTracker coverage planning (PredicateExpression.swift:126-186, 465-546; DeduplicatedLoadSubset.swift:50-169; ElectricFetchTracker.swift:185-229).

SubsetSQLCompiler validates dotted identifiers, emits PostgreSQL positional parameters, turns only null equality into IS NULL, and outputs sorted JSON parameter maps (SubsetSQLCompiler.swift:79-176, 179-273). This is the clearest portable semantic seam, though Circuits already has Rust SQL/predicate machinery and should not maintain a second source of truth.

Codable behavior has two edge cases:

- Predicate toJSONData/fromJSON deliberately use try? and return nil on malformed data (PredicateExpression.swift:536-545). That safely loses coverage/re-fetch optimization, but hides bad persisted metadata unless the host logs it.
- SubsetSQLParamValue is untagged JSON and decodes Bool -> Int -> Double -> String (SubsetSQLCompiler.swift:31-67). It cannot promise preservation of lexical numeric type across another decoder. Tests do not directly cover malformed predicate JSON, unknown enum values, or numeric ambiguity.

## Lifecycle, cursor ownership, reconnect, and backoff

- ElectricShapeReplica owns dormant/active/idle-grace/replacing/suspended state, shared stream ownership, ref-counted tokens, idle GC, snapshot/publication gates, and process-local tracker continuity (ElectricShapeReplica.swift:265-284, 1234-1481).
- On-demand DNF work has a synchronous lease fence. Recovery snapshots a revisioned demand inventory before releasing the tail, so last-lease cancellation cannot publish a stale tail (ElectricShapeReplica.swift:379-405, 888-985).
- Resume source distinguishes exact, legacy adopted/miss/pre-cutover, and app-attested eager<->progressive bridge. Only exact or bridged statically-simple durable state can rebuild membership; DNF/on-demand remain fail-closed (ElectricSyncClient.swift:1193-1224, 1332-1395).
- MoveOutTagTracker gives DNF membership/move-out logic; capability policy quarantines malformed or incompatible tagged input before durable writes (MoveOutTagTracker.swift:34-112; ElectricProtocolQuarantine.swift:80-157).
- ExponentialBackoffCircuitBreaker uses 0.5 s base, 30 s maximum, jitter, and rapid-identical-failure circuit opening (CircuitBreaker.swift:12-112).

These are valuable client correctness patterns but encode Electric's tag/offset/handle/cursor and local-row-ownership semantics. They do not belong in the Circuits server, where Postgres remains authoritative and circuit topology must not scale with shapes.

## Concurrency and Sendable posture

ElectricSyncClientImpl is the primary state owner. Purpose-specific actors protect fetch de-duplication, bootstrap admission, command serialization, recovery, snapshot tracking, and publication gates. Most crossing values and closures are Sendable.

Reference types used for small synchronous fences/tokens/trackers are deliberately NSLock-backed and marked @unchecked Sendable; an older stream manager uses one private serial DispatchQueue (ElectricSyncClient.swift:1159-1230; ElectricCollectionStreamManager.swift:3-62; ElectricShapeReplica.swift:888-898). That internal locking discipline looks intentional, but every host-supplied Sendable provider/tracer remains outside compiler proof. Tracing also uses @preconcurrency (Tracing.swift:65-86).

Two lifecycle notes matter when borrowing patterns:

1. keepSynced and subscription use detached/unstructured long-lived tasks (ElectricCollection+KeepSynced.swift:93-109; Collections.swift:1257-1268). Tokens and a cancellation relay cover normal teardown, including regression tests, but those tasks do not inherit caller cancellation or task-local context. Retain that design only with an explicit lifecycle owner.
2. liveBatchStream creates AsyncThrowingStream with the default unbounded continuation buffer (ElectricSyncClient.swift:2678-2759). The 50,000 cap bounds a no-boundary batch, not completed batches queued behind a slow database writer. A bounded policy would need a mandatory resync-on-overflow path: dropping ordered shape batches is incorrect.

The package has no reachability/sockets in source. Concrete provider behavior remains unverified: TLS, cancellation propagation to URLSession, network transitions, IPv6/VPN/low-signal behavior, and HTTP/SSE status semantics must be tested in the application. The provider contract documents ordered delivery but cannot enforce it (Providers.swift:352-358).

## Tests reviewed and coverage gaps

The suite is unusually strong for state-machine correctness. The largest suites are ElectricSyncClientTests (6,776 LOC), ElectricReplicaOwnerLifecycleTests (6,127 LOC), and MoveOutSemanticsTests (4,020 LOC). They cover cursor identity/adoption, atomic replacement, DNF recovery, cancellations, live/snapshot fencing, truncation, quarantine, ownership/tombstones, and GRDB transaction boundaries. Focused suites cover parser chunks, compiler output, coverage subtraction, dedupe, SSE fallback, circuit breaking, waiters, tracing, and stream GC.

Weak or absent coverage: a production URLSession/SSE decoder (none exists), hostile parser input and slow-consumer/backpressure behavior, persisted metadata schema migrations/old-field decoding, predicate/SQL fuzz or property tests, and end-to-end Shape-service compatibility.

## Circuits disposition

| Bucket | LOC | Share | Disposition |
|---|---:|---:|---|
| Portable utility semantics: predicate AST/compiler/mapper, SSE framing, runtime/tracing, backoff | 1,432 | 10.8% | Design reference; port only for a native Swift client. |
| Adapter-bound models/providers/fetch/session/protocol helpers | 2,380 | 18.0% | Redesign around Circuits identifiers/API and typed store transaction. |
| Electric lifecycle/runtime: collections, client, replica, DNF tags, stream/cursor ownership, legacy admission | 9,391 | 71.1% | Do not port; Circuits already owns a different server lifecycle. |

If Circuits needs Swift support, begin with a small HTTP/provider adapter against its Electric-compatible endpoint and add protocol-equivalence tests. Only then consider borrowing the portable predicate/SSE/backoff patterns; do not transplant the client-side local ownership and DNF-recovery runtime.

