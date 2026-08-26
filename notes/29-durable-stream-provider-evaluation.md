# Durable-stream provider evaluation and swappable-store design

Status: **provider boundary implemented; qualification punchlist active**

As-of date: 2026-08-25

Active scope: introduce a generic provider boundary, target
[pgxsinkit/durable-streams-rust](https://github.com/pgxsinkit/durable-streams-rust), remove the
provider assumptions currently embedded in Electric Circuits, and qualify one narrow production
profile. The PicoMQ comparison is retained below as research only; PicoMQ implementation,
integration, and qualification are explicitly deferred.

The engine-side provider-neutral port and the first ds-rust adapter are implemented. This note does
not claim provider durability, restore, capacity, or release qualification; those concrete gates
remain open below.

## Decision

1. **Electric Circuits owns a provider-neutral durable-stream port.** The engine depends on the
   durability and wire semantics it needs, not on PicoMQ, ds-rust, `reqwest`, a particular offset
   syntax, or a particular request-size ceiling.
2. **Keep the provider out of the engine process.** Both candidates speak the open Durable Streams
   HTTP protocol. The production boundary remains a process/network boundary; do not embed either
   Rust crate into the engine.
3. **Use `pgxsinkit/durable-streams-rust` as the first qualified/default provider.** It is the
   already-integrated, simpler single-node deployment and has much stronger protocol and crash
   evidence. Its supported production topology is deliberately narrow: one WAL-mode server on a
   durable volume, supervised restart/replacement, and tested backup/restore. It is not highly
   available.
4. **Do not implement or qualify a second provider now.** The port must be real enough that a future
   conforming provider can be added, but we will not add PicoMQ dependencies, configuration,
   conditionals, deployment manifests, or test lanes in this work.
5. **Do not automatically fail over from one provider or provider deployment to another.** The
   catalog, change-log segments, and every shape stream form one namespace and one engine epoch.
   Runtime fallback to an empty or divergent backend can acknowledge gaps permanently. Changing
   providers is an explicit reset/re-snapshot operation, or a separately designed offline migration
   that proves every stream and offset was preserved.

The initial profile makes no durable-stream service HA claim: ds-rust has no replicated hot tier.
Production recovery means restarting or replacing the process against the same qualified durable
volume, not switching to another backend.

## Why the protocol alone is not enough

Both servers expose Durable Streams, which is the correct interchange protocol. Electric Circuits,
however, relies on a narrower semantic contract that must be tested against every provider:

- an acknowledged append survives the provider's documented failure model;
- one append becomes visible atomically and in order;
- offsets are opaque, durable resume tokens;
- JSON append/read behavior preserves the envelope sequence;
- long polls wake on append and on terminal close;
- close is terminal, while delete and create have retry-safe outcomes;
- ambiguous errors can be classified without discarding a live shape batch;
- request, response, stream, and queue sizes are bounded and exposed;
- cancellation never turns a possibly committed append into an assumed failure;
- the deployment exposes enough identity to reject an accidental empty/wrong backend.

The upstream [Durable Streams protocol](https://github.com/durable-streams/durable-streams) is the
baseline. The Circuits contract suite is the additional product contract. A provider is supported
only when both are green for an exact image/version and production configuration.

## Deferred comparison record

This section preserves the research that led to the choice, but it creates no PicoMQ work in the
active implementation plan.

| Area | `pgxsinkit/durable-streams-rust` | PicoMQ | Deferred research implication |
| --- | --- | --- | --- |
| Durable write | Local sharded WAL; WAL mode acknowledges after `fdatasync` (`F_FULLFSYNC` on macOS). | Records are acknowledged after the WAL bulk reaches object storage; quiet writes pay an object-store round trip. | Both can meet durable-before-ack, but the fault models and latency profiles differ. |
| Topology | One server with local files; optional S3 cold-tier offload does not replicate the acknowledged hot tail. | Nodes cache only; object storage holds records and SQL holds an ordered metadata log/snapshot. PostgreSQL is used for multi-node deployments. | ds-rust is operationally simpler. Pico adds object store and metadata-DB availability/restore coupling. |
| HA | No built-in replicated hot tier. Restart/replacement must recover the same durable volume. | Routing, epoch fencing, and planned live ownership transfer exist. Auto-balancing remains open. | Pico is the HA candidate, not yet proof of automatic crash takeover. |
| Routing/auth | Stable single endpoint; no production auth boundary should be inferred from the server alone. | A non-owner replies `307`; clients must reach every advertised node. Built-in bearer scopes exist, and the Pico client explicitly reattaches auth because standard clients drop `Authorization` on cross-origin redirects. | A future Pico adapter would need explicit redirect and credential handling. This is not part of the current adapter. |
| Append ceiling | Circuits currently assumes the server's 1 GiB body ceiling. | Current Pico and Durable Streams frontends cap request bodies at 8 MiB. | Pico is not drop-in with Circuits' 64 MiB change-log and 16 MiB backfill defaults. |
| Offset representation | Current server offsets include a byte-position component. | Current Durable Streams frontend uses a record-sequence offset. | Offsets must be opaque. Segment size/accounting must be owned durably by Circuits, not parsed from the offset. |
| Protocol evidence | Fork records 326/326 passes against conformance 0.3.5; 0.3.6 has 329 passes and three failures covering close-only TTL sliding and CORS preflight. Crash-simulation findings are documented. | Repository CI runs Rust tests. The inspected Durable Streams HTTP integration suite has five cases using the official Rust client, but no full upstream server-conformance gate was found. | ds-rust has substantially stronger evidence today. Pico must run the same full upstream and Circuits suites. |
| Maturity/release | Circuits already pins fork version `0.1.5`; the fork exists to own packaging after extraction from Electric. Two current open issues are the 0.3.6 conformance gaps. | Workspace version is `0.1.0`; no GitHub release was listed. Current open work includes auto-balancing, disk cache, retention, CDC, range delete, and consumer groups. | Pin immutable images by digest. Treat Pico as experimental until gates below pass. |
| Operations | Simple volume, WAL, process, and optional OTLP footprint; capacity and restore still need Circuits qualification. | `/health`, `/ready`, admin state, transfers, auth, Postgres metadata, and object-store operations. | Pico has broader control surfaces and a materially larger failure matrix. |
| PostgreSQL 18 | Not a metadata dependency. | The repository's cluster harness uses `postgres:18.6-bookworm`. | PG18 is a plausible Pico metadata target, but it still requires promotion/restore tests separate from the application database. |

Primary sources:

- [ds-rust README and durability/observability model](https://github.com/pgxsinkit/durable-streams-rust)
- [ds-rust provenance and conformance status](https://github.com/pgxsinkit/durable-streams-rust/blob/main/PROVENANCE.md)
- [ds-rust crash-simulation findings](https://github.com/pgxsinkit/durable-streams-rust/blob/main/CRASH_SIM_FINDINGS.md)
- [PicoMQ architecture overview](https://github.com/PicoMQ/picomq/blob/main/website/pages/docs/design/overview.md)
- [PicoMQ durable write path](https://github.com/PicoMQ/picomq/blob/main/website/pages/docs/design/writes.md)
- [PicoMQ deployment and routing](https://github.com/PicoMQ/picomq/blob/main/website/pages/docs/operations/deployment/docker.md)
- [PicoMQ authentication and redirect warning](https://github.com/PicoMQ/picomq/blob/main/website/pages/docs/operations/auth.md)
- [PicoMQ Durable Streams frontend, including the 8 MiB bound](https://github.com/PicoMQ/picomq/blob/main/picomq/pico-frontend/src/ds.rs)
- [PicoMQ open issues](https://github.com/PicoMQ/picomq/issues)

### PicoMQ crash-takeover qualification blocker

PicoMQ's deployment documentation says a stream whose node remains down is found closed and revived
by another node. In the inspected revision (`b5861ff03c1801ee913c168851baff033fa8f441`), the ownership
router sends every `Opened` stream to its recorded owner, and node registration updates the node epoch
without closing that node's open streams. No heartbeat/liveness expiry that changes those rows was
found. Therefore an abrupt owner death appears to leave other nodes returning `307` to the dead owner
until that node id restarts or a transfer completes. This is an inference from the current
[ownership implementation](https://github.com/PicoMQ/picomq/blob/main/picomq/pico-server/src/ownership.rs),
not a proven incident result.

This is not a reason to reject PicoMQ permanently. It is a concrete red E2E case: kill the owner
without graceful shutdown, send append and long-poll requests through another node, and require
bounded recovery with no lost acknowledged records, duplicate effects, redirect loop, or stale-owner
write. Until that test passes, Circuits must not claim PicoMQ gives automatic stream HA.

## Current Electric Circuits coupling

The existing `apps/engine/src/ds.rs` is both an HTTP adapter and a domain-policy object. That kept the
initial implementation small, but these details prevent provider neutrality:

| Current behavior | Coupling/problem | Required owner |
| --- | --- | --- |
| `DsClient` stores `reqwest::Client` and a base URL while also implementing reliable retry, gone-stream reconciliation, envelope encoding, and byte accounting. | Transport mechanics and engine correctness policy cannot be substituted or tested independently. | Split a single-attempt store port from the engine-owned semantic facade. |
| `DsClient::stream_url` derives the client URL from the internal write/control endpoint. | A gateway/proxy, replacement deployment, or future provider can have a different public read location. | Separate internal store endpoint from public read locator. |
| `changelog::offset_bytes` parses ds-rust's `<seq>_<byte>` token. | Provider offsets are opaque protocol tokens even when one implementation happens to encode byte positions. | Persist Circuits-owned segment byte counts. |
| Fallback byte accounting is process-local. | A restart undercounts existing streams and can violate rotation/retention budgets. | Make physical/logical byte accounting durable and restore it before serving. |
| `txn_buffer::DS_MAX_BODY_BYTES` is hardcoded to 1 GiB. | A server build, proxy, gateway, or future provider may enforce a smaller ceiling. | Provider/deployment limit is startup configuration validated by contract tests. |
| Shape emission may serialize a source transaction into one provider append. | Merely lowering the cap can make large transactions unwriteable; arbitrary splitting can expose a partial source transaction. | Define bounded transaction framing and a client-visible transaction-end contract before splitting output. |
| HTTP errors are largely `anyhow` plus a few special types. | Correctness policy can accidentally depend on vendor status wording or collapse `413`, auth, redirect, gone, and ambiguous timeout. | Map transport responses to a closed typed error vocabulary. |

## Target boundary

Keep this inside `apps/engine`; a new workspace crate is unnecessary until a second in-process
consumer actually needs one.

```text
catalog / changelog / sequencer / lifecycle / output
                         |
                         v
              DurableStreams facade
       codec, retry budgets, reconciliation,
       retirement policy, durable byte accounting
                         |
                         v
              DurableStreamStore port
       one attempt; bytes + typed outcomes only
                         |
                         v
             HttpDurableStreams adapter
                         |
                         v
        pgxsinkit/durable-streams-rust endpoint

public shape response -> StreamReadLocator -> gateway/proxy or explicitly public DS URL
```

`DsClient` can remain the cloneable facade to minimize call-site churn, but should hold
`Arc<dyn DurableStreamStore>` rather than a concrete `reqwest::Client`. `Envelope`, catalog policy,
retry/backoff, and `GoneVerdict` stay above the port because those are Circuits semantics.

### Private port

The port should express the smallest single-attempt protocol Circuits uses:

```text
ensure(stream, content_type) -> Created | Existing
append(stream, content_type, bytes) -> AppendAck { next_offset }
read(stream, opaque_offset, mode, bounded_limit) -> ReadPage
head(stream) -> Missing | StreamHead { next_offset, closed, content_type }
close(stream) -> Closed | AlreadyClosed | Missing
delete(stream) -> Deleted | Missing
```

`ReadPage` carries the bytes plus typed protocol metadata such as `next_offset`, `up_to_date`, and
`closed`. The adapter may know HTTP headers; callers may not. Offset values are strings that can only
be stored, compared for equality, and sent back to the same store generation.

The closed error vocabulary should distinguish at least:

```text
Unavailable | AmbiguousWrite | Missing | Closed | Conflict | Unauthorized | Forbidden
TooLarge { configured_max } | UnexpectedRedirect | ProtocolViolation | Corrupt | Cancelled
```

The active adapter performs one bounded attempt and does not follow redirects. A redirect is a typed
configuration/protocol failure. The facade/engine decides whether an operation retries, waits,
reconciles, retires, or fails closed. This preserves the existing invariant that a live registered
shape's batch is either landed or the shape is durably retired.

### Required contract versus deployment capabilities

Correctness is not optional capability negotiation. Every production provider must pass:

- durable-before-ack under its declared failure model;
- ordered, atomic append visibility;
- opaque resume tokens;
- terminal close that releases a long poll;
- idempotent/retry-classifiable create and delete;
- exact JSON envelope order and boundaries;
- bounded requests, reads, and waits.

Deployment facts are explicit startup configuration, verified by tests and probes:

- maximum append body bytes;
- maximum bounded read page/chunk;
- redirect policy (`reject` for the active ds-rust adapter);
- credential source;
- internal endpoint and public read locator;
- stable store/namespace generation id;
- provider profile name and immutable build/image digest for evidence only.

Do not introduce provider feature branches such as `if pico { ... }` into the sequencer or lifecycle.
When both products implement the same behavior, the HTTP adapter is the same. A profile supplies
bounds and deployment wiring; a provider-specific adapter is justified only by a real wire
difference.

## Store identity and safe switching

“Swappable” means replaceable behind a proven contract, not hot-swappable while traffic is flowing.

Add a durable `StoreBound` record before any other catalog state:

```text
StoreBound {
  store_id,          // operator-created stable UUID for this logical store
  namespace,
  generation,
  protocol_version,
  created_at
}
```

The engine receives the expected `store_id` and namespace separately from the endpoint URL. A URL may
change during ordinary replacement; identity may not. At boot:

1. read and validate `StoreBound` before folding or creating any shape;
2. reject a mismatched id, namespace, or unsupported generation;
3. reject an unexpectedly empty store when the configured replication slot/lineage already exists;
4. require an explicit initialize/reset command to bind empty storage;
5. persist the store generation beside every opaque offset so an offset can never be replayed
   against another provider generation.

For the initial release, provider migration is a reset workflow: stop writes/ingest at a causal
fence, durably retire old shape streams, bind a new store generation and replication epoch, and force
clients to replace handles and re-snapshot from PostgreSQL. A future zero-reset migration must copy
the catalog, every retained change segment, every active/dormant shape stream, terminal state, and
cursor mapping, then prove equality before cutover. It is not part of this punchlist.

## TDD implementation punchlist

Each work item starts with a failing externally observable test. No item is complete merely because a
mock passes; the relevant scenario must run against the real pinned ds-rust process.

### `DSP-001` — Freeze the provider contract against ds-rust

- Add a provider conformance harness that creates a fresh namespace and executes the common matrix
  below against the currently pinned ds-rust image/binary.
- Capture raw HTTP characterization only inside the adapter tests; engine tests assert typed
  outcomes.
- Done when current behavior is green without product behavior changes and every engine-used
  operation has a contract case.

### `DSP-002` — Extract the private port without behavior change

- Make `DsClient` a semantic facade over an injected `Arc<dyn DurableStreamStore>`.
- Move only single-attempt HTTP mechanics into `HttpDurableStreamsStore`.
- Keep codec, retries, gone reconciliation, retirement, and catalog decisions in Circuits.
- Add a deterministic scripted store for fault/cancellation tests; do not use it as production
  evidence.
- Done when `DSP-001` and all existing engine/conformance suites remain green.

### `DSP-003` — Introduce typed outcomes and ambiguous-write handling

- Write red tests for 3xx, 401, 403, 404, 409 closed, 410, 413, 429, 5xx, timeout before headers,
  disconnect after commit/before response, invalid headers, and corrupt JSON.
- Map HTTP/vendor details once in the adapter. Preserve the original cause for diagnostics without
  letting callers branch on strings.
- Prove that an ambiguous append never advances the sequencer/catalog checkpoint until replay or
  reconciliation establishes a safe result.

### `DSP-004` — Separate internal store endpoint from public read location

- Add explicit internal DS URL and public stream-read base/proxy configuration, retaining the old
  environment variable as a documented transition alias.
- Ensure shape responses never expose internal credentials or topology.
- Add E2E cases for internal-only ds-rust and for a public locator different from the write endpoint.

### `DSP-005` — Bind store identity to the engine epoch

- Add `StoreBound` catalog schema/fold behavior and explicit initialization/reset flow.
- Red cases: wrong endpoint with empty storage, wrong namespace, same URL with replaced volume,
  restored older generation, and a cursor from a prior generation.
- Boot must fail closed without creating a slot, stream, or new catalog in every mismatch case.

### `DSP-006` — Make append/read bounds provider-neutral

- Replace the 1 GiB constant with validated deployment bounds.
- Red cases at `max-1`, `max`, and `max+1`, including an engine configuration with an 8 MiB ceiling
  so the engine cannot accidentally rely on the current 1 GiB server default.
- Bound response bytes, JSON records, long-poll time, and concurrent in-flight appends. Export
  saturation/rejection metrics.

### `DSP-007` — Remove byte-position offset assumptions

- Treat offsets as opaque everywhere outside the adapter.
- Persist per-segment logical appended bytes in the Circuits catalog atomically with rotation
  progress and restore it before retention can delete anything.
- Red cases use decimal, composite, and deliberately non-monotonic-looking tokens and restart in the
  middle of a segment. No policy may parse or lexically order provider offsets.

### `DSP-008` — Bound large output transactions without partial visibility

- First write an E2E red test for one source transaction whose encoded shape output exceeds 8 MiB.
- Define/stamp a transaction-end frame on output streams and make the Swift/TS materializers hold an
  incomplete transaction across provider pages/appends.
- Split only at envelope boundaries, never expose a partial source transaction as committed, and
  advance source/checkpoint state only after the final append lands.
- Cover cancellation, restart, ambiguous final append, and one transaction spanning three or more
  provider appends.

### `DSP-009` — Qualify the ds-rust production profile

- Pin the owned binary/image by version and digest; close or explicitly waive the two 0.3.6 gaps only
  if the supported Circuits surface cannot reach them.
- Run full upstream server conformance plus the Circuits provider and engine E2E suites in WAL mode.
- Fault cases: process `SIGKILL`, host restart simulation, torn/corrupt WAL, disk full, read-only
  volume, fsync/I/O failure, volume restore, and engine/provider restart order.
- Publish exact volume, backup, restore, capacity, alert, and no-HA statements.

### `DSP-010` — Provider-neutral telemetry and operations

- Emit bounded OpenTelemetry spans/metrics from the Circuits facade: append/read/close/delete
  latency and outcome, retries, ambiguous writes, bytes, in-flight work, queue wait,
  checkpoint lag, and retirement backlog. Never label with raw stream, shape, tenant, or SQL values.
- Accept the configured OTLP URL and auth headers through the existing Circuits telemetry boundary.
- Add dashboards/alerts that work for either provider; provider-native metrics are supplemental.
- Exercise trace propagation through engine -> store and verify secrets are absent from exported
  attributes/events.

### `DSP-011` — Make provider support a release matrix

- Publish exact supported combinations: adapter contract version, provider/version/digest, bounds,
  durability mode, topology, auth mode, and qualification evidence.
- CI runs the common provider suite against the supported ds-rust profile. The matrix schema can
  accept another provider later, but there is no second-provider CI lane in the active scope.
- Upgrade tests run old -> new, new -> old rollback where supported, backup/restore, wrong-image
  startup, and config alias migration.

## Common black-box contract matrix

| Contract | Minimum red/green scenario |
| --- | --- |
| Create | First create, repeated identical create, content-type conflict, initial body, cancellation at response boundary. |
| JSON | Single object and array append; exact flatten/order; empty/invalid body; values around request bound. |
| Append durability | Ack, kill provider immediately, restart, resume from prior offset, verify all acknowledged and no unacknowledged record is assumed present. |
| Atomic visibility | A concurrent reader sees none or all of one append, including append near the body ceiling. |
| Offsets | Resume at start/middle/tail; stale token; token from another store generation; arbitrary opaque token format. |
| Long poll | Wake on append; timeout; wake on close; cancellation; reconnect with the returned token. |
| Close | Close-only request, repeat close, append after close, read closed tail, delete closed stream. |
| Delete | Existing/missing/repeated delete; reader during delete; create same path after delete according to protocol. |
| Error mapping | 3xx rejection, 401/403/404/409/410/413/429/5xx, malformed response, connection reset, and timeout. |
| Ambiguous append | Provider commits and drops response; engine retries/reconciles; consumer converges without loss and without applying a source effect twice. |
| Bounds | Exact body/read/record/redirect/concurrency boundaries and observable rejection/saturation. |
| Catalog promise | `Created`, new `Joined`, `Left`, and client `Dropped` remain durable-before-ack through provider outage and request cancellation. |
| Changelog | Chunked source transaction, segment rotation, restart mid-segment, dormant pin, deletion floor, and opaque offsets. |
| Provider identity | Wrong empty backend, restored-old backend, namespace collision, URL-only replacement, explicit generation reset. |

## Local validation performed for this analysis

- Inspected Electric Circuits' current `ds.rs`, `changelog.rs`, `txn_buffer.rs`, output/lifecycle
  call sites, deployment docs, and durable-store invariants.
- As deferred research only, inspected PicoMQ revision
  `b5861ff03c1801ee913c168851baff033fa8f441`, including architecture,
  writes, deployment/auth documentation, the DS frontend, ownership routing, metadata transitions,
  and open issues.
- As deferred research only, ran `cargo test -p pico-frontend --test ds_http` in a fresh PicoMQ
  clone: **5 passed, 0 failed**.
  These tests cover the official Rust Durable Streams client, basic JSON/live behavior,
  close/delete, and producer fencing; they are not the full upstream server-conformance suite.
- Inspected ds-rust revision `2c1382a32d6962f6fefc513c7c625746e03fc526`, its provenance,
  conformance report, crash-simulation report, deployment options, and open issues.
- Queried current GitHub issue/release state on 2026-08-25. PicoMQ listed six open feature issues and
  no release; ds-rust listed the two inherited 0.3.6 conformance gaps and no release. PicoMQ state is
  not an active dependency or release input.

No Electric Circuits product code was changed by this analysis. The next implementation work item is
`DSP-001`, followed by the behavior-preserving extraction in `DSP-002`.
