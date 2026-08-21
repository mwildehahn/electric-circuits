# Deploying electric-circuits with Postgres

electric-circuits can run **Postgres as the system of record**: your application writes to Postgres
normally, and electric-circuits keeps every declared shape (a filtered view of a table) live by ingesting
changes from Postgres **logical replication** and reading rows back from Postgres for backfill. There
is no separate write API and no in-memory table copy to keep in sync — Postgres is the source of truth.

```
  app ──writes──▶  Postgres  ──logical replication──▶  engine  ──append──▶  durable-streams
                     ▲                                   │                    (shape/<id>)
                     └──────────── backfill SELECT ──────┘                         │
                                                                                   ▼
                                                                          client (live rows)
```

## What you need

- **Postgres 10+** with logical decoding (the built-in `pgoutput` plugin — no extensions to
  install). Managed Postgres works if it allows `wal_level = logical` and a logical replication slot
  (RDS, Cloud SQL, Supabase, Neon, etc. all do).
- **A durable-streams server** (the transport/persistence layer). Set its base URL in
  `ELECTRIC_CIRCUITS_DS_URL`.
- **The engine binary** (`apps/engine`, Rust): `cargo build -p electric-circuits-engine --release` →
  `target/release/electric-circuits-engine`.

## Step 1 — Configure Postgres

Logical replication must be on. In `postgresql.conf` (or your provider's parameter group):

```conf
wal_level = logical
max_replication_slots = 10     # ≥ number of engine instances
max_wal_senders = 10
```

Then restart Postgres (the `wal_level` change requires a restart).

The engine sets everything else up for you on startup, per configured table:

- `ALTER TABLE <t> REPLICA IDENTITY FULL` — so an UPDATE/DELETE carries the **full old row** (needed to
  compute the exact delta). The role you connect with must own the tables (or be superuser) for this.
- `pg_create_logical_replication_slot('<slot>', 'pgoutput')` + `CREATE PUBLICATION <slot>_pub FOR
  ALL TABLES` (superuser) — the replication slot and its publication, created once
  and reused.

> **Each table needs a single-column primary key.** The engine introspects columns, types, and the pk
> from the catalog; composite primary keys are not supported.

> **One slot per engine instance.** Replication-slot names are unique across the whole Postgres
> instance. If you run more than one engine against the same database, give each a distinct
> `ELECTRIC_CIRCUITS_PG_SLOT`.

## Step 2 — Run the engine

Point it at Postgres, list the tables to watch, and give it the durable-streams URL:

```sh
export ELECTRIC_CIRCUITS_DS_URL="https://streams.internal:8080"
export ELECTRIC_CIRCUITS_PG_URL="postgres://user:pass@db.internal:5432/appdb"
export ELECTRIC_CIRCUITS_PG_TABLES="users,projects,reporting.tasks"
export ELECTRIC_CIRCUITS_BIND="0.0.0.0:9000"

./electric-circuits-engine
```

On startup the engine sets `REPLICA IDENTITY FULL` on each table, introspects it, ensures the slot, starts
the replication ingestor, and begins serving the control API on `ELECTRIC_CIRCUITS_BIND`. It prints
`ENGINE_LISTENING <addr>` once ready.

### Configuration reference

| Variable                  | Required | Default          | Meaning |
|---------------------------|:--------:|------------------|---------|
| `ELECTRIC_CIRCUITS_DS_URL`    | yes      | —                | durable-streams base URL. |
| `ELECTRIC_CIRCUITS_PG_URL`    | yes¹     | —                | Postgres connection string. Setting it enables Postgres mode. |
| `ELECTRIC_CIRCUITS_PG_TABLES` | yes¹     | (empty)          | Comma-separated tables to watch: `schema.name`, a bare `name` (= `public.<name>`), or `schema.*` / `*` for every table with a primary key in that schema. `*` and an empty setting both mean `public.*` — never every schema. |
| `ELECTRIC_CIRCUITS_PG_SLOT`   | no       | `electric_circuits`  | Logical replication slot name (unique per engine). |
| `ELECTRIC_CIRCUITS_PG_POLL_MS`| no       | —                | Legacy; accepted but unused (the ingestor streams pgoutput, push delivery). |
| `ELECTRIC_CIRCUITS_BIND`      | no       | `127.0.0.1:0`    | Address for the control/HTTP API. |
| `ELECTRIC_CIRCUITS_LOG`       | no       | `info`           | Log filter (`error`, `warn`, `info`, `debug`). |

¹ Omit `ELECTRIC_CIRCUITS_PG_URL` to run in library/no-source mode (shapes start empty; used by tests).

## Step 3 — Connect the client

The client subscribes to shapes over the engine's API and materializes them with TanStack DB.
Writes go to **Postgres**, not the client:

```ts
import { createClient } from '@electric-circuits/client'

const client = createClient({ apiUrl: 'http://engine.internal:9000', schema })

// Declare a shape; rows stay live as Postgres changes.
const activeUsers = await client.shape({
  table: 'users',
  where: { col: 'active', op: 'eq', value: true },
})

activeUsers.subscribe((rows) => render(rows))

// To change data, write to Postgres however you already do (psql, your ORM, etc.):
//   UPDATE users SET active = false WHERE id = 42;
// electric-circuits picks it up via logical replication and updates the shape.
```

## Operating notes

- **Adding a table:** add it to `ELECTRIC_CIRCUITS_PG_TABLES` and restart the engine. It will set
  replica identity on the new table and introspect it at startup.
- **Replication slot lag:** an engine that is stopped for a long time holds its slot, and Postgres
  retains WAL for it. If you decommission an engine, drop its slot:
  `SELECT pg_drop_replication_slot('<slot>');` Monitor `pg_replication_slots.confirmed_flush_lsn` vs
  `pg_current_wal_lsn()` to watch lag.
- **Consistency:** on shape registration the engine takes a `REPEATABLE READ` snapshot of the
  matching rows (the backfill) and, atomically with it, captures the snapshot's
  `pg_current_snapshot()` — the **snapshot gate**. Each replicated change is stamped with its
  transaction's **commit LSN, xid, and in-transaction position**, and the engine skips a change iff
  its xid was **visible to the backfill snapshot** (already in the seed); everything else is taken
  from the live stream. Visibility — not WAL position — is the fence because a commit's WAL record
  exists before the transaction becomes snapshot-visible; an LSN-only comparison would drop rows in
  that window. Ingest delivery is at-least-once (append, then advance the slot), and the engine
  de-duplicates by `(commit LSN, position)`, so each change takes effect exactly once. This assumes a
  single ingestor per database (the model above). Running multiple ingestors over the same tables is
  not supported.
- **Migrations cost one resync of that table's shapes.** The engine compares every table's compiled
  schema with what Postgres reports — on each pgoutput `Relation` message (which Postgres re-sends
  after DDL) and on a background reconciler tick (`ELECTRIC_CIRCUITS_SCHEMA_RECONCILE_SECS`, default
  60 s, for DDL that no write follows, and for primary-key changes, which the replication stream
  cannot describe). On any difference — a column added, dropped, retyped or reordered, a changed
  primary key, or a `REPLICA IDENTITY` reset from `FULL` (which the engine re-asserts) — it
  re-introspects the table and **retires every shape that depends on it**: the streams are closed and
  deleted, and clients re-subscribe and backfill through the new schema. Rows already on a stream can
  never gain a new column, so this is enforced rather than hoped for. `TRUNCATE` retires the same
  shapes (the engine holds no row copy from which to synthesise the deletes). Other tables' shapes
  are untouched. A shape creation that overlapped the migration is refused with
  `schema of <t> changed during creation; retry` rather than being served over the old schema.
  One exception: a table with a counts pipeline (`ELECTRIC_CIRCUITS_DBSP_COUNTS`) has no runtime
  circuit rebuild, so the engine additionally exits (code `75`) to be restarted once the retirements
  have landed — plan migrations *and truncates* on those tables like a rolling restart.
- **Migrating while the engine is stopped works too.** Every shape record stores the fingerprint its
  table had when the shape was created; at boot the engine re-introspects and retires any shape whose
  table moved while it was down, so a restart after a migration is a resync of the affected tables,
  never a silent resume over the old schema.
- **When a migration cannot be settled, the table is parked, not guessed at.** If the engine cannot
  re-introspect (Postgres unreachable) or cannot get the `ACCESS EXCLUSIVE` lock to restore
  `REPLICA IDENTITY FULL` within 5 s, the table becomes **unresolved**: its shapes are retired, its
  changes are dropped, and creates on it are refused with
  `schema of <t> is unresolved after a change; retry later`. A per-table retry task keeps working on
  it (2 s → 30 s backoff) and un-parks it on the first successful re-introspection. Watch
  `GET /tables` (each table's `unresolved` flag) and the `schema_unresolved_total` counter.
- **The publication must deliver whole rows.** A `Relation` message that describes fewer columns than
  the table has can never be reconciled with its schema, so the engine **refuses to boot** if its
  publication has a per-table column list, naming the tables. Stored generated columns follow
  `pg_publication.pubgencols` (PG18+): the engine includes them in a table's schema fingerprint
  exactly when the publication publishes them. The engine's own `<slot>_pub` is `FOR ALL TABLES` and
  satisfies both; point it at a hand-made publication only if it does too.
- **Dropping a table is one-way until a restart.** A table dropped from Postgres has its shapes
  retired and is untracked; re-creating it under the same name does not resume syncing — restart the
  engine, exactly as when adding a table.
- **Permissions:** the engine's Postgres role needs `SELECT` on the watched tables, ownership (for
  `ALTER TABLE … REPLICA IDENTITY`), and the `REPLICATION` attribute (to create/read the slot).
- **Introspection surface:** the engine's control port also serves the pipeline-visualizer backend —
  `/trace` (per-envelope SSE), `/graph`, `/state`, and the `/state/node` deep dumps, which expose row
  data. These endpoints are **unauthenticated**; anyone who can reach the control port can read them.
  The runtime cost is near zero while nobody subscribes (instrumentation is gated on subscriber
  count), so leaving them on is fine when the port is private. If the control port is reachable
  beyond your trust boundary, either front it with network policy or disable the surface outright
  with `ELECTRIC_CIRCUITS_TRACE=0` (the routes are then never registered).
