# TypeScript client reference for a Swift port

Behavior-first reference for porting packages/client to Swift. It records exact TypeScript client algorithms, with tRPC, TanStack DB, browser, and JavaScript glue separated from correctness work. Sources: every client source/test/README, protocol, API core/router, engine HTTP/query/output, and client-facing architecture.

## Scope

| API | Server object | Initial state | Direct-stream state |
| --- | --- | --- | --- |
| shape(def) | shared normal shape | engine backfill in stream | keyed live row map |
| subset(def) | shared changesOnly feed | one-shot Postgres page | bounded page map + LSN watermarks |
| aggregate(def) | shared aggregate shape | first aggregate envelope | latest {value,n} |

The engine owns rows, predicates, backfills, sharing, retention, and stream creation. The client owns direct durable-stream reads, local materialization, named lease renewal/release, replacement rebinding, and subset page/feed reconciliation.

## Wire contract

### Control plane

The public TS client uses tRPC, but that is transport glue. The API core forwards to engine routes:

    POST /shapes
      { table, where: Predicate? | null, columns: [String]? | null,
        changesOnly: Bool = false, subscription: String? }
      -> ShapeHandle

    POST /aggregate
      { table, where: Predicate? | null, fn: count|sum|avg|min|max,
        col: String? | null, subscription: String? }
      -> ShapeHandle

    POST /query
      { table, where, columns, orderBy: { col, desc }?, limit, offset }
      -> { rows: [Row], lsn: "HI/LO" }

    DELETE /shapes/{shapeId}?subscription={percent-encoded-id}
      -> { ok: true }       // HTTP 404 means already released

Create response JSON uses camel case:

    {
      "shapeId": "s17",
      "table": "public.issues",
      "streamPath": "shape/s17",
      "streamUrl": "http://durable-streams/shape/s17",
      "subscription": "caller-uuid",
      "leaseSeconds": 60
    }

The create response is authoritative. subscription and leaseSeconds are omitted by GET /shapes/{id}, which is metadata, not a claim.

### Stream envelope, offsets, types

    struct StreamEnvelope: Decodable, Sendable {
      let type: String       // canonical schema.table
      let key: String        // opaque primary-key encoding
      let value: Row?
      let headers: Headers
    }
    struct Headers: Decodable, Sendable {
      let operation: String  // insert | update | upsert | delete
      let txid: String?
      let offset: String?
      let lsn: String?       // PostgreSQL commit LSN, HI/LO hex
      let seq: Int?
      let last: Bool?
    }

Row streams use upsert/delete behavior. Treat insert, update, and upsert identically: replace local value at envelope.key; delete removes that key. Aggregate streams use key "agg" and value { "value": AggregateValue, "n": Int }; type remains the canonical table.

The durable-stream transport required by this client is:

- HEAD exposes stream-next-offset, an opaque cursor. "-1" is the reference fallback/origin cursor.
- A direct HTTP stream supports long-poll or SSE and decodes an async sequence of envelope JSON.
- Stream closure, 404, and 410 are observable terminal conditions.

Never compare stream offsets, turn them into integers, or confuse them with LSNs. The client only captures an offset to choose where a subset tail begins.

Value is number | string | bool | null, schema-directed. An int beyond ±(2^53-1) is an exact decimal JSON string. Swift must not decode all numbers as Double: use a lossless enum, Int64 plus decimal text, or arbitrary precision. For ordering, parse int in either wire form numerically. Local map identity is supplied opaque envelope key, never String(row[primaryKey]). This protects the engine composite-key encoding (backslash and U+001F escaped, then components joined with U+001F).

Canonical table name is schema.name; bare name means public.name only. Reject invalid references (empty part, quote, extra dot). Canonicalize local schema once and reject issues plus public.issues as a collision. Stream type is always canonical even if caller used bare spelling.

## Identified subscription + lease

Each materialization mints one UUID and sends it on initial create, every renewal, and DELETE.

- Repeated create with same id on same shared shape is renewal: same claim, no second reference.
- Same id belonging to another shape is server 409.
- Delete with id releases exactly that claim and is idempotent.
- Delete without it is legacy anonymous decrement and is not retry-safe. Do not use it.
- Equal definitions share engine state. Client must not recreate engine signature canonicalization.

Lease cadence is server supplied: when leaseSeconds > 0, renew every clamp(leaseSeconds/3, 0.25 seconds...300 seconds). When zero, do not schedule a timer, but manual renewal remains valid.

Critical close ordering:

    mark lease keeper stopped
    cancel timer
    await every renewal accepted before stop (ignore its error for close)
    cancel readers
    DELETE last returned/claimed shape handle with subscription UUID

A renewal is a create. If it completes after DELETE it reclaims the subscription and leaks it. Therefore renewal calls need one serial chain, not detached parallel tasks. TS keeps claimedHandle advanced by every create result, even if it is the same stream, and releases that final handle.

Swift actor sketch:

    actor MaterializationState {
      var stopped = false
      var renewalTail: Task<Void, Never> = Task {}
      var claimedHandle: ShapeHandle
      // Serialize attempts. Store a failure-swallowing tail so one bad
      // renewal never prevents the next; explicit renew observes its own error.
      // stop marks stopped, cancels ticker, awaits renewalTail, then DELETEs.
    }

Do not hold a lock across await. Swift actors are reentrant, so after a network await use a generation/token before publishing a replacement reader/state.

TS delete is best effort: at most five attempts, waits 200/400/800/1600 ms, accepts 404/not-found as success, warns then resolves after final failure. That can leave the claim until lease expiry. A Swift API should document this exactly or report/persist failure, but must use the named claim on all retries.

TS tracks all materializations. Every close is one-shot: concurrent calls await same close task and completion removes it from client-wide open state. client.close snapshots and closes open items. Use an actor registry.

## shape()

State machine:

    Creating -> Active(handle, reader, rowMap, listeners, lease)
                   | same shapeId + streamPath renewal -> Active
                   | different handle renewal
                   v
               Rebinding (new reader fully preloaded) -> Active
                   | close
                   v
                Closing -> Closed

Algorithm:

    sub = UUID()
    handle = create normal shape(def, sub)
    open direct stream for handle
    preload all currently stored stream data
    apply each table-type envelope to rowMap by envelope.key
    start live reader and LeaseKeeper
    return materialization

Normal listeners receive future change batches only. On replacement, create and fully preload a new reader/map before atomically publishing it, then cancel/close old reader. Existing listeners are reattached with initial-state inserts from new map. Callers must not cache old collection/snapshot references across replacement.

External durable-stream state DB supplies TS snapshot preload, collection transactions, validation, and awaitTxId. Essential Swift equivalents:

- shape returns only after snapshot preload;
- stable keyed upsert/delete view plus AsyncSequence or callback changes;
- awaitTxId completes after consuming an envelope bearing txid;
- listener/replacement behavior above.

Exact external-package awaitTxId transaction semantics are not visible here; see open contracts.

## aggregate()

Create with separate UUID, then read stream URL at offset "-1". TS default is long-poll for aggregate/subset (liveMode true also maps to long-poll here). For a current-reader envelope with value containing value:

    current = envelope.value.value ?? null
    count = envelope.value.n ?? 0
    notify subscribers(current)

Initial state is value == null, count == 0; adding a subscriber does not invoke it immediately. n counts matching rows for every function. Empty SUM/AVG/MIN/MAX is null. Integer SUM can be decimal text; float SUM and AVG are numbers; MIN/MAX retain column type.

Each reader needs cancellation plus monotonic generation. Buffered output after cancellation must be discarded unless generation is current. Renewal replacement aborts old reader, increments generation, starts new. Close uses same stop/drain/release rule as shape.

## subset(): required correctness algorithm

### Window and comparison

Resolve local schema. If projection is requested, force include PK and order column. Default limit is 100. Query ordering is requested orderBy plus PK in same direction; with limit/offset but no orderBy engine uses PK ascending.

Comparator is schema-directed:

- null or missing projection sorts last ascending, therefore first descending;
- int uses arbitrary precision numeric comparison across number/text wire forms;
- float uses numeric comparison;
- text uses Unicode code-point/scalar order, never UTF-16 code-unit comparison or locale order;
- PK is total-order tiebreaker in same direction.

Postgres uses COLLATE "C" for known collatable introspected text columns. Tables only declared through /schema may retain database collation because native type is unknown: TS cannot fully solve that mismatch.

State:

    boundary: final currently loaded row in sort order, optional
    lower: first initial row only when initial offset > 0
    ended: Bool
    present: Set<opaque envelope key>
    applied: Map<key, LSN>       // transient absent-key tombstones included
    snapshotLSN: UInt64
    windowGeneration: UInt64
    loadsInFlight: Int

Parse HI/LO as (UInt64(hi, radix:16) << 32) | lo. Missing/empty/malformed LSN is nil and must not terminate the tail; it behaves as fresh library/no-Postgres data. Strict hex parsing is safer than TS parseInt's permissive malformed edge behavior, but document/test it.

    inView(row):
      if lower != nil && cmp(row, lower) < 0: false
      else if ended || boundary == nil: true
      else cmp(row, boundary) <= 0

lower closes the skipped bottom of offset window. Live rows must not grow the window by moving into deliberately skipped rows.

### Initial open order

This order must remain exact:

    1. sub = UUID()
    2. create changesOnly feed for base predicate and forced projection
    3. HEAD feed; capture stream-next-offset, fallback "-1"
    4. query first Postgres page with original where/limit/first-page offset
    5. atomically seed local window
    6. start feed at captured offset
    7. start lease keeper only after collection/state exists

Creating/capturing feed before page query prevents a gap. A shared old feed can have backlog; HEAD skips it. Overlap from captured offset through page snapshot is rejected by LSN watermarking. HEAD failure falling back to origin is correct but expensive. If setup after feed creation fails, delete the feed using same subscription before throwing.

Seed/reseed in one observer-visible transaction:

    pageLSN = parse(page.lsn) ?? 0
    delete present keys absent from new page
    upsert each page row
    present = page keys
    applied = every page key -> pageLSN
    boundary = final row or nil
    lower = first row iff original offset != 0 and rows nonempty
    ended = (limit == 0) || rows.count < limit
    snapshotLSN = pageLSN
    windowGeneration += 1

Reject a page that lacks forced PK rather than silently creating an unstable key.

### Merge one live delta

Ignore envelope when type is not canonical requested table. Let d = parse(envelope.headers.lsn), w = applied[key].

    fresh = d == nil || (w == nil ? d >= snapshotLSN : d >= w)
    if !fresh: ignore

    if operation == delete:
      existed = present.remove(key) != nil
      if d != nil: applied[key] = d     // tombstone, even unseen key
      else: applied.remove(key)
      emit delete(key) only if existed
      return

    if value missing: ignore
    if inView(value):
      emit present.contains(key) ? update(value) : insert(value)
      present.insert(key)
      if d != nil: applied[key] = d
      return

    if present.contains(key):             // move out
      present.remove(key)
      if d != nil: applied[key] = d       // move-out tombstone
      else: applied.remove(key)
      emit delete(key)

Same-LSN delta is fresh. Unseen pre-snapshot upsert is ignored because it belongs to a later page, not live window. Unseen delete creates tombstone to prevent stale in-flight page ghost. Missing LSN falls back to idempotent-by-key.

### loadMore

Return zero without I/O if closed, ended, pageSize <= 0, no boundary, or uninitialized. Default pageSize is original limit. Never reapply initial offset.

Build strict keyset predicate AND base where:

- no orderBy: pk > boundary.pk;
- non-null ascending: col > b.col OR (col = b.col AND pk > b.pk);
- descending reverses both inequalities;
- ascending non-null boundary includes OR col IS NULL;
- ascending null boundary: col IS NULL AND pk > b.pk;
- descending null boundary: (col IS NULL AND pk < b.pk) OR col IS NOT NULL;
- descending non-null boundary: ordinary descending predicate.

Capture windowGeneration, increment loadsInFlight, query, then discard with zero if generation changed due to replacement reseed. Otherwise:

    pageLSN = parse(page.lsn) ?? snapshotLSN
    for row:
      if applied[key] exists and pageLSN < applied[key]: skip
      else upsert row; present.insert(key); applied[key] = pageLSN
    boundary = final returned row, if any
    if returned rows < requested pageSize: ended = true
    return returned row count, not actual inserted count
    finally:
      decrement loadsInFlight
      when zero, remove applied entries for keys absent from present

Tombstones are retained only while a page can race them. TS does not serialize concurrent loadMore calls; actor serialization is safer but test public behavior if changing it.

### Replacement

If renewal returns a distinct shapeId or streamPath:

    abort old tail
    HEAD new feed -> captured offset
    query fresh first page
    atomically reseed entire window
    start new tail at captured offset

Replacement changes-only feed has no history for lease-lapse gap, so origin replay is not recovery. Fresh query is mandatory. Pages beyond first are intentionally lost. Tail readers need cancellation plus generation fencing.

## Replaceable glue vs correctness

| TS component | Swift substitute | Required behavior |
| --- | --- | --- |
| tRPC/batch link | URLSession/direct engine API | no |
| Zod | Decodable/schema validator | validation useful; library no |
| durable-stream client | URLSession SSE/long-poll AsyncSequence | yes, equivalent cursor/framing |
| durable-stream state DB | actor map and AsyncStream | yes: preload, key map, txid barrier |
| TanStack collection | snapshot/change publisher | atomic seed/reseed required |
| AbortController | Task cancellation + generation | both required |
| setInterval/unref | cancellable renewal Task | cadence + drain required |
| JS BigInt/iterator | lossless int + Unicode scalar compare | exact comparison required |

Use one Swift actor per materialization for handles, readers, generation, listeners, lease chain, and row/window state. Expose immutable Sendable snapshots/change batches. Keep UI bindings on MainActor outside networking actor.

## Porting checklist

- [ ] Canonical table parsing, aliases, collision rejection.
- [ ] Lossless value decoding and opaque envelope-key maps.
- [ ] Direct create/query/delete control-plane client with UUID claim every operation.
- [ ] HEAD offset capture plus cancellable durable-stream envelope AsyncSequence.
- [ ] Shape snapshot preload, keyed materializer, replacement, txid barrier.
- [ ] One-shot materialization/client close registry.
- [ ] Serialized lease renewal and drain-before-release.
- [ ] Aggregate reader with generation fence.
- [ ] Full subset comparator, null-aware cursor, first-page lower bound, feed-before-page, LSN/tombstone merge, atomic reseed.
- [ ] Port subset.test.ts coverage: overlap, stale delete, unseen tombstone, move-out, malformed/missing LSN, large int/text/code-point order, delete retries, renewal drain.
- [ ] Add integration coverage for eviction replacement, replacement during loadMore, background lease lapse, stream close, txid batches.

## Open contracts / limitations

1. **Known subset seam.** Architecture explicitly says LSN positioning alone has a theoretical Postgres snapshot-visibility race. Strict correction needs snapshot xid visibility data, which /query does not return. Do not promise perfect seam correctness until server fencing changes.
2. **Reconnect behavior is unspecified here.** TS delegates shape reconnect to external state DB; subset/aggregate only log non-abort tail errors, without retry. Define Swift resume/recreate/error behavior.
3. **"-1", SSE framing, and JSON batching are external durable-stream contracts.** Obtain that specification before raw Swift reader implementation.
4. **awaitTxId is external-package behavior.** Specify per-envelope versus transaction-last behavior, already-consumed txid, timeout, close/replacement, and stream error outcomes.
5. **Close swallows final release failure in TS.** Decide whether Swift matches best effort or reports/persists teardown failure.
6. **Engine supports composite key encoding but current TS schema declares one PK column.** Opaque keys are safe now; defer public composite-PK model.

