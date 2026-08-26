# Swift collection framework for Electric Circuits

Status: **prototype core implemented; not a qualified production contract**

As-of date: 2026-08-26

This note defines provider-neutral terminology and a small Swift-facing collection model for using
Electric Circuits with a canonical local store. It does not amend the production-readiness task graph
in `18-production-readiness-spec-reviewed.md`.

## Decision

The application API should be organized around a stable, business-scoped **collection**, not around
individual server shapes, date windows, snapshot calls, or durable-stream subscriptions.

- A **collection** identifies one normalized local entity set, such as the current principal's calendar
  events or issues.
- A screen expresses a temporary **demand** over that collection: predicate, order and optional limit.
- The framework acquires a **lease** for the demand and internally creates or reuses one or more
  **materializations** that make the demanded rows locally available.
- **Coverage**, **row claims**, **snapshot fences** and **stream cursors** are internal correctness
  concepts. Feature code should not name or persist them.
- Domain rows remain in one canonical application table per entity type. The framework does not create
  one domain table per shape or demand.

For example, September, October, Today and an assignee-filtered calendar are demands over one
`calendarEvents` collection. They are not four collections and do not require four calendar-event
tables.

## What to take from TanStack DB

TanStack DB's useful public separation is:

1. Define a normalized, typed collection once.
2. Attach a source adapter, such as Electric, to populate it.
3. Let components issue live queries without knowing whether rows came from Electric, REST, SQLite or
   another source.
4. Route optimistic mutations through collection handlers and reconcile them with confirmed source
   events.

Its query-collection guidance makes a particularly important distinction: a collection represents a
business scope, while `where`, ordering and limiting describe a requested subset. Creating one
collection per filter or page is the wrong topology.

The current Electric adapter has eager, on-demand and progressive synchronization paths. Its on-demand
path compiles query demand into an Electric snapshot request; the progressive path can use a requested
subset as a fast path while a broader collection loads in the background.

Do not copy the present TanStack internals literally. An open TanStack RFC documents correctness and
lifecycle problems around subset de-duplication, overlapping query ownership and cleanup, and proposes
making coverage entries reference-counted resources. That vocabulary and separation are useful, but
the proposal is not yet a stable implementation contract.

Primary references:

- <https://tanstack.com/db/latest/docs/collections/electric-collection>
- <https://tanstack.com/db/latest/docs/collections/query-collection>
- <https://github.com/TanStack/db/blob/main/docs/overview.md>
- <https://github.com/TanStack/db/blob/main/packages/electric-db-collection/src/electric.ts>
- <https://github.com/TanStack/db/issues/1657>
- <https://github.com/TanStack/db/blob/main/packages/db-sqlite-persistence-core/src/persisted.ts>

## Terminology

| Term | Meaning | Public? |
| --- | --- | --- |
| **Collection** | Stable, normalized entity set with an ID, key function, decoder, source policy and store mapping. | Yes |
| **Demand** | Rows a consumer needs now: predicate, total order, optional limit and freshness policy. | Yes |
| **Lease** | A consumer's cancellable interest in satisfying a demand. Multiple consumers may share work. | Yes |
| **Load state** | Whether demanded data is unavailable, cached/stale, refreshing, live or failed. | Yes |
| **Sync mode** | Collection policy: `eager`, `onDemand` or `progressive`. | Yes |
| **Materialization** | One internal server-backed resource used to satisfy demand, such as a shape or a subset snapshot/feed pair. | Internal |
| **Coverage** | A proved statement that a completed materialization satisfies a demand. It is derived from materialization records, not necessarily a separate table. | Internal |
| **Row claim** | Association saying that a materialization currently includes a canonical row. Formerly described as membership/ownership. | Internal |
| **Snapshot fence** | Opaque server value that makes the snapshot-to-live handoff gap-free. | Internal |
| **Stream cursor** | Opaque durable resume position committed atomically with row changes. | Internal |
| **Generation** | Reset epoch that prevents stale materializations/cursors from being reused after an incompatible server or principal transition. | Internal |
| **Confirmation** | Token or receipt used to reconcile an optimistic mutation with authoritative source changes. | Mostly internal |

Use **row claim**, not `membership`, when describing the local many-to-many association. Membership is
easy to confuse with whether a row satisfies a SQL predicate. A claim answers the narrower storage
question: "which completed materialization currently justifies retaining this canonical row?"

Use **materialization cursor**, not a model-specific name such as
`calendarEventCircuitsCheckpoint`. A checkpoint is not one per domain table; it is one per live
materialization/generation.

## Public Swift model

The intended call-site shape is approximately:

```swift
let calendarEvents = CollectionDefinition<CalendarEvent, UUID>(
  id: "calendar-events",
  key: \.id,
  syncMode: .onDemand,
  source: circuits.calendarEvents,
  store: appStore.calendarEvents
)

let demand = CollectionDemand<CalendarEvent>(
  predicate: .inWindow(start: september1, endExclusive: september9),
  order: [.ascending(\.startAt), .ascending(\.id)],
  limit: nil
)

let lease = try await collections.acquire(calendarEvents, demand: demand)
// Read rows through the application's normal GRDB observation/query path.
// Observe lease.state to distinguish not-yet-loaded from genuinely empty.
```

A SwiftUI + local-store adapter should reduce this to:

```swift
@SyncedQuery(
  CalendarEvent.collection,
  where: .inWindow(start: start, endExclusive: end)
)
private var events: [CalendarEvent]
```

`@SyncedQuery` is an optional ergonomic facade, not part of the provider-neutral core. It composes two
responsibilities without conflating them:

1. compile the typed predicate into a `CollectionDemand`, acquire/reuse its materialization lease and
   expose availability through `$events.state`;
2. compile the same predicate through the store adapter and observe matching canonical rows, including
   cached rows and application-owned optimistic overlays.

The wrapper must not return rows straight from a Circuits response. GRDB remains the Indexed source of
truth for feature observation, relationships and sorting. Applications without GRDB use the same
collection/demand/coordinator contracts with another store adapter and may omit the wrapper entirely.
Changing `start` or `endExclusive` acquires the new demand before releasing the old lease, then switches
the local observation to the new predicate. Previously cached rows remain in the canonical table while
row claims justify them, but they do not appear in a non-matching query.

The projected value should expose only stable product state and actions:

```swift
$events.state       // unavailable | cached | refreshing | live | failed
$events.refresh()   // explicit retry/refresh for the current demand
```

It must not expose shape IDs, stream cursors, row-claim records or provider-specific predicates. The
first prototype slice does not implement this wrapper yet; it establishes the typed predicate and dual
renderer that the wrapper requires.

The next entity should require only:

1. a `CollectionDefinition`;
2. a typed demand/predicate mapping supported by the source adapter;
3. a store adapter that can decode and transact on that entity's canonical table;
4. optional mutation handlers.

It should not require new checkpoint, coverage or row-claim types or tables.

## Typed schema and predicate contract

Feature code must not choose a provider's column spelling. Each collection declares a typed field
registry once:

```swift
enum CalendarEventFields {
  static let startDateTime = CollectionField<CalendarEvent, String>(
    id: "startDateTime",       // stable, provider-neutral query identity
    sourceName: "start_datetime", // PostgreSQL / Circuits wire column
    storageName: "startDateTime"  // canonical local-store / ElectricSync field
  )
}
```

The three names have distinct contracts:

| Name | Used for | Stability rule |
| --- | --- | --- |
| `id` | canonical demand identity, diagnostics and future persisted query fingerprints | Stable across source/store adapters; changing it invalidates compatible cached demand identity |
| `sourceName` | Circuits `Predicate` and native API payloads | Must exactly match the admitted Postgres publication schema |
| `storageName` | local query/store adapter and legacy ElectricSync predicate | Must exactly match the model's canonical local column mapping |

`CollectionField<Model, Value>` uses `Model` as a phantom collection type and `Value` as the permitted
comparison value. Consequently a calendar-event field cannot be combined with an issue predicate, and
a UUID field cannot be compared with a date string. Nullability is expressed explicitly through
`.isNull` and `.isNotNull`; comparison with an optional `nil` is not part of the grammar.

The v1 predicate AST is intentionally closed and small:

```text
comparison(field, = | < | <= | > | >=, scalar)
isNull(field, true | false)
and([predicate])
or([predicate])
not(predicate)

scalar = string | integer | double | boolean
```

UUID values normalize to lowercase strings. String identity escapes a single quote by doubling it.
Adjacent `AND` nodes and adjacent `OR` nodes flatten while preserving operand order. Canonical identity
uses only logical field IDs and normalized values; it never contains source or storage names. Boolean
nodes retain explicit grouping so distinct trees cannot collapse to the same demand identity. Provider
rendering follows this fixed table:

| AST node | Circuits renderer | ElectricSync/local renderer |
| --- | --- | --- |
| comparison | `Predicate.leaf(column: sourceName, ...)` | `SyncPredicateExpression.comparison(field: storageName, ...)` |
| is null | `Predicate.isNull(column: sourceName, isNull: true)` | `storageName = NULL` |
| is not null | `Predicate.isNull(column: sourceName, isNull: false)` | `NOT (storageName = NULL)` |
| boolean composition | structurally identical `and`/`or`/`not` tree | structurally identical `and`/`or`/`not` tree |

Provider-specific operations that cannot be represented faithfully by every selected adapter must fail
at definition/admission time. The core must not silently approximate, drop or push a different
predicate into only one side.

Domain predicates hide schema complexity while remaining inspectable:

```swift
extension CollectionPredicate where Model == CalendarEvent {
  static func inWindow(start: DateOnly, endExclusive: DateOnly) -> Self
}
```

The calendar implementation includes bounded recurring exceptions, active recurring masters, all-day
overlap and timed overlap. Both the Circuits request and the current ElectricSync subset request must
be compiled from this one function. Adding another calendar screen composes or reuses that predicate;
it does not reproduce raw column strings.

Macros or generated schema declarations may later produce field registries, but runtime reflection is
not part of the design. An explicit registry is debuggable, supports different source/store spellings
and makes schema drift a build/test failure instead of a dynamic lookup failure.

### Typed-predicate acceptance tests

1. One AST with deliberately different source and storage names renders the exact expected tree for
   both adapters.
2. Canonical identity contains the logical IDs and contains neither provider spelling.
3. UUID, string, integer, double and boolean values render identically in semantic value on both sides.
4. `.isNull` and `.isNotNull` preserve their truth table in both adapters.
5. Mixed-model fields and wrong value types fail to compile (retain small compile-fail fixtures once the
   type is extracted into the reusable package).
6. The calendar window renders snake_case Circuits JSON and camelCase local/ElectricSync fields from
   the same domain predicate.
7. Existing calendar overlap/recurrence tests remain unchanged and green after their implementation is
   switched to the typed domain predicate.
8. The local Circuits prototype's base principal scope is created from the typed `userID` field, with
   no raw `"user_id"` at the call site.

### `@SyncedQuery` acceptance tests

1. Cached matching rows are emitted synchronously before network materialization starts, with state
   `.cached` or `.refreshing` rather than a false empty/loading result.
2. An uncached demand exposes `.unavailable`/`.refreshing`, then emits exactly the rows committed by the
   store transaction before transitioning to `.live`.
3. Two wrappers with the same canonical demand share one materialization and independently release
   their leases.
4. Changing a window acquires the replacement demand before releasing the old one; late events from the
   prior generation cannot advance or populate the replacement.
5. Source and local evaluation of the same predicate agree for nulls, interval boundaries and ordering;
   disagreement fails the test rather than widening either side.
6. Snapshot rows, row claims and the snapshot fence commit atomically; killing the process at every
   transaction cut leaves either the old state or the complete new state.
7. A live batch, optimistic reconciliation and stream cursor commit atomically; replay after each injected
   crash is idempotent.
8. Account/principal changes cancel observations and leases, purge private rows and metadata, and never
   render the prior principal's cached rows.

`ElectricCircuitsCollections` now owns the typed AST, demand identity, exact-demand coordinator,
lease lifecycle, Circuits subset source and in-memory reference store. Indexed owns the first GRDB
provider and its Calendar Today/Full Day integration. This remains a validation vehicle: a second
entity, account-generation reset and a SwiftUI query wrapper are still required before treating the
package boundary as stable.

## Framework layers

```text
Feature / SwiftUI
  |-- local GRDB query and observation
  `-- CollectionDefinition + CollectionDemand
             |
             v
      CollectionCoordinator
      |-- demand leases and load state
      |-- exact-demand de-duplication
      |-- bounded coverage proofs
      `-- materialization lifecycle
             |
       +-----+--------------------+
       |                          |
       v                          v
  CircuitsSourceAdapter      CollectionStore
  |-- choose shape/subset    |-- canonical domain rows
  |-- snapshot + fence       |-- materialization records
  |-- live stream + cursor   |-- row claims
  `-- mutation confirmation  `-- optimistic overlays/receipts
```

`ElectricCircuitsSwift` should retain low-level transport and stream primitives. The general framework
can be a separate target layered on it, for example `ElectricCircuitsCollections`, so applications
that need only direct protocol access do not inherit collection policy.

The core framework remains storage-neutral. It defines `CollectionStore`; an in-memory provider
supports tests and demos. Indexed owns its GRDB provider and schema. A reusable GRDB implementation can
later live in an optional target, but GRDB must not become a dependency of the core transport or
collection module.

## Generic persistence topology

The application still has one canonical domain table per model:

```text
calendarEvent
issue
project
...
```

All collection types share the same framework metadata tables:

```text
_circuits_collection
  collection_id
  principal_scope
  schema_version
  generation

_circuits_materialization
  materialization_id
  collection_id
  canonical_demand
  demand_hash
  kind                    -- shape | subset
  state                   -- loading | cached | live | stale | failed
  generation
  snapshot_fence
  server_resource_id      -- opaque shape/feed/subscription identity
  applied_stream_cursor
  applied_source_version  -- diagnostic/ordering value, not resume authority
  last_refreshed_at

_circuits_row_claim
  materialization_id
  row_key
  source_version

_circuits_optimistic_mutation   -- optional if the app already has a generic overlay log
  mutation_id
  collection_id
  row_key
  operation
  payload
  confirmation
  state
```

`_circuits_collection` may be configuration-derived rather than persisted if the provider can still
perform account purge, schema migration and generation reset safely. `Coverage` does not need its own
table initially: a live or completed `_circuits_materialization` row plus its canonical demand is the
coverage fact.

The current proposed model-specific record:

```text
calendarEventMaterialization(...)
```

becomes one row in `_circuits_materialization` with `collection_id = "calendar-events"`. Its checkpoint
columns live on that row because they belong to that materialization. September and October produce two
rows only when they cannot safely reuse an existing materialization; both write into `calendarEvent`.

## Atomic store contract

The store must offer one transaction boundary that can:

1. compare the expected materialization generation and cursor;
2. apply canonical row upserts/deletes using authoritative source ordering;
3. replace or incrementally update this materialization's row claims;
4. delete a canonical row only when no materialization claims it and no optimistic overlay retains it;
5. record the snapshot fence or advance the stream cursor;
6. reconcile matching optimistic mutations;
7. commit all effects together or commit none.

This extends the existing `ShapeMaterializer.apply(batch, expecting:, advancingTo:)` invariant from
"rows plus cursor" to "rows, claims, cursor and optimistic reconciliation." The exact GRDB transaction
is an application/provider implementation detail.

Snapshot replacement must be scoped to one materialization. If an October refresh omits a row, it may
remove October's claim; it may not delete a row still claimed by September, Today or another assignee
view. Failed or cancelled loads do not replace prior successful coverage with an empty result.

A live shape `delete` means "this key is no longer a member of this materialization"; it does not say
whether the source row was deleted, moved outside the predicate or left the authorization scope. This
matters when an inactive overlapping materialization still claims the old canonical row: merely
releasing the active claim can leave stale field values that still satisfy the application's direct
GRDB predicate. Production observation must close that gap in one of two proved ways:

1. enrich/resolve the leave against the collection's base authorization scope, atomically updating
   the canonical row when it still exists and removing every claim when it no longer exists; or
2. make active-view observation join the demanded materialization's current row claims instead of
   treating every retained canonical row as active membership.

The current prototype does neither yet. Row-claim retention is therefore cache/storage evidence, not
by itself a proof that direct canonical-table predicates remain exact across inactive overlapping
windows.

## Demand and coverage rules

Do not build a general predicate theorem prover or import TanStack's internal DNF machinery for v1.
Start with conservative, explicitly supported proofs:

- exact canonical demand identity;
- a complete/eager collection covers every demand within the collection's base authorization scope;
- an interval covers contained intervals with the same base predicate;
- an exact key set covers subsets of those keys;
- an ordered prefix of `N` covers the same predicate/order with limit `M <= N`;
- otherwise, load another materialization.

Canonical demand identity must include the authorized base scope, predicate, total ordering (including
a unique tie-break), limit, principal and generation. Equivalent demands may later be normalized, but
an uncertain equivalence must cause a redundant fetch rather than false coverage.

A live feed alone never proves ordered/limited coverage. For a top-N demand, the materialization stays
live only if the client or server maintains the ordered window, including refill when an unchanged row
crosses the boundary. The existing snapshot-plus-feed-and-refresh approach is a valid simple
materialization strategy only when it performs that bounded refresh. The current
`CircuitsSubsetSource` therefore rejects limited live demands before creating a server resource; use
the one-shot subset API for a limited snapshot until a top-N-maintaining adapter lands.

## Lifecycle semantics

- The first lease for an uncovered demand starts a load.
- Concurrent equivalent demands share the in-flight load and materialization.
- A lease reports cached rows immediately, but distinguishes `.cached`/`.refreshing` from `.live`.
- Releasing the last lease may stop the remote subscription without deleting canonical rows.
- Retained materializations become stale according to policy; stale coverage can render immediately but
  triggers refresh when reacquired.
- Account change, authorization-scope change or incompatible generation invalidates the affected
  materializations and performs a transactional private-data purge/reset.
- Cancellation of a load never converts the last successful materialization into a successful empty
  result.

## Source-adapter contract

The Circuits adapter should hide whether it selected:

- a complete live shape;
- a bounded subset snapshot plus changes feed;
- a fast subset followed by broader progressive synchronization; or
- a future server-native ordered materialization.

It returns an internal materialization plan with an opaque snapshot fence, server resource identity and
cursor protocol. Feature code sees only demand load state.

The existing Circuits subset endpoint's LSN watermark must not be promoted to a generic production-safe
snapshot fence without proving the exact snapshot/live visibility contract. The framework API should use
an opaque `SnapshotFence` now so a stronger server fence can replace the current representation without
changing feature APIs or persistent domain tables.

## Optimistic mutations

A collection may provide `onInsert`, `onUpdate` and `onDelete` handlers. A mutation:

1. writes an optimistic overlay keyed by the stable UUIDv4 client ID;
2. sends through the application's authenticated/custom HTTP client;
3. receives a confirmation token or causal receipt from the write API;
4. retains the overlay until the authoritative stream confirms or rejects it;
5. reconciles confirmation in the same store transaction as the authoritative row event.

This preserves Indexed's generic per-table overlay model. The collection framework coordinates it; it
does not require a second optimistic copy of each domain table.

## Concrete calendar example

```text
calendarEvents collection
  |
  |-- lease: Today
  |     demand = [todayStart, tomorrowStart)
  |
  |-- lease: September 1-8
  |     demand = [Sep 1, Sep 9)
  |
  `-- lease: October 1-8 assigned to Alice
        demand = [Oct 1, Oct 9) AND assignee = Alice

Internal state
  _circuits_materialization: up to three active/cached records
  _circuits_row_claim: associations from those records to row IDs
  calendarEvent: one de-duplicated canonical row per event ID
```

Revisiting September reads `calendarEvent` immediately. The coordinator finds the retained September
materialization, reports cached state, and either renews its live resource or refreshes it according to
policy. No calendar-specific checkpoint or coverage code runs.

## TDD acceptance sequence

The framework should be built from public high-level contracts in this order:

1. One collection and one demand: cold snapshot becomes a genuine empty or populated ready state.
2. Two identical concurrent demands: one remote load, two independent leases.
3. Two disjoint windows: one canonical domain table, two materializations, correct union of rows.
4. Two overlapping windows: one canonical row, two row claims; releasing or refreshing one cannot
   remove the other's row.
5. Overlapping-window move-out: keep one overlap inactive, move a row out of the active predicate,
   and prove the active local query stops returning it before its applied cursor advances; cover both
   surviving-base-row and actual-delete/authorization-loss outcomes.
6. Cancelled/failed refresh: prior successful rows and coverage survive; cursor does not advance.
7. Atomic crash/replay: rows, claims and cursor advance together and replay idempotently.
8. Cached revisit: cached rows render first, then refresh/renew reaches live state.
9. Account switch/generation reset: old private rows, claims, cursors and optimistic overlays cannot
   leak into the new scope.
10. Ordered top-N: an insertion, update, deletion or move across the boundary produces the same final
   local result as an independent PostgreSQL query, including refill.
11. Optimistic UUIDv4 insert: overlay appears immediately and reconciles exactly once with the
    authoritative stream event.
12. Store portability: run the same coordinator contracts against in-memory and GRDB providers.
13. Entity portability: register `issues` after `calendarEvents` without adding framework schema or
    lifecycle code.

These tests observe collection demand, local rows and public load state. They should not assert private
shape IDs, retry counts, task structure or log text.

## Relationship to current code

- Indexed's `ElectricCollectionDemand` and session-scoped `ElectricCollectionRegistry` already have the
  right public direction: stable registered collection ownership plus query-level predicate/order/limit.
- Indexed's generic `ElectricFetchMetadata` and `ElectricShapeRowOwnershipRecord` already embody much of
  materialization metadata and row claims. Migration should rename/consolidate their concepts rather
  than recreate calendar-specific tables.
- `ElectricCircuitsSwift.MaterializationScope`, `StreamCursor`, `ShapeMaterializer` and
  `ShapeSubscriptionCoordinator` remain low-level primitives. The separate
  `ElectricCircuitsCollections` product now supplies the higher-level collection/demand/lease layer.
- The TypeScript Circuits client currently exposes a separate TanStack collection for each shape or
  subset call. It is a protocol reference, not yet the desired stable business-scoped collection
  facade.

## Implemented prototype boundary

The provider-neutral `ElectricCircuitsCollections` layer now contains:

- `CollectionDefinition<Model, Key>`;
- `CollectionDemand<Model>`;
- `CollectionLease` and public load state;
- `CollectionCoordinator` and demand de-duplication/lifecycle;
- `CollectionSourceAdapter` implemented by Circuits;
- `CollectionStore` with the atomic contract above;
- an in-memory reference store and reusable contract test suite.

Keep Indexed's GRDB store adapter, canonical tables and optimistic overlay integration in Indexed until
the contract stabilizes. The result should feel like TanStack DB's Electric integration at the call
site while retaining stronger offline, overlap and crash-safety semantics for a durable iOS cache.
