# Circuits wire protocol for a native Swift client

Status: research note — verified against this checkout on 2026-08-22.

## Bottom line

There are two different client contracts:

| Surface | Intended consumer | Swift suitability | Compatibility status |
|---|---|---|---|
| `GET /v1/shape` | Existing Electric clients | Best reuse path for an Electric-compatible Swift sync implementation | **Compatibility surface.** Implements the snapshot/handle/long-poll protocol, but only the subset of SQL `where` that this adapter parses. |
| Engine native HTTP: `/shapes`, `/query`, `/aggregate` | The repository's Node core/client | Practical direct REST contract for a new Swift client, but not versioned or authenticated | **Functional but not declared stable.** JSON names and semantic tests exist; no OpenAPI/version negotiation. |
| durable-streams `shape/<id>` | Native client data plane | Required for native shapes, subset feeds, and aggregates | **Required implementation dependency, not an app-level protocol spec.** Treat offsets and `stream-*` headers as opaque. |
| tRPC (`apps/api`) | `@electric-circuits/client` | Do not hand-write for Swift unless the server team commits to a tRPC transport/version | **Internal/unstable transport.** Its typed TypeScript router is the source of truth, not a language-neutral HTTP spec. |
| `/graph`, `/state`, `/trace`, table-editing and metrics routes | Pipeline visualizer/operators | Do not ship in a mobile client | **Internal/debug/admin.** Several are unauthenticated when enabled. |

Recommendation: retain the existing Electric Swift sync layer against `/v1/shape` where possible. If building a Circuits-native Swift client, use direct engine lifecycle/query endpoints plus durable-streams reads behind one authenticated reverse proxy, and explicitly own reconnection, leases, stream retirement, and JSON precision. Do not depend on the tRPC batch encoding.

Evidence: adapter routing is registered in [`apps/engine/src/http.rs` `router_with_introspection`](../apps/engine/src/http.rs#L15-L71); the TypeScript client itself calls tRPC only for lifecycle/query then reads the stream URL directly ([`packages/client/src/index.ts`](../packages/client/src/index.ts#L154-L245)).

## Shared data model

### Table names, rows, predicates

* A table is canonically `schema.name`; bare `items` means `public.items`. Reject empty parts, a second dot, and quoted identifier text instead of attempting to repair it. See [`packages/protocol/src/sql.ts` `parseTableRef`](../packages/protocol/src/sql.ts#L35-L67).
* Native JSON predicates are a recursive AST: comparison (`eq`, `neq`, `lt`, `lte`, `gt`, `gte`), null test, `and`/`or`/`not`, and a one-column `IN`/`NOT IN` subquery. The full TypeScript contract is [`packages/protocol/src/types.ts`](../packages/protocol/src/types.ts#L43-L143). SQL NULL semantics apply: only TRUE matches; comparison with NULL is UNKNOWN.
* A projection omits non-requested columns, but the engine includes all primary-key columns. This is materially important for a composite PostgreSQL key ([`packages/conformance/src/conformance-native-composite-pk.test.ts`](../packages/conformance/src/conformance-native-composite-pk.test.ts#L48-L68)).
* The public TypeScript `Schema` says `primaryKey: string`, whereas engine PostgreSQL introspection supports an ordered array of PK columns ([`apps/engine/src/schema.rs`](../apps/engine/src/schema.rs#L173-L199)). A Swift native-stream decoder must therefore support multiple PK fields even if it does not use the TS schema model.

### Scalar fidelity, timestamps, and keys

* Wire cells are JSON `number | string | boolean | null`. An `int` with absolute value over `2^53 - 1` is a decimal **string**, not a rounded JSON number. Decode declared integer columns as `Int64` from either JSON number or decimal string; preserve strings where an `Int64` cannot be parsed. The exact rule is in [`apps/engine/src/value.rs` `Value::to_json`](../apps/engine/src/value.rs#L40-L129), with an end-to-end bigint test at [`conformance-native-scalar-fidelity.test.ts`](../packages/conformance/src/conformance-native-scalar-fidelity.test.ts#L25-L38).
* Timestamps, UUIDs, and other PostgreSQL types currently coarse-map to `text`, so native feeds/pages carry their Postgres text representation. Backfill deliberately casts those values to text to agree with pgoutput, rather than promising ISO-8601 JSON timestamp objects ([`apps/engine/src/pg.rs` `row_json_expr`, comments](../apps/engine/src/pg.rs#L1177-L1182)). Decode timestamp strings using a tolerant Postgres-text formatter; do not assume a `T` separator or a fixed fractional precision.
* `StreamEnvelope.key` is a string identity, not necessarily a JSON encoding of PK values. A single PK uses the bare value string. A composite PK escapes `\\` as `\\\\`, U+001F as the literal six-character sequence `\\x1f`, then joins components with U+001F ([`apps/engine/src/schema.rs`](../apps/engine/src/schema.rs#L236-L275)). Retain the opaque key for map identity; only decode it into components if the app actually needs that, using this exact escape grammar.

## 1. Electric-compatible `GET /v1/shape`

This is the most mature externally compatible path. It is served by the Rust engine, not `apps/api` ([`apps/api/README.md`](../apps/api/README.md#L11-L15)). `OPTIONS /v1/shape` returns `204` and `Access-Control-Allow-Methods: GET, POST, HEAD, DELETE, OPTIONS`, but the adapter itself is GET-only ([`apps/engine/src/http.rs`](../apps/engine/src/http.rs#L129-L134)).

### Snapshot request and response

Request a fresh snapshot with `offset=-1` (or omit `offset`). `table` is required. `where` is a SQL-text expression; `columns` is one comma-separated query parameter, not a JSON array.

```http
GET /v1/shape?table=public.issues&offset=-1&where=status%20%3D%20%27open%27&columns=id,title,status HTTP/1.1
Host: circuits.example
```

```http
HTTP/1.1 200 OK
Content-Type: application/json
Cache-Control: no-store
electric-handle: s42h7
electric-offset: 0000000000000012_0000000000004096
electric-schema: {"id":{"type":"int8","pk_index":0},"title":{"type":"text"},"status":{"type":"text"}}
electric-up-to-date:

[
  {"headers":{"operation":"insert"},"key":"7",
   "value":{"id":"7","title":"Fix tailer","status":"open"}},
  {"headers":{"control":"up-to-date","global_last_seen_lsn":"0"}}
]
```

The snapshot creates or joins a maintained engine shape, folds it to its current state, emits all rows as `insert`, then emits the control message. The handle is per snapshot/client even when the backing native shape is shared. Source: [`apps/engine/src/electric.rs` `shape_inner`](../apps/engine/src/electric.rs#L854-L955). Values here are **all Postgres text** (except NULL): even an `int8` value is `"7"`; `electric-schema` tells Electric value mappers which values to coerce ([`electric.rs`](../apps/engine/src/electric.rs#L248-L296)).

### Positioned catch-up and live tail

Subsequent requests must echo `table`, `handle`, and the most recent `electric-offset`. `live=true` means long-poll.

```http
GET /v1/shape?table=public.issues&handle=s42h7&offset=0000000000000012_0000000000004096&live=true HTTP/1.1
```

```http
HTTP/1.1 200 OK
Content-Type: application/json
Cache-Control: no-store
electric-handle: s42h7
electric-offset: 0000000000000013_0000000000004352
electric-cursor: 8
electric-up-to-date:

[{"headers":{"operation":"update"},"key":"7",
  "value":{"id":"7","title":"Fix tailer","status":"closed"}},
 {"headers":{"control":"up-to-date","global_last_seen_lsn":"0"}}]
```

Rules a Swift poller must implement:

1. Persist and only advance from the returned `electric-offset`; it is an opaque durable-stream offset. The present server tokens are zero-padded `read-seq_byte` strings, but that is an implementation detail ([`apps/engine/src/electric.rs`](../apps/engine/src/electric.rs#L410-L420)).
2. Apply `insert`/`update`/`delete` in order. The adapter reconstructs insert-vs-update from a per-handle key set because native streams are absolute upserts/deletes ([`electric.rs`](../apps/engine/src/electric.rs#L540-L566)).
3. On idle live timeout it returns `204 No Content`, with `electric-handle`, `electric-offset`, `electric-cursor`, and `electric-up-to-date`; immediately issue the next long-poll. The overall deadline is `ELECTRIC_LIVE_TIMEOUT_MS`, default 20 seconds ([`electric.rs`](../apps/engine/src/electric.rs#L755-L787)).
4. `409` with `[{"headers":{"control":"must-refetch"}}]` and `electric-offset: -1` means the handle is unknown/expired or its shape retired. Drop local state and start a new snapshot; never reuse that handle ([`electric.rs`](../apps/engine/src/electric.rs#L334-L340), [`electric.rs`](../apps/engine/src/electric.rs#L996-L1021)). Handles are process-local and do not survive engine restart; idle handles default to a 600-second TTL ([`electric.rs`](../apps/engine/src/electric.rs#L186-L235)).
5. `400 {"message":"…"}` is a permanent request/parse/validation error; `500 {"message":"…"}` is retryable. `503` plus `Retry-After: 1` means a new live poll arrived while the engine is draining; back off ([`electric.rs`](../apps/engine/src/electric.rs#L342-L407)).

The adapter coalesces concurrent `live=true` polls at the same `(handle, offset)` and serializes state updates. A Swift implementation should still maintain one poll per handle; parallel polls at different offsets complicate local exactly-once processing without benefit ([`electric.rs`](../apps/engine/src/electric.rs#L598-L652)).

### SQL `where`, parameters, and caveats

Supported SQL text is deliberately a parser subset: comparison, `LIKE`, `BETWEEN`, literal-list `IN`, one-column `IN (SELECT ...)`, `IS [NOT] NULL`, boolean operators, parentheses, and boolean/numeric/string literals ([`apps/engine/src/where_sql.rs`](../apps/engine/src/where_sql.rs#L1-L11)). It is not a general PostgreSQL predicate language. In particular, qualified inner tables in SQL-text subqueries are rejected; use the native JSON predicate API for a non-public inner table ([`where_sql.rs`](../apps/engine/src/where_sql.rs#L339-L358)).

`$N` parameters may arrive as `params[1]=value&params[2]=value` or one JSON `params={"1":"value"}` field; numbers must be sequential from 1. Values are converted to strings then safely substituted before parsing ([`apps/engine/src/params.rs`](../apps/engine/src/params.rs#L1-L16), [`params.rs`](../apps/engine/src/params.rs#L78-L132)).

`cursor` and `replica` are accepted for compatibility but not semantically honoured: the server creates its own cursor and always emits full-row semantics ([`electric.rs`](../apps/engine/src/electric.rs#L854-L865)).

## 2. Native engine lifecycle/query HTTP

The following routes are JSON REST endpoints on the engine. The Node core uses these exact calls ([`apps/api/src/core.ts`](../apps/api/src/core.ts#L95-L199)), so they are the cleanest wire-level starting point for Swift if Electric compatibility is not required.

### Create/renew a materialized shape

```http
POST /shapes
Content-Type: application/json

{
  "table":"public.issues",
  "where":{"and":[
    {"col":"status","op":"eq","value":"open"},
    {"col":"priority","op":"gte","value":3}
  ]},
  "columns":["id","title","status","priority"],
  "subscription":"ios-install-uuid"
}
```

```json
{
  "shapeId":"s42",
  "table":"public.issues",
  "streamPath":"shape/s42",
  "streamUrl":"https://streams.example/shape/s42",
  "subscription":"ios-install-uuid",
  "leaseSeconds":300
}
```

`changesOnly: true` creates a no-backfill feed used to keep a client-side subset live; normal materialized shapes omit it. The request/response structs and camel-case field names are in [`apps/engine/src/http.rs`](../apps/engine/src/http.rs#L187-L300), and creation happens at [`http.rs`](../apps/engine/src/http.rs#L328-L337).

Identical definitions share a shape stream. A non-empty caller-provided `subscription` (max 128 bytes; no control characters) makes creation idempotent: repeat the same definition and subscription to **renew** rather than taking a second claim. A subscription already owned by a different shape returns `409`; invalid subscription input returns `400` ([`http.rs`](../apps/engine/src/http.rs#L210-L247), [`http.rs`](../apps/engine/src/http.rs#L818-L850)). An omitted subscription makes the engine mint one in the response, but the original create is not idempotent: preserve and subsequently use that returned value.

Native data-plane reads bypass the engine, so sending this same `POST /shapes` before `leaseSeconds` expires is mandatory. The shipped client renews at one third of that interval, with a 250 ms–5 min clamp ([`packages/client/src/subset.ts`](../packages/client/src/subset.ts#L216-L270)). If an old shape has already been evicted, renewal may return a *replacement* `shapeId` and `streamUrl`; atomically switch the reader to the returned handle and reload materialized state.

### Inspect, release, or purge

```http
GET /shapes/s42
```

```json
{
  "shapeId":"s42","table":"public.issues",
  "streamPath":"shape/s42","streamUrl":"https://streams.example/shape/s42",
  "state":"active","subscriptions":1
}
```

```http
DELETE /shapes/s42?subscription=ios-install-uuid
```

```json
{"ok":true}
```

Identified deletion is idempotent and durable before success is returned; retry a timeout/transport failure with the same subscription. A `DELETE` without `subscription` is legacy anonymous decrement and is **not** safe to retry. `?purge=true` force-retires the whole shared shape and ignores `subscription`; reserve it for an administrative action, never ordinary client close ([`apps/engine/src/http.rs`](../apps/engine/src/http.rs#L541-L582)). A cancellation or lost response does not cancel engine-side durable work, so retry rather than infer rollback.

`GET /shapes/{id}` is diagnostic and does not renew a lease. `404` means the record is already retired; a client should create a fresh shape rather than polling it ([`apps/engine/src/http.rs`](../apps/engine/src/http.rs#L363-L372)).

### Native HTTP error handling

Success bodies on the engine-native REST routes are JSON. Handler-originated failures use `{"error":"…"}`; malformed JSON/path extraction is framework-generated and should not be parsed as a stable schema. The meaningful current status mapping is:

| Status | Native meaning | Client action |
|---|---|---|
| `200` | create/get/query/release completed | Consume body. A release’s success is durable. |
| `400` | invalid subscription or invalid table path | Do not retry unchanged input. |
| `404` | requested shape/table/tailer is not present | For a shape/feed, create a fresh subscription; for a table, treat as configuration/schema error. |
| `409` | subscription belongs to another shape; or operator requested epoch reset while healthy | Generate a new subscription only if this is a truly independent materialization; never reuse it for a different predicate. |
| `503` | degraded/broken/resetting epoch, or repeated create-vs-retirement race | Back off and check readiness; do not consume a stale stream. |
| `500` | all other engine/stream/Postgres failures | Retry with bounded exponential backoff when the request is idempotent (named create/release), otherwise surface failure. |

The typed mapping is in [`apps/engine/src/http.rs` `AppError::from`](../apps/engine/src/http.rs#L818-L850). It is a behavior description, not a versioned error specification.

### One-shot subset query and live subset feed

```http
POST /query
Content-Type: application/json

{
  "table":"public.issues",
  "where":{"col":"status","op":"eq","value":"open"},
  "columns":["id","title","priority"],
  "orderBy":{"col":"priority","desc":true},
  "limit":50,
  "offset":0
}
```

```json
{"rows":[{"id":7,"title":"Fix tailer","priority":4}],"lsn":"0/1A2B3C4"}
```

`/query` is a Postgres query-back: no stream and no server-side cursor. `lsn` is useful only for the companion changes-only feed seam; it is not a visibility fence for shape backfill. The request schema is [`apps/engine/src/http.rs`](../apps/engine/src/http.rs#L141-L176), and the protocol type is [`packages/protocol/src/types.ts`](../packages/protocol/src/types.ts#L166-L207).

To make an ordered window live, create `/shapes` with `changesOnly:true` and the **base** predicate, capture the feed `HEAD` offset before reading `/query`, then tail raw envelopes and merge them by per-row LSN. The shipped implementation preserves delete tombstones to stop an in-flight page from resurrecting a removed row ([`packages/client/src/subset.ts`](../packages/client/src/subset.ts#L380-L431), [`subset.ts`](../packages/client/src/subset.ts#L304-L349)). This algorithm is required for correct Swift pagination; do not try to make `limit`/`offset` a server-side live shape.

Ordering is `orderBy` plus primary key as a tie-breaker. For introspected collatable text, the engine deliberately uses PostgreSQL `COLLATE "C"`; a Swift comparator must use Unicode scalar/code-point order, not locale order. For a JSON-declared (non-introspected) schema, this guarantee does not hold ([`apps/engine/src/schema.rs`](../apps/engine/src/schema.rs#L309-L336), [`packages/client/src/subset.ts`](../packages/client/src/subset.ts#L53-L125)).

### Live aggregate

```http
POST /aggregate
Content-Type: application/json

{"table":"public.issues","where":{"col":"status","op":"eq","value":"open"},"fn":"count","subscription":"ios-agg-uuid"}
```

The response is the same `ShapeHandle` shape as `/shapes`; create/renew/release uses the same lease rules. `fn` is `count | sum | avg | min | max`; `col` is required except for `count`, and aggregate predicates cannot contain subqueries ([`apps/engine/src/http.rs`](../apps/engine/src/http.rs#L339-L361), [`apps/engine/src/engine/lifecycle.rs`](../apps/engine/src/engine/lifecycle.rs#L561-L587)). Its stream is not rows: read the latest envelope with key `"agg"` and value `{ "value": <JSON scalar|null>, "n": <matching row count> }` ([`apps/engine/src/engine/output.rs`](../apps/engine/src/engine/output.rs#L195-L212)). `n` is an `i64` JSON number today; treat it as potentially non-representable in JavaScript but directly decodable by Swift `Int64`.

## 3. Durable-streams data plane

The engine returns an absolute URL, but a production Swift client should normally construct it through the app's authenticated same-origin/reverse-proxy path. Native data reads are never mediated by the engine.

### Read and tail

```http
GET /shape/s42?offset=-1 HTTP/1.1
Accept: application/json
```

```http
HTTP/1.1 200 OK
Content-Type: application/json
stream-next-offset: 0000000000000012_0000000000004096
stream-up-to-date: 1

[{"type":"public.issues","key":"7","value":{"id":7,"title":"Fix tailer"},
  "headers":{"operation":"upsert","txid":"a5…","lsn":"0/1A2B3C4","offset":"…"}}]
```

Continue from `stream-next-offset`:

```http
GET /shape/s42?offset=0000000000000012_0000000000004096&live=long-poll HTTP/1.1
```

* `offset=-1` means from the beginning. Always URL-encode a returned offset; do not generate or numerically compare it.
* `200` contains a JSON array (possibly empty); `stream-up-to-date` is present when caught up.
* `204` means no new events / long-poll timeout; retain the supplied next offset, if any, and poll again.
* `stream-next-offset` is the resume token; the server stamps individual envelope `headers.offset` on read. The engine’s client behavior is defined in [`apps/engine/src/ds.rs`](../apps/engine/src/ds.rs#L628-L666).
* `stream-closed: true` means terminal retirement; stop immediately and re-create/re-snapshot. A close wakes parked long polls before deletion. `404`, `410`, and `409` with `stream-closed: true` are equivalent terminal cases ([`apps/engine/src/ds.rs`](../apps/engine/src/ds.rs#L89-L129), [`apps/engine/src/ds.rs`](../apps/engine/src/ds.rs#L553-L604)).
* Transient network error or `5xx` is retryable with exponential backoff and jitter. Do not release a subscription just because a data read failed.

### Envelope schema

```json
{
  "type":"public.issues",
  "key":"7",
  "value":{"id":7,"title":"Fix tailer"},
  "headers":{
    "operation":"upsert",
    "txid":"write-uuid",
    "lsn":"0/1A2B3C4",
    "offset":"0000000000000012_0000000000004096"
  }
}
```

`type`, `key`, `headers.operation` are the essential fields. On a shape/feed stream, `operation` is normally `upsert` or `delete`; apply it to a `[String: Row]` keyed by opaque `key`. `txid` is optional and supports app-level “await my write” correlation. `lsn` is present for Postgres live output, absent on backfill and library mode; `seq` and `last` belong to table/change-log ingestion rather than a native shape stream. The protocol definition is [`packages/protocol/src/envelope.ts`](../packages/protocol/src/envelope.ts#L9-L44), with native output construction at [`apps/engine/src/engine/output.rs`](../apps/engine/src/engine/output.rs#L96-L166).

Never route the native client through the segmented internal `changes/<n>` log. `last:true`, `seq`, and `(segment, offset)` belong to engine ingestion/exactly-once recovery and are not a consumer API. The only legitimate application use is diagnostics through `GET /replication/lsn` / `GET /tables/{name}/offset`; those routes are unversioned operator surfaces ([`apps/engine/src/http.rs`](../apps/engine/src/http.rs#L592-L610), [`apps/engine/src/http.rs`](../apps/engine/src/http.rs#L737-L764)).

## 4. tRPC procedure matrix

These are semantic procedures that the published TypeScript client calls. Their input validation comes from Zod in [`apps/api/src/router.ts`](../apps/api/src/router.ts#L13-L166). A Swift client could technically produce tRPC HTTP batches, but that couples it to tRPC encoding/link behavior and loses the direct REST contracts above.

| Procedure | Kind | Input/result | Swift position |
|---|---|---|---|
| `schema.define` | mutation | `{schema}` → `{ok:true}` | Admin/library mode only. |
| `ingest.write` | mutation | `{table, op, pk, row?, txid?}` → `{txid}` | Library mode only; real apps write PostgreSQL. |
| `shapes.create` | mutation | native shape definition + subscription → handle | Map to `POST /shapes`. |
| `shapes.get` | query | `{id}` → handle/404 | Map to `GET /shapes/{id}`. |
| `shapes.delete` | mutation | `{id, subscription?}` → `{ok:true}` | Map to identified `DELETE /shapes/{id}`. |
| `subset.query` | query | subset definition → `{rows,lsn}` | Map to `POST /query`. |
| `subset.live` | mutation | base shape def + subscription → changes-only handle | Map to `POST /shapes` with `changesOnly:true`. |
| `aggregate.create` | mutation | aggregate def + subscription → handle | Map to `POST /aggregate`. |

The API server has no authentication middleware and by default binds only localhost; deployment exposes it by choosing `host: 0.0.0.0` ([`apps/api/src/server.ts`](../apps/api/src/server.ts#L16-L33)). This reinforces treating it as an internal adapter, not the iOS-facing public API.

## Auth, proxy, cancellation, and lifecycle matrix

| Concern | `/v1/shape` | Native REST + durable-streams | Swift guidance |
|---|---|---|---|
| Auth | Only this route checks `ELECTRIC_SECRET`, accepted as `secret` or `api_secret` query parameter; failure is `401 {"message":"Unauthorized"}`. | No native engine control-plane auth; durable-streams access is likewise assumed network-private. | Put engine, API, and streams behind TLS plus one auth/authorization proxy. Do not put a long-lived secret in a query string if you can avoid this compatibility route. |
| CORS | `OPTIONS` only advertises methods; no general allow-origin header is set by the engine. | No CORS policy in these services. | Native Swift is not CORS-constrained, but use a proxy for browser coexistence. |
| Create timeout/cancel | Snapshot creates a hidden native claim; a lost response means re-snapshot. | Named `POST /shapes` may commit after client cancellation; same-subscription retry is a renewal. | Generate and persist one subscription UUID per active materialization before creating it. |
| Release timeout/cancel | Handle TTL eventually releases its hidden claim. | Identified DELETE is safe to retry and completes independently of request cancellation. | Send best-effort close, retry identified delete, then let lease expiry be the fallback. |
| Lease renewal | Every adapter request renews the hidden claim. | Native durable-stream reads are invisible to engine. | Schedule `POST /shapes` / `/aggregate` renewal at roughly `leaseSeconds/3`; serialize renewals with close. |
| Server restart / stale state | Handle registry disappears → `409 must-refetch`. | A stream can remain, go dormant, or be evicted; renewal can return a replacement handle. | Treat any terminal stream outcome or changed handle as resubscribe + rehydrate, not a delta-only resume. |
| Schema drift / TRUNCATE / replica identity regression | Adapter returns must-refetch after backing stream closes. | Affected shape streams are close-then-delete; current read sees closed/404/410. | Invalidate cached schema/data for that shape and recreate only after the app can accept the new schema. |

Auth evidence: [`apps/engine/src/electric.rs`](../apps/engine/src/electric.rs#L795-L846), [`apps/engine/src/config.rs` `secret_ok`](../apps/engine/src/config.rs#L497-L503). The lack of control-plane auth is visible in the router construction ([`apps/engine/src/http.rs`](../apps/engine/src/http.rs#L15-L71)).

### Schema drift and retirement are client-visible

The engine retires a table’s dependent shapes on a schema fingerprint mismatch, `TRUNCATE`, or replica-identity regression; it closes then deletes each stream. This is not a normal delete delta and cannot be repaired by continuing from the old offset. A parked native long-poll is explicitly released with `stream-closed` in the conformance test ([`packages/conformance/src/conformance-schema-drift.test.ts`](../packages/conformance/src/conformance-schema-drift.test.ts#L111-L154)). Recreate the shape and replace local materialized state wholesale. An unresolved table rejects new shapes until engine reconciliation succeeds; `/tables` exposes this only as an operational hint ([`apps/engine/src/http.rs`](../apps/engine/src/http.rs#L302-L325)).

## Swift implementation checklist

1. Choose one mode per feature: Electric protocol for compatibility, or native lifecycle + raw durable-streams for new capabilities. Do not mix an Electric handle with a raw `shape/sN` stream.
2. Model `Value` by declared column type: `Int64` accepts JSON number/string, `Double`, `Bool`, `String`, and nullable variants. Keep envelope `key`, stream offset, LSN, table type, handle, and subscription as strings.
3. For a materialized native shape: persist `{definition, subscription, handle}`, fold raw upserts/deletes in sequence, renew, and make replacement-handle rebinding atomic with a full re-read.
4. For native subset pages: implement the current feed-offset → page-snapshot → LSN-watermarked tail sequence, including tombstones while a `loadMore` is in flight. This is the highest-risk part to reimplement.
5. Stop a tail immediately on `stream-closed`, `404`, `410`, or closed-stream `409`; do not spin. Use exponential backoff with jitter for ordinary network/5xx errors.
6. Serialize renew and close. Stop/drain renewals before DELETE so a late create cannot resurrect the released claim; this exact race is handled by the shipped client ([`packages/client/src/subset.ts`](../packages/client/src/subset.ts#L229-L270)).
7. Treat direct stream URLs as deployment details. Prefer a stable proxy URL you construct from `streamPath`; the TypeScript client has exactly this override for `/ds` proxies ([`packages/client/src/index.ts`](../packages/client/src/index.ts#L154-L161)).

## Explicitly unstable or out of scope for Swift production code

* tRPC request paths, batch envelopes, error payloads, and the TypeScript `AppRouter` are not a language-neutral versioned API.
* `/schema`, direct library-mode `ingest.write`, raw `changes/<segment>` streams, `/replication/lsn`, `/tables/*/offset`, `/epoch/reset`, and all visualizer routes are engine/control-plane internals. They expose useful diagnostics, not a mobile data API.
* `/shapes/{id}/rows` and `/shapes/{id}/log` are visualizer snapshots that fold/poll whole streams and cap output; do not use them for sync ([`apps/engine/src/http.rs`](../apps/engine/src/http.rs#L425-L539)).
* `streamUrl` is returned for convenience, but the code intentionally supports an override behind a development proxy; its hostname/auth policy is deployment-owned, not a durable client promise.
* `/v1/shape` is an Electric-compatibility adapter, not a promise of every Electric SQL feature. Its SQL parser, handle registry, `cursor`, `replica`, and parameter behavior should be compatibility-tested against the exact deployed build.
