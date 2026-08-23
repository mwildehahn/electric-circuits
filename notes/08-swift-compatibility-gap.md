# Swift ElectricSQL to Electric Circuits compatibility gap

## Scope and conclusion

This note compares the public contracts used by the dependency-free Swift client in the
sibling `electric-sync-swift` checkout with both Circuits surfaces:

- **Electric-compatible:** `GET /v1/shape`, implemented in
  `apps/engine/src/electric.rs`.
- **Native Circuits:** the engine HTTP endpoints, API tRPC router, and durable shape
  stream protocol in `apps/engine/src/http.rs`, `apps/api/src/router.ts`, and
  `packages/protocol/src/envelope.ts`.

**Fact:** `/v1/shape` is a useful bridge for an existing Swift ElectricSQL client after an
HTTP adapter translates responses, errors, and key/value encoding. It covers a complete
snapshot followed by long-poll live mutations. It does not cover the Swift client's
`changes_only`/on-demand bootstrap, tag-driven move-outs, ordered/limited shapes, or its
subset protocol without changes on one side.

**Fact:** Native Circuits covers shapes, one-shot subset pages, live subset feeds, and
aggregates, but its public implementation is TypeScript plus durable-streams, not an
Electric wire endpoint. A Swift native client is therefore a new transport/client layer,
not a mechanical endpoint rename. The TypeScript client is an essential behavioral
reference, especially for leases, lapsed feeds, LSN watermarks, tombstones, and cleanup
(`packages/client/src/index.ts`, `packages/client/src/subset.ts`).

**Design proposal:** retain the Swift package's app-owned cache interfaces and its public
collection model. Add a Circuits transport behind it, selected per collection as
`electricV1` or `native`. Start with the `/v1` bridge to preserve existing applications;
adopt native shapes/subsets/aggregates only after the Swift storage and lifecycle protocol
can represent their stronger identities and delivery semantics.

## Observed contracts (facts)

### Existing Swift client

- `HTTPClientProvider.fetch(_:)` returns a finite `[ElectricMessage]`; the separate
  `HTTPStreamClientProvider.stream(_:)` supplies an `AsyncThrowingStream`. Both are
  application-provided (`electric-sync-swift/Sources/ElectricSync/Providers.swift:347-358`,
  symbols `HTTPClientProvider`, `HTTPStreamClientProvider`). The Swift package contains no
  built-in URLSession/SSE parser or authentication policy.
- `SyncState` persists `offset`, `handle`, `cursor`, `isUpToDate`, and semantic epoch;
  `offset == "-1"` means a full bootstrap and only a non-`-1` offset may resume
  (`electric-sync-swift/Sources/ElectricSync/Models.swift:118-194`, `SyncState`).
- `ElectricShapeRequest` carries `table`, `predicate`, `orderBy`, `limit`, `offset`,
  `handle`, `cursor`, `live`, `log`, `replica`, and optional `subset`
  (`electric-sync-swift/Sources/ElectricSync/Models.swift:462-592`, `ElectricShapeRequest`).
  `ElectricSubsetRequest` is raw SQL where/params plus order/limit/offset
  (`electric-sync-swift/Sources/ElectricSync/Models.swift:390-412`).
- `ElectricMessage` represents mutation/snapshot/truncate plus controls `upToDate`,
  `snapshotEnd`, `mustRefetch`, and `subsetEnd`; it also carries tags, active conditions,
  `txids`, offset/handle/cursor, and field presence
  (`electric-sync-swift/Sources/ElectricSync/Models.swift:595-660`). The
  quarantine requires `.upToDate` to have `isUpToDate == true`
  (`electric-sync-swift/Sources/ElectricSync/ElectricProtocolQuarantine.swift:211-225`,
  `invalidControlDetail`).
- `ElectricSyncClient` applies a buffered `SyncBatch`, clears/reset persists state on
  truncate or must-refetch, and writes the normal resume state only after processing the
  batch (`electric-sync-swift/Sources/ElectricSync/ElectricSyncClient.swift:376-987`,
  `SyncBatch.apply`). It chunks work for memory (`:196-274`, `SyncBatch.chunked`) and
  bounds live buffering at 50,000 (`:2575-2788`, `liveBatchStream`).
- Initial on-demand streaming requests use offset `"now"`; buffered fetches reject an
  empty non-completing HTTP page and require progress
  (`electric-sync-swift/Sources/ElectricSync/ElectricSyncClient.swift`,
  `initialOffsetForStream`,
  `fetchBufferedMessages`; constants `maxHTTPFetchesPerSync`, `maxLiveBatchMessages`).
- The client has eager, on-demand, and progressive collection modes and long-poll/SSE
  choices (`electric-sync-swift/Sources/ElectricSync/Collections.swift`,
  `ElectricCollectionSyncMode`, `ElectricLiveTransport`).
  Session identity is local generation/teardown state with no auth credential contract
  (`electric-sync-swift/Sources/ElectricSync/ElectricSyncSessionProvider.swift:3-62`).
- `DataCacheProvider` and `MetadataProvider` make persistence application-owned
  (`electric-sync-swift/Sources/ElectricSync/Providers.swift:5-151,333-345`).
  `BackgroundTaskProvider` is the explicit hook
  for finite background work (`:417-439`).
- Tagged `moveIn`/`moveOut`, `activeConditions`, and row ownership are a legacy
  synchronization mechanism, not mere mutation names
  (`electric-sync-swift/Sources/ElectricSync/MoveOutTagTracker.swift`,
  `electric-sync-swift/Sources/ElectricSync/ElectricSyncClient.swift:635-768`).
- `ElectricRowKey` parses Electric's structured quoted key grammar
  (`electric-sync-swift/Sources/ElectricSync/ElectricRowKey.swift:3-123`).
  `SyncPredicateExpression` has comparisons,
  membership, boolean composition, and `Int`, but no subquery AST leaf
  (`electric-sync-swift/Sources/ElectricSync/PredicateExpression.swift:3-182`).

### Circuits Electric-compatible surface

- `GET /v1/shape` accepts `table`, `offset`, `handle`, SQL `where`, `params`, `columns`,
  `live`, `cursor`, `replica`, `secret`, and `api_secret`
  (`apps/engine/src/electric.rs:66-91`, `ShapeParams`). It has no `log`, `order_by`,
  `limit`, or `subset` parameter in this type.
- `offset=-1` parses the SQL predicate, creates a materialized shape, emits snapshot
  inserts followed by an up-to-date control, and creates a per-client handle. A later
  non-`-1` request requires that handle (`electric.rs:854-965`, `shape_inner`).
- The service ignores the submitted cursor and replica for response generation and serves
  full rows (`electric.rs:854-864`). It accepts long polling; no SSE endpoint is implemented.
  A quiet live poll returns HTTP 204 after the configured wait (`electric.rs:1-45`).
- Change JSON has `headers.operation`, `key`, and optional `value`; controls have
  `headers.control` and the current offset in the response protocol
  (`electric.rs:298-319`, `change_msg`, `control_msg`). The adapter derives updates from
  absolute `upsert`/`delete` output (`:542-570`, `apply_changes`).
- A stale/unknown v1 handle returns HTTP 409 and an Electric must-refetch body; malformed
  parameters are 400; most internal failures are 500. Create races/draining can be 503 with
  retry-after (`electric.rs:334-407`). Handles renew on use, have a default TTL, and do not
  survive engine restart (`electric.rs:1-45`).
- Only query parameters `secret`/`api_secret` are checked by this handler when configured
  (`electric.rs:820-846`, `shape`). This is not an Authorization-header protocol.
- Values are rendered as Postgres text for Electric compatibility (`electric.rs:1-45`).

### Circuits native surface

- Native shapes use a JSON predicate AST and a named subscription; `ShapeResp` returns
  `shapeId`, stream path/URL, subscription, lease duration, state, and subscriptions
  (`apps/engine/src/http.rs:188-300`, `CreateShapeReq`, `ShapeResp`). The tRPC equivalent
  is `shapes.create` (`apps/api/src/router.ts:68-100`).
- Named `DELETE /shapes/{id}?subscription=...` is idempotent and durable-before-ack;
  anonymous release is legacy and not retry-safe (`http.rs:542-585`). The TypeScript client
  renews at one-third of the lease and drains renewal before close
  (`packages/client/src/subset.ts:209-278`, `startLeaseRenewal`).
- Native change envelopes carry `type`, `key`, optional typed `value`, and headers with
  operation plus optional `txid`, `offset`, `lsn`, `seq`, and `last`
  (`packages/protocol/src/envelope.ts:9-46`, `StreamEnvelope`). Table-input envelopes,
  rather than shape-output envelopes, stamp `last: true` (`:51-59`, `toTableEnvelope`;
  `apps/engine/src/engine/output.rs:98-169`, `translate_output`).
- `POST /query` supplies one-shot subset pages: JSON predicate, selected columns, one order
  clause, limit and offset, returning rows plus LSN (`apps/engine/src/http.rs:148-177`,
  `QueryReq`). API subset operations use the same restrictions
  (`apps/api/src/router.ts:102-143`).
- Native live subsets use a changes-only feed plus a page captured between LSN watermarks,
  merge both with tombstones, and reload when a lapsed lease changes the feed
  (`packages/client/src/subset.ts:362-660`). `loadMore` uses a keyset cursor with tombstone
  protection; it is not generic offset-only pagination.
- Native aggregates are first-class create/read streams (`apps/api/src/router.ts:146-165`);
  output records are `upsert` envelopes keyed `agg` with `{ value, n }`
  (`apps/engine/src/engine/output.rs:190-214`, `agg_envelope`).
- In the inspected native handlers/router there is no per-request authorization field or
  auth middleware. This is an observation about those files, not a claim that a deployment
  cannot enforce authentication at a proxy or future middleware.

## Symbol-by-symbol mapping

| Existing Swift symbol/behavior | `/v1/shape` bridge | Native Circuits destination | Required compatibility decision/change |
| --- | --- | --- | --- |
| `HTTPClientProvider.fetch` | URLSession long-poll/fetch adapter; decode body and headers. | Engine/API RPC for create/query/renew/release plus durable stream read. | Add explicit response metadata; body-only `[ElectricMessage]` cannot hold native lease/LSN state. |
| `HTTPStreamClientProvider.stream` | Do **not** select SSE; map to repeated long polls. | Durable-stream tail adapter, reconnecting from durable position. | Expose cancellation and bounded buffering. |
| `ElectricShapeRequest` | Map table, SQL where/params, columns, offset, handle, live, secret. | Translate predicate AST and columns to `CreateShapeReq`; order/page become subset calls. | Split this overloaded request into shape, page, and feed requests. |
| `offset`, `handle`, `cursor` | `-1` bootstrap + returned handle works; cursor is cache hint only. | Durable position and query LSN are distinct. | Never reuse a v1 offset string as a native durable cursor/LSN watermark. |
| `live` | Long-poll tail works. | Tail returned `shape/{id}` stream. | Cancellation closes the tail but must not release a still-owned claim. |
| `log == changesOnly` | **Blocker:** no `ShapeParams.log`; initial `offset=now` fails without a handle. | Supported by native `changesOnly` shape. | On-demand needs native transport or a v1 `log=changes_only` extension. |
| `orderBy`, `limit`, `subset` | **Blocker:** v1 type has none. | `QueryReq`/subset API supports page order + limit and live feed. | Migrate bounded lists to native subset lifecycle. |
| `ElectricSubsetRequest` raw SQL | No direct v1 subset operation. | Requires AST translation; native supports one order field. | Retain SQL compiler for v1 only; add Swift AST encoder. |
| `ElectricMessage` mutation/snapshot/control | Decode `insert`/`update`/`delete`; synthesize `.upToDate(isUpToDate: true)`. | Decode `StreamEnvelope` into a new native event model. | Native envelopes are not Electric messages; controls/field presence differ. |
| truncate / must-refetch | 409 body maps to `.mustRefetch`. | Missing/replaced feed triggers page/feed recreation. | Use one atomic cache/metadata reset path. |
| tags, `moveIn`, `moveOut`, conditions | **Mismatch:** none emitted. | None emitted; membership is absolute upsert/delete. | Disable tagged mode; model move-out as authoritative delete. |
| `ElectricRowKey` | Conditional blocker: Circuits key is not documented as quoted Electric grammar. | Key is opaque protocol data. | Version a codec only if application identity requires decoding. |
| `SyncState` / semantic epoch | Persist v1 offset + handle only while valid. | Persist shape/subscription, durable position, query LSN, page cursor separately. | Create a versioned metadata migration. |
| `SyncBatch.apply` | Snapshot controls yield a boundary; 204 needs synthetic completion. | Output may contain `txid` but no confirmed final marker. | Do not promise source-transaction atomic local delivery. |
| circuit breaker/retry | 400 terminal; 409 reset; 204 idle; 503/5xx/network retry. | Also classify RPC/tail and definite release results. | Add typed errors before existing breaker logic. |
| session / background | Polls renew v1 handle. | Renewal is explicit named claim. | Bind subscription to persisted collection/session, not an ephemeral Task. |
| app-owned cache providers | Existing storage interfaces remain usable. | Same, with feed/page watermark persistence. | Use local DB transactions; durable stream is not local source of truth. |
| aggregate collection | Not through v1. | Native aggregate stream. | New typed aggregate model, lease/cache identity needed. |

## Behavior-by-behavior compatibility analysis

### Snapshot, live tail, offsets, and control messages

**Fact:** V1 bootstrap is close to Swift's eager path: request `offset=-1`, apply inserts,
persist the returned offset/handle/cursor, and treat the trailing up-to-date control as the
snapshot boundary. Its HTTP 204 long-poll is a mismatch: Swift buffered fetching treats an
empty page before completion as an error. The adapter must synthesize a Swift control from
204 plus `electric-up-to-date` response headers, or use a separate Circuits polling path.

**Fact:** V1's cursor is not authoritative: `shape_inner` ignores submitted cursor/replica.
The resume pair is the server handle plus returned offset, subject to expiry/restart. A 409
must-refetch is recovery rather than ordinary continuation.

**Fact:** Native subset correctness needs three state values that v1 `SyncState` does not
distinguish: durable stream position, snapshot-page LSN, and tombstones for live rows arriving
before the page lands. `mergeFeedDelta` and guarded page seed in
`packages/client/src/subset.ts` are the reference behavior.

**Design proposal:** define a tagged `resumeToken` and store it with a versioned local cache
epoch. A reset deletes, or logically invalidates, old-epoch rows in the same local transaction
that records a new bootstrap/feed identity.

### Upsert, delete, updates, and move-out

**Fact:** Circuits changes are *absolute*: `upsert` means a row matches now with current
values; `delete` means it does not. V1 derives insert/update/delete from a per-handle prior
key set. A predicate move-out is an ordinary delete and move-in is an upsert
(`apps/engine/src/electric.rs:542-570`).

**Fact:** Swift tagged mode has independent membership/tag bookkeeping. No Circuits surface
emits `tags`, `removedTags`, `activeConditions`, or `moveIn`/`moveOut`. Synthesizing those
from absolute changes would invent semantics for overlapping predicates/owners.

**Design proposal:** a Circuits collection reducer accepts only `upsert`, `delete`, and
reset. Keep tagged collections behind the old provider. An application that needs ownership
precedence needs an explicit ownership table keyed by opaque row key and feed ID.

### Transaction boundaries and delivery ordering

**Fact:** Circuits handles source transactions atomically in its change log: final source
envelopes have `headers.last=true`, and the sequencer holds incomplete trailing transactions
(repository `AGENTS.md`, “A transaction is one unit of visibility…”). This prevents partial
source transaction publication.

**Fact:** That marker is not propagated as a shape-output completion signal:
`translate_output` emits `last: None` (`apps/engine/src/engine/output.rs:98-169`). Native
shape events may have `txid`, but repeated txids cannot identify final delivery across
reconnect/page boundaries. V1 exposes neither txid nor transaction boundary.

**Compatibility consequence:** Swift can preserve event order and atomically apply one event
with metadata, but cannot promise observers an all-or-nothing local commit for every Postgres
transaction. This is a blocker only when applications require observer-visible transaction
atomicity.

**Design proposal:** native output should preserve `txid` and document `headers.last=true`
on each source transaction's final shape envelope. Swift can then group exactly one transaction
with bounded/spill policy consistent with the engine; it must not drop a partial group on
suspension or cancellation.

### Authentication, errors, and retries

**Fact:** Swift leaves authentication to its host. V1 only checks configured query
`secret`/`api_secret`. Inspected native handlers have no auth input/middleware. Native use
also needs deployment-defined durable-stream credentials.

**Design proposal:** use a `Sendable` credential provider that produces request headers or
query values and supports refresh. Define it for engine/API, v1, and durable-stream
reader/HEAD/tail. Never persist long-lived secrets or log credential-bearing URLs.

**Design proposal:** classify wire outcomes before using the existing circuit breaker:

| Outcome | Swift action |
| --- | --- |
| V1 204 / normal tail timeout | Emit idle/up-to-date boundary; poll again while active. |
| V1 400 | Terminal configuration/protocol failure; do not retry. |
| V1 409 must-refetch | Atomically reset cache/metadata; full bootstrap with bounded backoff. |
| V1 503, 5xx, timeout, connection loss | Retry with breaker/backoff; honor retry-after. |
| Native definite release 404 | Complete only under named-claim endpoint contract. |
| Native lapsed/replaced feed | Recreate claim, reseed page/feed under watermarks, then resume. |

Use URLSession async APIs, propagate cancellation, avoid reachability preflight, and preserve
decoding context rather than `try?` (`axiom-networking`, `axiom-data`).

### Leases, releases, persistence, and suspension

**Fact:** Both modes lease but identify claims differently. V1's opaque server-memory handle
renews by polling, expires, and is lost on restart. Native uses a caller-named subscription
on a durable shape; renewal repeats create/claim. Named native release is retry-safe;
anonymous release is not.

**Fact:** Mobile suspension is normal. Long polling and timers cannot run reliably then. A
lease lapse, replacement feed, or V1 409 must be routine recovery. `BackgroundTaskProvider`
should receive only finite checkpoint work the OS grants.

**Design proposal:** persist a subscription record before opening a tail, renew at one-third
lease while foreground-active, serialize renewal with close, and validate/recreate on resume.
Delay destructive release until tail and renewal task terminate. Use an actor for this
independent lifecycle and `Sendable` values (`axiom-concurrency`, `axiom-swift`).

### Pagination, subsets, and aggregates

**Fact:** `/v1/shape` supplies only full filtered shapes. It cannot faithfully implement
`ElectricSubsetRequest` (raw where, order, limit/offset, subset controls) or native live
subset behavior.

**Fact:** Native subset changes the contract: JSON predicate AST, one sort clause, and an
LSN/tombstone page/live seam. `loadMore` is keyset-oriented. Offset-only pagination can be
offered only if apps accept shifting later offsets in a changing ordered data set.

**Design proposal:** expose `CircuitsSubset` with opaque `PageCursor` and explicit
`SnapshotWatermark`; retain `ElectricSubsetRequest` for legacy provider. Reject multi-sort
and unsupported predicates at build time. Make aggregate a separate observable because
`{value,n}` is not a row collection.

## Proposed Swift-facing protocol (design proposal; not implementation)

The types below prevent accidental mixing of v1 offsets, native LSNs, and durable positions:

```swift
enum CircuitsTransportKind: Sendable { case electricV1, native }

enum CircuitsResumeToken: Codable, Sendable {
  case electricV1(offset: String, handle: String, cursor: String?)
  case nativeShape(shapeID: String, subscription: String, streamPosition: String?)
  case nativeSubset(feedID: String, subscription: String, streamPosition: String?,
                    snapshotLSN: String?, pageCursor: String?)
}

struct CircuitsSubscriptionRecord: Codable, Sendable {
  let transport: CircuitsTransportKind
  let collectionID: String
  let resume: CircuitsResumeToken
  let leaseSeconds: Int?
  let cacheEpoch: UUID
}

enum CircuitsDelta: Sendable {
  case upsert(key: String, row: [String: CircuitsValue], txid: String?, lsn: String?)
  case delete(key: String, txid: String?, lsn: String?)
  case snapshotComplete(resume: CircuitsResumeToken)
  case reset(reason: CircuitsResetReason)
  case aggregate(value: CircuitsValue, multiplicity: Int64)
}

protocol CircuitsReplicaTransport: Sendable {
  func bootstrap(_ request: CircuitsBootstrapRequest) async throws -> CircuitsBootstrap
  func tail(from resume: CircuitsResumeToken) -> AsyncThrowingStream<CircuitsDelta, Error>
  func renew(_ record: CircuitsSubscriptionRecord) async throws -> CircuitsSubscriptionRecord
  func release(_ record: CircuitsSubscriptionRecord) async throws
}
```

`CircuitsValue` should preserve native JSON types, including the documented decimal-string
representation for large integers (`packages/protocol/src/types.ts:12-24`), rather than force
all payloads through v1 text decoding. `key` remains opaque; local primary-key conversion
happens only when a versioned collection-specific codec proves it valid.

The transport manager should be an actor with one tail, one serial renewal, and one close
transition per `collectionID`. Its stream must establish `onTermination` cleanup, propagate
Task cancellation to URLSession/durable-stream work, and use a bounded policy that never
silently drops ordered deltas. If it cannot keep up, surface a recoverable reset/rebootstrap,
not a discarded arbitrary prefix (the `AsyncThrowingStream` guidance in
`axiom-concurrency`).

## Exact migration work and blockers

1. **Build a v1 HTTP adapter first.** Construct supported query parameters, decode JSON and
   headers, synthesize 204 completion, classify v1 errors, and treat keys as opaque. Configure
   secrets through a host credential provider. This enables eager full shapes and long-poll
   tails.
2. **Gate unsupported legacy options.** For a v1 collection, reject or route away
   `log == changesOnly`, initial `offset == "now"`, `orderBy`, `limit`, `subset`, tagged
   ownership mode, and SSE. Silently ignoring any is incorrect behavior or data loss.
3. **Add native control-plane and durable-stream transport.** Define authentication with the
   deployment owner, then create/renew/release/tail with recovery. Persist a stable native
   subscription; never mint a new name on every retry.
4. **Migrate local metadata transactionally.** Version `SyncState` into tagged resume tokens.
   Store cache epoch, subscription, durable position, query LSN, and page cursor separately.
   Retain cached rows only when schema and key codec compatibility is proven; otherwise
   bootstrap a new epoch.
5. **Implement native subsets from the TypeScript reference, not offset polling.** Match the
   HEAD/feed/page LSN seam, tombstones, feed replacement, and keyset load-more behavior in
   `packages/client/src/subset.ts`. This is the largest behavioral migration.
6. **Add a separate aggregate observable.** It needs typed decoding, multiplicity handling,
   named lease, cache persistence, and reset semantics.
7. **Decide transaction semantics.** Whole-Postgres-transaction observer visibility is a
   protocol blocker: request documented final output markers and test them. Otherwise state
   that v1/native reducers use event-level local transactions.
8. **Test lifecycle failures.** Include 204 idle, interrupted batch, 409 expiry, restart,
   suspension lease lapse, cancellation during renewal/close, stream recreation, schema reset,
   page/feed race, absolute move-in/out, and release retry. Verify local rows and metadata
   after every injected failure.

## Decisions needed from the migration owner

- Is the target **compatibility first** (eager full shapes through `/v1`) or **native
  capability first** (subsets/aggregates and a new Swift transport)? A per-collection mixed
  rollout is viable.
- Can every app treat Circuits keys as opaque, or does an app parse `ElectricRowKey` or rely
  on quoted key grammar? The latter needs a documented codec and migration tests.
- Does any app require legacy tagged ownership? There is no direct Circuits equivalent;
  preserve the provider or define application ownership before migration.
- Does any observer require Postgres transaction-atomic local visibility? If yes, native
  output completion must be added; `/v1` cannot supply it.
- What authenticates engine control, v1, and durable-stream endpoints per deployment? This
  must be settled before a native mobile client is secure.
