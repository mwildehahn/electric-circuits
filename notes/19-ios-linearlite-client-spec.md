# iOS LinearLite client and Electric Circuits integration spec

Status: proposed  
Date: 2026-08-23

## Goal

Build a SwiftUI iOS frontend for the existing LinearLite example. The app keeps a GRDB-backed local
read cache, reads that cache while offline, and stays synchronized with Electric Circuits when the
network is available.

The first milestone is a working vertical slice, not a complete generic sync framework:

```text
Postgres 18 (system of record)
        │
        ├── application write API ── iOS app
        │                             │
        │                         GRDB cache
        │                             │
        └── logical replication → Electric Circuits → durable shape streams
                                                    │
                                                    └── iOS sync worker
```

The app must prove the important loop:

```text
SQL write → replication → circuit shape → durable stream → HTTP client → GRDB transaction → SwiftUI
```

## Library decision

### Recommendation: create a new native library

Create a new Swift package, tentatively named `ElectricCircuitsSync`, in a sibling repository
(`electric-circuits-swift`) or a clearly isolated `ios/` package. Keep the iOS LinearLite app under
`examples/linearlite-ios` in this repository so its E2E contract is versioned beside the server.

Do not make `electric-sync-swift` the implementation target for this integration.

`electric-sync-swift` is valuable prior art and may remain the compatibility client for deprecated
ElectricSQL deployments, but it is the wrong abstraction boundary here:

- it speaks the Electric `/v1/shape` protocol, while the fork's accepted product surface is the
  native shape control plane plus durable streams (`docs/adr/0001-fork-scope-native-path.md`);
- it carries DNF topology, row ownership, move-out tombstones, protocol epochs, and legacy cursor
  migration because it supports overlapping Electric collections and historical Electric wire
  behavior;
- its storage and HTTP layers are provider protocols; GRDB is only used by its tests, so the app
  would still need a substantial adapter;
- changing it to understand native State-Protocol envelopes risks breaking the existing ElectricSQL
  compatibility surface and makes the simpler native contract harder to reason about.

The new package should reuse ideas and small protocol-safe utilities only after the vertical slice is
working. Do not copy the DNF/ownership state machine into the new package.

### Explicit simplification for milestone 1

The app has one canonical server shape per local replica table and performs presentation filtering,
sorting, and search in GRDB. This avoids overlapping shape deletes and therefore avoids the DNF row-
ownership problem entirely.

Initial replicas:

- `issues`: visible to the selected user through the existing `project_id IN (project_members …)`
  subquery; sync all columns needed by list, board, search, and detail.
- `projects`: projects visible to the selected user.
- `project_members`: the selected user's memberships.
- `users`: roster needed for labels/assignee display.
- `comments`: either all demo comments for the first slice or a single bounded visibility shape;
  do not create overlapping per-issue comment shapes in milestone 1.

The selected user is a session configuration. Changing users closes the old claims, replaces the
affected local working set atomically, and creates the new shapes. Multi-account simultaneous caches
are out of scope for the first slice.

## Server contract required by the app

### Native JSON control API

The current API is exposed as tRPC for TypeScript. A Swift client should not implement tRPC batching
as its public contract. Add a small REST/JSON facade over the existing `ElectricCore` with stable
versioned routes (the exact prefix can be chosen in implementation):

| Operation | Required semantics |
|---|---|
| Create/renew shape | Accept table, predicate AST, columns, and a caller-owned subscription id. Repeating the same subscription is idempotent and may return a replacement handle after retention eviction. |
| Delete shape | Accept shape id + subscription id. Repeating it is a no-op. |
| Query shape metadata | Optional for diagnostics; not needed to render the app. |
| One-shot subset query | Add in milestone 2, when the app needs server-backed paging. |
| Live subset feed | Add in milestone 2. |
| Aggregate | Add in milestone 2. |
| Application write | Execute parameterized SQL against Postgres, not the engine's library-mode change-log append. Return a request/transaction id and a typed result. |

The demo's Vite-only `/pg/write` middleware is not a production contract. The iOS app needs an
authenticated server endpoint that validates table/column/operation permissions and writes to
Postgres, preserving Postgres as the source of truth.

### Durable stream read contract

The native client reads the returned durable stream directly:

```http
GET {streamUrl}?offset={lastOffset}&live=long-poll
Accept: application/json
Authorization: Bearer …
```

The response is a JSON array of State-Protocol envelopes:

```json
{
  "type": "public.issues",
  "key": "42",
  "value": { "id": 42, "title": "…" },
  "headers": {
    "operation": "upsert",
    "txid": "…",
    "lsn": "0/16B6A20",
    "offset": "0000000000000000_0000000000000042"
  }
}
```

The client advances its offset from `stream-next-offset`, never from a guessed array index. A 204
with no body is a successful idle poll. `stream-closed`, 404, and 410 are terminal for that feed and
trigger the replacement/reseed path. Transient 5xx, timeout, DNS, and cancellation errors retry with
bounded backoff.

The client should use long-poll first. SSE is an optimization for a later milestone; it is not needed
to prove correctness.

## `ElectricCircuitsSync` package design

### Public API shape

```swift
let client = CircuitsClient(
  configuration: .init(
    controlURL: URL(string: "https://api.example.test/v1")!,
    streamsURL: URL(string: "https://streams.example.test")!,
    auth: .bearer(tokenProvider),
    database: databaseQueue
  )
)

let session = try await client.openLinearLiteSession(userID: userID)
await session.start()
```

The public surface should be small:

- `CircuitsClient`: owns URLSession, authentication, retry policy, and the GRDB database handle;
- `SyncSession`: an actor coordinating replica lifecycles for one selected user;
- `ShapeReplica`: one server shape + one local table working set;
- `ShapeDefinition`: table, predicate AST, selected columns, and a stable local replica key;
- `ChangeEnvelope`: Codable native State-Protocol envelope;
- `SyncStatus`: idle, syncing, offline, recovering, failed with typed reason;
- `SyncError`: validation, auth, transport, stream-gone, schema, decode, database, and terminal
  server errors;
- `TableCodec<Model>`: app-provided typed decode/apply hooks for GRDB records.

The package should not expose the server's predicate implementation as SQL strings. Define the native
predicate AST in Swift (`eq`, `neq`, `lt`, `lte`, `gt`, `gte`, `isNull`, `and`, `or`, `not`, `in`) and
encode the same JSON shape used by `packages/protocol/src/types.ts`.

### GRDB schema

The app owns its normal tables and migrations through `DatabaseMigrator`. The sync package owns only
metadata tables, for example:

```sql
CREATE TABLE circuits_replica (
  replica_key TEXT PRIMARY KEY,
  table_name TEXT NOT NULL,
  definition_json BLOB NOT NULL,
  shape_id TEXT,
  stream_path TEXT,
  subscription TEXT NOT NULL,
  offset TEXT NOT NULL,
  status TEXT NOT NULL,
  updated_at REAL NOT NULL
);

CREATE INDEX circuits_replica_table ON circuits_replica(table_name);
```

The app may add a request/idempotency table if the write API needs durable retry keys. The shape
offset, handle, and row mutations must be committed in one GRDB write transaction. A crash can then
only produce either the previous cache + offset or the new cache + offset; it must never checkpoint an
offset before its envelopes are applied.

Typed app models should use `FetchableRecord`/`PersistableRecord` (or equivalent codecs), with
`Int64` for Postgres `int` values, `Double` for `float`, and explicit nullable columns. Do not use
untyped JSON dictionaries as the long-term app data model.

### Replica state machine

```text
stopped
  └─ start → loadingMetadata
                ├─ same handle/stream → replay from saved offset
                ├─ replacement handle → atomically reseed table, then live tail
                └─ no handle → create + snapshot + live tail

liveTail
  ├─ 204 → poll again
  ├─ envelopes → apply entire response transactionally, checkpoint offset
  ├─ network error → offline/backoff, retain cache and offset
  ├─ 404/410/stream-closed → replacement snapshot
  └─ auth/schema/decode/database error → failed with actionable status
```

For a new or replacement stream, fold all envelopes from offset `-1` into a temporary/transactional
working set, then atomically replace the app table and metadata. Never clear the visible table before
the replacement snapshot is available.

For a resumed stream, apply all envelopes returned by one poll in one SQLite transaction. `upsert`
replaces the row by primary key; `delete` removes it idempotently. The native stream's absolute
emission means the client does not infer move-in/move-out from local predicates.

### Lifecycle and iOS behavior

- Start sync when the app enters the foreground or the selected workspace becomes active.
- Keep the local GRDB cache readable when suspended or offline.
- Renew each named subscription on the server cadence while active; explicitly renew on foreground
  resume. Do not rely on timers running while suspended.
- Cancel the long-poll task on background/stop, but persist the last committed offset first.
- Use `URLSession` with structured concurrency. The sync coordinator is an actor; database writes are
  serialized through GRDB's writer and never performed on `MainActor`.
- UI observes GRDB (`ValueObservation`/an app observation layer), not an in-memory array owned by the
  network task.
- Authentication is injected and refreshable. A 401 pauses sync until credentials are refreshed;
  it must not spin in an infinite retry loop.

## LinearLite iOS MVP

### Screens

1. Workspace/user selector.
2. Issue list with local SQL filtering, sorting, and search.
3. Board grouped by status, backed entirely by GRDB queries.
4. Issue detail with edit status, priority, title, assignee, and project.
5. Comments if the comments replica is included in the initial seed.
6. Sync status/debug screen showing last successful offset, last error, and replica state.

### Write policy

Milestone 1 is network-first:

1. User action submits a typed write to the application API.
2. The API commits Postgres.
3. The replication/circuit/stream path delivers the resulting envelope.
4. GRDB changes and UI update from the stream.

Do not add optimistic overlays until the non-optimistic path is proven. This keeps conflict semantics
out of the first integration and makes the E2E oracle unambiguous.

## High-level E2E/TDD contracts

These should be written before the implementation and run against the real LinearLite stack plus a
real Postgres 18 instance where possible.

1. **Cold install:** seed Postgres, start the app session, and assert local rows equal the Postgres
   visibility query for the selected user.
2. **Offline launch:** stop network services, relaunch the app, and assert the last committed GRDB
   cache remains queryable with no data loss.
3. **Live insert:** insert an issue into a visible project; assert it appears locally exactly once.
4. **Live update:** update title/status/priority; assert the local row changes without a duplicate.
5. **Live delete:** delete an issue; assert it disappears locally and remains absent after restart.
6. **Visibility enter/leave:** add/remove a user's project membership; assert all affected issues enter
   or leave the local cache.
7. **Move across projects:** move an issue between visible and invisible projects; assert the correct
   delete/upsert behavior.
8. **Reconnect catch-up:** write while the app is offline, reconnect from its saved offset, and assert
   convergence with Postgres with no duplicate rows.
9. **Crash boundary:** terminate between stream receipt and the next launch; assert the last complete
   GRDB transaction is replay-safe and no offset is ahead of applied rows.
10. **Replacement stream:** force a stream gone/retention replacement; assert atomic reseed and exact
    convergence, with no stale rows visible after completion.
11. **Subscription renewal:** advance/shorten the lease, suspend/resume the app, renew, and assert the
    active replica remains usable.
12. **Write round trip:** edit an issue in the app and assert Postgres, the server stream, GRDB, and
    the rendered screen converge to the same row.
13. **Transaction batch:** apply a multi-row transaction and assert the UI never observes a partially
    applied poll response.
14. **Local migration:** open a database from the previous app schema, run `DatabaseMigrator`, and
    assert sync metadata and user data survive.

## Implementation packets

Each packet follows the repository workflow: isolated worktree, genuine-red test first, worker writes
`DONE.md`, reviewer writes `REVIEWED.md`, and only the reviewed source/tests are integrated.

- **IOS-000:** freeze the REST/native wire contract and add server integration tests for create/renew,
  delete, stream handles, and Postgres application writes.
- **IOS-001:** scaffold `ElectricCircuitsSync` with Codable AST/envelopes, URLSession control client,
  typed errors, and deterministic fake transport tests.
- **IOS-002:** implement GRDB metadata migrations and transactional envelope application with a typed
  LinearLite codec.
- **IOS-003:** implement one shape replica with long-poll resume, offsets, retries, cancellation,
  and stream replacement; cover crash/reconnect tests.
- **IOS-004:** add the LinearLite Postgres schema/seed endpoint and `examples/linearlite-ios` SwiftUI
  shell with local-only read views.
- **IOS-005:** connect the app to real shapes and writes; add the cold-install/live mutation/visibility
  E2E suite.
- **IOS-006:** add foreground renewal, offline status, auth refresh, and replacement-stream E2E cases.
- **IOS-007 (later):** add native subset paging and aggregate subscriptions only after the single-shape
  path is stable.
- **IOS-008 (later):** evaluate optimistic writes and optional SSE after measuring the long-poll path.

## Non-goals for the first integration

- Supporting the deprecated ElectricSQL `/v1/shape` protocol in the new package.
- Reimplementing DNF topology, row ownership, move-out tombstones, or protocol capability epochs.
- Generic multi-shape overlap on one local table.
- Optimistic writes and conflict resolution.
- SSE, push notifications, background fetch guarantees, widgets, or multi-account simultaneous sync.
- Proving every unrelated engine hardening item before the app vertical slice works.
