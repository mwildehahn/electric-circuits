# Sources-table mode

Sources-table mode lets one engine host serve multiple PostgreSQL sources. The host owns no
source registry: the rows in the configured control relation are the complete desired set. Each
source has its own engine, replication slot, worker thread, and storage directory.

## Configuration

`ELECTRIC_CIRCUITS_SOURCES_MODE` is opt-in. Without it, the existing single-source boot path and
its `ELECTRIC_CIRCUITS_PG_*` settings are unchanged. Set it to `table` for PostgreSQL control
tables or `file` for local development.

| Setting | Default | Meaning |
| --- | --- | --- |
| `ELECTRIC_CIRCUITS_SOURCES_MODE` | — | `table` or `file`. |
| `ELECTRIC_CIRCUITS_SOURCES_PG_URL` | — | Required in `table` mode; the control database URL. |
| `ELECTRIC_CIRCUITS_SOURCES_TABLE` | `circuits_sources` | Control row table; a simple or schema-qualified identifier. |
| `ELECTRIC_CIRCUITS_SOURCES_VERSION_TABLE` | `circuits_sources_version` | Single-row revision table. |
| `ELECTRIC_CIRCUITS_SOURCES_POLL_SECS` | `30` | Version-row poll interval. Must be positive. |
| `ELECTRIC_CIRCUITS_SOURCES_FILE` | — | Required in `file` mode; a JSON array containing source rows. |
| `ELECTRIC_CIRCUITS_SOURCES_STORAGE_DIR` | `./data/sources` | Root for `<source_id>/` storage, DBSP state, and transaction spill. |

In `table` mode, `ELECTRIC_CIRCUITS_PG_URL`, `ELECTRIC_CIRCUITS_PG_SLOT`, and
`ELECTRIC_CIRCUITS_PG_TABLES` are rejected. `ELECTRIC_CIRCUITS_BIND`, secrets, Durable Streams, DBSP, transaction,
backfill, and shutdown settings apply to every source.

## Control rows

The engine reads, but never creates or alters, these relations:

```sql
CREATE TABLE circuits_sources (
  source_id       TEXT PRIMARY KEY,
  plugin          TEXT NOT NULL,
  database_secret TEXT NOT NULL,
  slot            TEXT NOT NULL,
  publication     TEXT NOT NULL,
  tables          TEXT[] NOT NULL,
  revision        BIGINT NOT NULL,
  updated_at      TIMESTAMPTZ NOT NULL
);

CREATE TABLE circuits_sources_version (revision BIGINT NOT NULL);
```

`tables` entries are schema-qualified, for example `public.thread_messages`. A file-mode row uses
the same field names and JSON types. `plugin` names the consumer's package that owns the source (recorded in status; decoding always uses `pgoutput`); `publication` must be the
slot's `<slot>_pub` publication. A row contains a secret reference, never a connection URL or
password. `source_id` must be a single safe filesystem path component: not empty, not `.` or `..`,
and without `/`, `\`, control characters, or other path separators. An unsafe id makes only that
source not ready.

## Database-secret resolution

The `database_secret` prefix selects the resolver:

- `env:NAME` reads and validates environment variable `NAME`.
- `file:/absolute/path` reads, trims, and validates the file contents.
- `aws-sm:NAME` reads the current `SecretString` from AWS Secrets Manager using the ambient
  credential chain and environment-selected region.

Every source start and restart resolves its secret again. Unknown prefixes, missing values,
malformed URLs, URL-shaped secret fields, and failed AWS lookups make only that source not ready.
The public `error` field is a fixed classification plus the resolver prefix only, for example
`resolve failed: env variable missing` or `resolve failed: aws-sm lookup error`. It never includes
the secret name, the resolved value, or an underlying error string.

## HTTP routes

The host exposes:

- `GET /health` for liveness.
- `GET /ready`, which becomes `200` after the first successful discovery fetch, even if one or
  more source rows are not ready.
- `GET /sources`, returning `{source_id, revision, ready, error}` summaries.
- `GET /sources/{source_id}/status`, returning the summary plus the source's changes route,
  epoch, position, segments, consumers, and readiness fields.
- Every engine route at `/sources/{source_id}/...`, forwarded to that source after rewriting the
  URI back to the engine root. The engine router is intentionally not nested. Operator/admin
  paths are not forwarded: `/sources/{source_id}/_admin/...` and
  `/sources/{source_id}/epoch/reset` return 404. There is no source-scoped admin surface.
- `POST /admin/refresh`, protected by the private control secret. It accepts no body, fetches rows
  unconditionally, reconciles them, and returns `{ "revision": ... }`.

There is no write API for sources and no source-specific admin route. Host-level `/admin/refresh`
is the only admin route in this mode. The existing `/_admin/*` deployment routes and
`POST /epoch/reset` remain single-source-only and return 404 under a source prefix.

## Discovery and reconciliation

The host fetches the full row set before binding its listener. If the control database is
unreachable, it retries with backoff. A table-mode poll reads only the one version row; unchanged
revision means no other control-table query. A changed revision or explicit refresh fetches all
rows. File mode rereads its file on each poll and refresh.

Reconciliation is idempotent:

- a new row starts one source;
- a missing row stops it;
- a changed row revision stops and restarts it;
- an unchanged healthy row does nothing.

A failed start is retained as a not-ready source with its classified error and is retried when a
later revision or an explicit refresh touches it, including when the row revision is unchanged.
Unchanged healthy rows remain no-ops. Polling does not retry a failed source while the version row
is unchanged, so failure is not a tight loop. One source's failure does not stop other sources.
Stopping leaves that source's storage directory in place, so a restart can restore its shape catalog.

Each source runs on its own operating-system thread with a current-thread Tokio runtime. The
control plane uses one serialized reconcile lock, so concurrent refresh calls cannot interleave
plans. The poll task is owned and joined on shutdown. Once the host shutdown token is set, control
I/O and reconciliation short-circuit and no new source is started.

## Process-global caveats

The existing DB pool, backfill, shutdown, and related `OnceLock` settings remain process-global.
They therefore apply uniformly to every source in a host. Source-specific Postgres URLs, slots,
tables, storage roots, DBSP directories, and transaction-spill directories are per-source. The
engine uses `PostgresSetup::ExternallyManaged`: the consumer's migration/bootstrap step owns
publications, slots, replica identity, and grants; the engine only verifies them.
