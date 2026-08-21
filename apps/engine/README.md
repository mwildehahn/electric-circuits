# electric-circuits-engine

The Rust engine at the center of [electric-circuits](../../README.md): a durable-streams client that
turns Postgres logical-replication changes into incrementally-maintained **shapes**, **subquery
inner-sets**, and **scalar aggregations** — one maintained stream per *distinct* definition,
ref-counted and shared across subscribers. It serves two HTTP surfaces from one process:

- the **control plane** (`/schema`, `/shapes`, `/aggregate`, `/query`, introspection), used by
  `@electric-circuits/api`;
- the **Electric-compatible `GET /v1/shape`**, so an unmodified ElectricSQL client can sync from it.

Design and execution model: [docs/ARCHITECTURE.md](../../docs/ARCHITECTURE.md) and
[docs/ivm-engine-internals.md](../../docs/ivm-engine-internals.md).

## Build & run

```bash
cargo build -p electric-circuits-engine          # or: pnpm engine:build (repo root)
cargo test  -p electric-circuits-engine          # or: pnpm engine:test

ELECTRIC_CIRCUITS_DS_URL=http://127.0.0.1:8791 \
ELECTRIC_CIRCUITS_PG_URL=postgres://postgres@127.0.0.1:5432/postgres \
ELECTRIC_CIRCUITS_PG_TABLES='*' \
target/debug/electric-circuits-engine
```

The engine prints `ENGINE_LISTENING <url>` to **stdout** (logs go to stderr) so a harness can
discover the bound port.

## Environment

| Var | Default | Meaning |
|---|---|---|
| `ELECTRIC_CIRCUITS_DS_URL` | *(required)* | Durable-streams server base URL (the change log) |
| `ELECTRIC_CIRCUITS_PG_URL` | *(unset)* | Enables **Postgres mode**: ingest via logical replication, backfill by query-back. Unset = library mode (writes arrive on table streams) |
| `ELECTRIC_CIRCUITS_PG_TABLES` | *(empty)* | Comma list of tables to replicate: `schema.name`, a bare `name` (= `public.<name>`), or `schema.*` for every table with a primary key in that schema. `*` (or empty) = `public.*` — never every schema (introspect-all sets `REPLICA IDENTITY FULL`, which must not touch managed system schemas) |
| `ELECTRIC_CIRCUITS_PG_SLOT` | `electric_circuits` | Logical replication slot name |
| `ELECTRIC_CIRCUITS_PG_POLL_MS` | `50` | Replication-slot poll interval |
| `ELECTRIC_CIRCUITS_BIND` | `127.0.0.1:0` | Bind address (`:0` = ephemeral port) |
| `ELECTRIC_CIRCUITS_LOG` | `info` | `tracing` EnvFilter (e.g. `warn`, `electric_circuits_engine=debug`) |
| `ELECTRIC_CIRCUITS_TRACE` | `1` (on) | `0`/`false`/`off` unregisters the introspection surface (`/trace` SSE, `/graph`, `/graph/node`, `/state`, `/state/node` — the pipeline-visualizer backend). When on, it costs ~nothing until a client subscribes (and stays unauthenticated — see the deployment doc) |
| `ELECTRIC_CIRCUITS_SHAPE_IDLE_SECS` | `1800` | Retention: idle time (no engine-visible reads, refcount 0) before an active shape goes **dormant** (engine state dropped; stream + record retained). `0` disables dormancy |
| `ELECTRIC_CIRCUITS_SHAPE_DORMANT_TTL_SECS` | `604800` (7 days) | Retention: how long a shape may stay dormant before it is **evicted** (stream + record deleted). `0` disables the TTL layer |
| `ELECTRIC_CIRCUITS_MAX_SHAPES` | `10000` | Retention: total shape-count cap; over it, least-recently-read **dormant** shapes are evicted (active shapes never are). `0` = unlimited |
| `ELECTRIC_CIRCUITS_SHAPE_DISK_BUDGET_MB` | `0` (disabled) | Retention: cap on shape-stream bytes (engine-side accounting of appended bytes — resets on restart); over it, least-recently-read dormant shapes are evicted |
| `ELECTRIC_CIRCUITS_RETENTION_SWEEP_SECS` | `60` | Retention: background sweep interval |
| `ELECTRIC_CIRCUITS_SCHEMA_RECONCILE_SECS` | `60` | Schema drift: how often the engine fingerprints every tracked table against the Postgres catalog, to catch DDL that no write follows. `0` disables the reconciler (the pgoutput triggers still fire) |
| `ELECTRIC_CIRCUITS_RESET_ON_SLOT_LOSS` | `true` | What to do when the replication slot can no longer be trusted (see "Replication slot and epochs"): `true` (Electric parity) retires every shape, binds a new epoch and carries on; `0`/`false`/`off`/`no` refuses instead — ingest stops, shape routes answer 503, and `POST /epoch/reset` is the operator's recovery |
| `ELECTRIC_HANDLE_TTL` | `600` | Seconds a `/v1/shape` handle may sit idle before its **handle state** is evicted and its shape subscription released (the shape + stream are retained and follow the retention lifecycle); a late request gets `409 must-refetch` and rejoins the retained shape |
| `ELECTRIC_LIVE_TIMEOUT_MS` | `20000` | Overall deadline for a `live=true` `/v1/shape` long-poll, then `204` |

### Benchmarking-fleet surface (`ELECTRIC_*`)

The engine also accepts Electric's own env surface so the `electric-circuits` image is a drop-in for
`electricsql/electric` in the [benchmarking-fleet](../../docs/fleet-conformance.md). These are resolved
in `config.rs`; the `ELECTRIC_CIRCUITS_*` vars above always **win** (dev/test behavior is unchanged). Any
unknown `ELECTRIC_*` var is accepted and logged once as "accepted (no-op)" — it never crashes boot.

| Var | Default | Meaning |
|---|---|---|
| `DATABASE_URL` | *(unset)* | Postgres URL (tolerates `?sslmode=disable`); `ELECTRIC_CIRCUITS_PG_URL` wins |
| `ELECTRIC_PORT` | `3000` when set / under `DATABASE_URL` | Binds `0.0.0.0:<port>`; `ELECTRIC_CIRCUITS_BIND` wins |
| `ELECTRIC_LOG_LEVEL` | `info` | `error`/`warning`/`info`/`debug` → log filter; `ELECTRIC_CIRCUITS_LOG` wins |
| `ELECTRIC_REPLICATION_STREAM_ID` | *(unset)* | Slot name `electric_slot_<id>`; also the `stack_id` metric tag |
| `ELECTRIC_INSTANCE_ID` | generated UUID | Tags every StatsD metric `instance_id:<id>` |
| `ELECTRIC_STATSD_HOST` | *(unset → StatsD off)* | `host[:port]` (default port 8125) StatsD destination |
| `TELEMETRY_POLLER_PERIOD` / `ELECTRIC_SYSTEM_METRICS_POLL_INTERVAL` | `5s` | Periodic-metrics interval (ms / human duration; the latter wins) |
| `ELECTRIC_SECRET` | *(unset)* | If set, `/v1/shape` requires `secret`/`api_secret` query param (else `401`) |
| `ELECTRIC_INSECURE` | *(unset)* | Accepted; no-op when no secret |
| `ELECTRIC_STORAGE_DIR` | *(unset)* | If set + exists, `du`'d every ~60s → `electric.storage.used.bytes` |

**`GET /v1/health`** reports the boot state machine as an exact, whitespace-free JSON body:
`{"status":"waiting"}` (202) until Postgres connects, `{"status":"starting"}` (202) through
introspection/slot/ingest spawn, then `{"status":"active"}` (200). Library mode is `active` at once.
`{"status":"degraded"}` (503) outranks all of them — see **Degraded** below.
`GET /` → 200 empty; `OPTIONS /v1/shape` → 204 with `access-control-allow-methods`.

**Degraded (fail-closed).** A subquery flip's Postgres query-back is retried (resuming the DAG walk
where it stopped, not restarting it). If the retries are exhausted the effects that batch was
carrying are lost — the inner-set node moved before the query-back ran, so nothing will re-derive
them. The engine then refuses to pretend otherwise: the batch never decrements `pendingFlips` (so
the convergence barrier `sync caught up + offsets at tail + pendingFlips == 0` now means *every
computed effect has landed, or the engine is degraded and says so*), `GET /replication/lsn` reports
a non-zero `flipFailures`, `/v1/health` turns `degraded` (503), and `POST /shapes`, `POST /aggregate`,
`POST /query`, `GET /shapes/{id}(/rows|/log)` and `GET /v1/shape` answer 503 with
`{"error":"degraded: …"}`. Every subquery shape's durable stream is retired too — closed, then
deleted — clients read durable-streams directly, past the HTTP surface, so that is the only way they
learn; the close releases a tailing long-poll at once with `stream-closed`. Observability
(`/replication/lsn`, `/metrics*`, `/memory`, `/subqueries`, `/graph`, `/state`, `/trace`, `/tables/*`,
`/health`) stays up. **Recovery is a restart**, which re-seeds every node from Postgres. A restart
*drops* every subquery shape — their inner-node state is not persisted, so the catalog restore
deliberately does not restore them — and clients recreate them with `POST /shapes`. Deleting the
streams therefore destroys nothing a restart would have kept.

**StatsD telemetry** (`statsd.rs`) is the fleet's only metrics channel — the datadog wire format
(`name:value|type|#instance_id:<id>,...`), non-blocking (bounded channel → batched ≤1432-byte UDP
datagrams), off unless `ELECTRIC_STATSD_HOST` is set. It emits a periodic system-metrics table
(`system.*`/`vm.*`, sampled with `sysinfo`) plus event metrics at the HTTP, replication, storage, and
snapshot paths. Only genuinely-measured values are emitted; anything unmeasurable on the host is
omitted, never faked. The existing `GET /metrics` (JSON) + `GET /metrics/prometheus` (OTel) are
unchanged.

## HTTP endpoints

| Route | Purpose |
|---|---|
| `GET /health` | liveness |
| `POST /schema` | define the schema (library mode; Postgres mode self-configures by introspection) |
| `POST /shapes` | create a shape (`table`, `where`, `columns`, `changesOnly`) — identical definitions share one stream |
| `POST /aggregate` | create a live scalar aggregation (`table`, `where`, `fn`, `col`) |
| `GET /shapes/{id}` / `DELETE /shapes/{id}` | look up a shape (incl. its retention `state`) / release one subscription — the shape is retained and ages through the retention lifecycle. `DELETE …?purge=true` force-drops it immediately (admin/debug; the visualizer's trash) |
| `GET /shapes/{id}/rows` | current contents of an existing shape (folds its stream; visualizer preview) |
| `GET /shapes/{id}/log` | tail of a shape's stream as-is (op/key/value/lsn) — the visualizer's feed change log |
| `POST /query` | one-shot subset query: `SELECT … ORDER BY … LIMIT/OFFSET` + snapshot LSN |
| `GET /trace` | SSE: per-envelope pipeline traces (hops + outcomes) and `shapeAdded`/`shapeDropped` lifecycle events; lossy by design, zero cost with no subscribers |
| `GET /tables` | every tracked table + its schema-drift `unresolved` flag |
| `GET /tables/{name}/offset` · `GET /tables/{name}/families` | tailer position / routing-family stats |
| `GET /subqueries` · `GET /graph` · `GET /graph/node?sig=…` | shared-node stats, pipeline graph, one node's live index |
| `GET /replication/lsn` | ingestor LSN + sync status + `pendingFlips` / `flipFailures` (the convergence barrier) + the `epoch` object (slot binding + `state`/`reason`) |
| `POST /epoch/reset` | operator recovery from a broken epoch under `ELECTRIC_CIRCUITS_RESET_ON_SLOT_LOSS=false`: retire every shape, bind a new epoch, resume ingest (409 if the epoch is not broken) |
| `GET /metrics` · `POST /metrics/reset` · `GET /memory` · `GET /metrics/prometheus` | counters/histograms, memory snapshot, OTel/Prometheus exposition |
| `GET /v1/shape` | Electric protocol: snapshot (`offset=-1`), live long-poll, handles/offsets/`must-refetch` |

The `/v1/shape` adapter parses Electric's SQL `where` grammar and is validated against Electric's own
oracle/property/integration tests ([electric-conformance/](../../electric-conformance/README.md)).

**Creating a subquery shape** (`POST /shapes` with an `IN (SELECT …)` predicate) registers the
shape's dependency edges before it reads Postgres, so a membership change can reach it mid-create:
work aimed at the not-yet-installed shape is queued on the pending create, and work aimed at a
parent inner-set node this create is still seeding is queued on that node — both replayed (and, for
a node, walked on down the graph) the moment the seed and the shape are in. The create's rollback
state stays registry-owned across the whole install, so a client disconnect at any point — a
partly-installed membership seed included — is unwound exactly and the same shape is immediately
creatable again.

## Schema changes

The engine never keeps serving rows over a schema Postgres no longer has (`docs/adr/0005-schema-drift-retires-per-table.md`).
The compiled schema of each table carries a fingerprint — its live columns in `attnum` order with
`(name, type OID, typmod)`, plus `relreplident` and the primary key — and four things are compared
against it: the pgoutput `Relation` message Postgres re-sends after any DDL, that message's replica
identity, `TRUNCATE`, and the background reconciler (`ELECTRIC_CIRCUITS_SCHEMA_RECONCILE_SECS`, for
DDL that no write follows). (The `Relation` message cannot describe a primary key — under
`REPLICA IDENTITY FULL` every column is flagged as part of the identity — so a PK change is caught by
the reconciler, not on the wire.)

**Schema drift** (a column added, dropped, retyped or reordered; a PK change; an identity that
regressed from `FULL`, which is re-asserted first) re-introspects the table, **retires every
dependent shape** — including aggregates and any subquery shape whose predicate references it — by
closing then deleting their streams, swaps the compiled schema everywhere, and records
`schemaChanged` in the durable catalog. **`TRUNCATE`** retires the same dependents and stops there:
nothing about the schema changed, so there is nothing to re-introspect, swap or record. Clients treat
the closed stream exactly as they treat eviction: re-subscribe, and the new shape backfills through
the new schema. A create that was already in flight when a drift retired its table is refused
(`schema of <t> changed during creation; retry`) and rolled back rather than installed against a
schema that is gone.

Granularity is per table: a migration on one table never resyncs another. The one exception is a
table with a counts pipeline (`ELECTRIC_CIRCUITS_DBSP_COUNTS`) — the circuit is built and seeded once
at boot with no runtime rebuild, so once the retirements and catalog records have landed the process
exits `75` to be restarted; boot re-seeds the circuit and the catalog restores every other table's
shapes. This applies to `TRUNCATE` as much as to drift (a truncate emits no per-row envelopes, so the
pipeline would otherwise keep its pre-truncate groups).

**Migrations applied while the engine is down** are seen by nothing on the live path. Each shape
record carries the fingerprint its table had when the shape was created, and the catalog restore
retires any shape whose table no longer matches — its retained stream holds rows shaped by the old
schema and can never be brought up to date.

**Unresolved tables.** If the drift cannot be settled — Postgres unreachable, a catalog read that
errored, or an `ALTER … REPLICA IDENTITY FULL` that could not get its `ACCESS EXCLUSIVE` lock within
5 s (the wait is bounded so one long reader cannot stall *all* ingest) — the table is parked as
**unresolved**: dependents retired, its changes dropped, and `POST /shapes` / `POST /aggregate` on it
refused with `schema of <t> is unresolved after a change; retry later`. A per-table retry task keeps
attempting the resolution (2 s → 30 s backoff) regardless of the reconciler setting, and the first
successful re-introspection un-parks it. `GET /tables` lists every tracked table with its
`unresolved` flag, and `schema_unresolved_total` counts the parkings.

**Publication requirements.** The engine needs **whole rows** on the wire: a `Relation` message that
describes fewer columns than the catalog holds can never be reconciled with the table's schema. At
boot the engine therefore refuses to start if its publication carries a per-table **column list**, and
reads `pg_publication.pubgencols` (PG18+) so that **stored generated columns** are included in the
schema fingerprint exactly when the publication publishes them. The engine's own `<slot>_pub` is
`FOR ALL TABLES`, which satisfies both.

A table that is **dropped** has its dependents retired and is untracked; a table re-created under the
same name is not synced again until the engine restarts (same as adding a table).

## Replication slot and epochs

The engine records **which slot, in which cluster, it is bound to** (`docs/adr/0004-slot-epoch-and-reset.md`).
The first time it creates — or adopts — its slot it writes a `slotBound`
(`system_identifier`, `timeline_id`, `slot`, `bound_at`) to the durable catalog; the last such record
is the current **epoch**, and every shape in that catalog belongs to it. Before *every* connection —
at boot and on each ingestor reconnect, not just at boot — the slot is verified against that binding.

A durable catalog the engine cannot **read** at boot is fatal, by design: it is not a catalog with no
epoch in it, and treating it as one would create a slot at the current WAL head and orphan every shape
already in the log. The engine refuses to start until storage is healthy.

An **epoch break** is a slot the engine can no longer vouch for: it is gone, `wal_status = 'lost'`,
it is there under a different output plugin, or `pg_control_system().system_identifier` is not the
one the binding names. There is no way to fill the resulting gap — the changes between the old slot's
`confirmed_flush_lsn` and the current WAL head simply are not available — so every shape over it is
wrong. (Two look-alikes that are **not** breaks: a slot held by another walsender, which is a second
engine on the same slot and is waited out; and a `timeline_id` that moved, which is logged and
recorded but not acted on — one primary, no promotion, per the ADR.)

- **`ELECTRIC_CIRCUITS_RESET_ON_SLOT_LOSS=true`** (default, Electric parity): the engine **resets**.
  Every shape — active and dormant — is retired (stream closed, then deleted; ADR-0007), the slot is
  dropped and recreated, and a new `slotBound` is written. That record *is* the new epoch. Clients see
  closed streams and re-subscribe, exactly as for eviction or schema drift. An unattended deployment
  heals itself, at the cost of an unscheduled backfill storm.
- **`…=false`**: the engine **refuses**. Ingest does not start, `/v1/health` reports `degraded` (503),
  every shape route answers 503, and `GET /replication/lsn` names the reason
  (`epoch.state = "broken"`, `epoch.reason` one of `slot_lost` / `slot_wal_lost` /
  `system_identifier_mismatch`). Nothing is destroyed while refusing. Recovery is a deliberate act:
  `POST /epoch/reset` runs exactly the reset above and resumes ingest.

Reconnects (and refusals) back off exponentially with jitter, 1 s → 30 s. The schedule resets only
when a connection actually **delivered** — a `START_REPLICATION` the server rejected (the slot is held
by another walsender, say), an auth failure or a `pg_hba` rule all climb to the ceiling rather than
retrying at the floor. An operator's reset cuts the wait short rather than leaving the engine asleep. `epoch_breaks_total` and `epoch_resets_total`
count both halves. The slot NAME never changes, so the StatsD slot gauges keep reporting across a
reset. A reset while a counts pipeline is running exits `75` to be restarted, for the same reason
schema drift on a circuit-served table does: the circuit is seeded once at boot and has no runtime
rebuild across the gap.

## Shape retention lifecycle

Shapes follow a three-tier lifecycle (`src/retention.rs`) instead of delete-on-last-unsubscribe —
a deliberate divergence from upstream Electric, which keeps every retained shape actively
maintained:

- **Active** — maintained live. Unsubscribing (`DELETE /shapes/{id}`, `/v1/shape` handle expiry)
  does not deactivate; brief reconnects rejoin the same warm stream.
- **Dormant** — after `ELECTRIC_CIRCUITS_SHAPE_IDLE_SECS` with no reads and no subscribers: engine
  routing state is dropped, the durable stream and shape record are retained at zero engine cost.
  Any touch (rejoin, `/v1/shape` re-snapshot, rows/log read) reactivates by replaying the
  `table/<name>` stream from the captured resume offset — no Postgres backfill.
- **Evicted** — record deleted and the stream **retired**: closed, then deleted (see
  `docs/adr/0007-retirement-closes-before-delete.md`), so a client tailing it is released at once
  with `stream-closed` rather than blocking to the long-poll timeout. `/v1/shape` clients get
  `409 must-refetch` (the adapter turns a closed stream into that) and re-snapshot; extended-API
  clients **must** treat `stream-closed`, `404` and `410` alike: re-subscribe. (A **dormant**
  shape's stream is never closed — reactivation appends to it.)

Eviction is layered, least-recently-read first, and **dormant-only** (active shapes are never
evicted): the dormancy TTL (hygiene), the `ELECTRIC_CIRCUITS_MAX_SHAPES` count cap (engine cost bound),
and the disk budget (hard backstop). When a cap/budget is exceeded with nothing dormant to evict,
the engine logs loudly and bumps the `retention_pressure` metric instead of evicting.

Subquery and aggregate shapes never go dormant (their state is not rebuildable from a bounded
replay); once unsubscribed, the TTL layer instead evicts them straight from active after the same
total grace an ordinary shape gets (idle timeout + dormancy TTL). Lifecycle state is in-memory
today — restart recovery (persistent catalog, GH #8) will persist it.
