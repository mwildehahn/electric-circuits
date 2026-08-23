# PostgreSQL 18 support audit and TDD contract

## Recommendation

**Use PostgreSQL 18 as the only first-production database profile, but do not call
the current tree production-supported on it yet.** A cleartext, single-primary PG18
with `wal_level=logical`, built-in `pgoutput`, and no generated columns should work:
the engine consumes the still-valid pgoutput protocol v1 and already reads PG18
publication metadata. That is not production certification.

This first-pass audit reproduced three immediate P0 implementation items:

1. Treat every PG18 logical-slot invalidation, including `idle_timeout`, as an epoch
   break.
2. Make generated-column handling exactly match the PG18 pgoutput wire schema, or
   reject unsupported generated columns before serving them.
3. Add verified TLS to every Postgres connection path, including the replication
   walsender.

The first release remains **one writable primary and one active engine per logical
slot**. PG18 failover slots are a future option, not an excuse to advertise seamless
failover. Initial promotion semantics are epoch break → close/reset → client
rehydration, backed by an E2E test. The later differential review expanded the complete
blocker set to slot-incarnation/frontier proof, immutable publication/RLS admission,
connector-by-connector TLS, provider restore and minor maintenance; see
[`24-postgres18-and-e2e-tdd-addendum.md`](24-postgres18-and-e2e-tdd-addendum.md) and
[`25-pg18-e2e-differential-review-disposition.md`](25-pg18-e2e-differential-review-disposition.md).

PostgreSQL facts below link only to PostgreSQL's official version-18 documentation
and release notes. Local paths/symbols are evidence about this repository.

## Relevant PG18 facts and repository status

| PG18 fact | Effect | Local evidence | Required disposition |
|---|---|---|---|
| `pgoutput` accepts protocol versions 1–4; v1 remains valid. v2–v4 add streamed/parallel/two-phase capabilities. [Protocol](https://www.postgresql.org/docs/18/protocol-logical-replication.html) | PG18 does not require a protocol upgrade. | `apps/engine/src/pgoutput.rs` declares protocol v1; `replication.rs` uses `pgwire-replication`; ADR `0003-ingest-pgoutput-v1-with-spill.md` deliberately spills v1 transactions. | Docs + real-PG18 regression only. Do **not** widen the ingest design for this release. |
| PG18 can publish **stored** generated columns with `publish_generated_columns = stored`; they are not published by default. Logical replication currently supports stored, not virtual, generated columns. [CREATE PUBLICATION](https://www.postgresql.org/docs/18/sql-createpublication.html), [generated columns](https://www.postgresql.org/docs/18/logical-replication-gencols.html), [release notes](https://www.postgresql.org/docs/18/release-18.html) | Snapshot/backfill and live pgoutput must agree on their column set. | `pg.rs::inspect_publication` reads PG18 `pubgencols`; `pg.rs::fingerprints` filters some generated fields. But `pg.rs::introspect_opt` reads all `information_schema.columns`; `TableSchema::row_from_json` maps a missing live field to `NULL`; fingerprint logic admits both stored and virtual columns when `pubgencols='s'`. A focused PG18.6 smoke using `postgres@sha256:06cad38a5d9f5d24b4d83d86def30795d5e4b757fedbf5281172b576dedcd941` reproduced it: backfill returned virtual `title_upper='TWO'`, live `/v1` emitted `title_upper:null`. | **P0 code + E2E. Current virtual-generated-column behavior is a launch blocker.** |
| PG18 adds `idle_replication_slot_timeout`, invalidating inactive slots at checkpoint. `pg_replication_slots.invalidation_reason` reports `idle_timeout`, `wal_removed`, `rows_removed`, or `wal_level_insufficient`. [Release notes](https://www.postgresql.org/docs/18/release-18.html), [settings](https://www.postgresql.org/docs/18/runtime-config-replication.html), [slot view](https://www.postgresql.org/docs/18/view-pg-replication-slots.html) | An invalidated slot cannot resume an existing shape epoch honestly. | `pg.rs::SlotRow` reads `wal_status`, not `invalidation_reason`; `engine/epoch.rs::verdict` breaks only on `wal_status == 'lost'`. | **P0 code + E2E.** |
| PG18 logical failover slots synchronize only with `failover=true`, standby `sync_replication_slots=true`, and—if delivery must be fenced—primary `synchronized_standby_slots`. [Slot synchronization](https://www.postgresql.org/docs/18/logicaldecoding-explanation.html), [failover](https://www.postgresql.org/docs/18/logical-replication-failover.html), [settings](https://www.postgresql.org/docs/18/runtime-config-replication.html) | Useful future HA profile; unsafe to infer from server version alone. | `pg.rs::create_slot` calls the two-argument slot function (failover false). `engine/epoch.rs` logs timeline change but does not act; ADR 0004 explicitly assumes one primary/no promotion. | No first-release topology change; separate ADR/feature gate/E2E suite. |
| Current PG18 minor releases restrict output plugins through `output_plugin_libraries`, whose default includes `pgoutput`. [Settings](https://www.postgresql.org/docs/18/runtime-config-replication.html), [18.6 release notes](https://www.postgresql.org/docs/18/release-18-6.html) | The bundled plugin remains valid; hardened servers can exclude it. | `pg.rs::PGOUTPUT = "pgoutput"` and `create_slot` names it. | Document and negative-boot-test the override; no ordinary-path change. |
| PG18 deprecates MD5 password auth; SCRAM is the forward profile. [PG18 press kit](https://www.postgresql.org/about/press/presskit18/) | Production should use SCRAM. | `apps/engine/Cargo.toml` enables pgwire replication `scram`, not `md5`. | Document and exercise in TLS lane. |

## Exact production profile to claim after P0

> PostgreSQL 18.x primary, `wal_level=logical`, a dedicated SCRAM replication role,
> verified TLS on every connector, one engine leader per logical slot, the bundled `pgoutput`
> plugin, a bootstrap-owned immutable explicit-table publication, and no tracked-table RLS.
> Stored generated columns are supported only when the publication publishes them;
> virtual generated columns are rejected for tracked tables. Promotion is reset and
> rehydrate, not seamless failover.

Remove current contradictory claims when this lands: `docs/deployment-postgres.md`
says “Postgres 10+”, whereas `README.md`, `docs/live-queries-guide.md`,
`packages/conformance/README.md`, and `examples/web/README.md` say PG16. The code
already has PG18-specific `pubgencols` behavior, so a narrow, testable PG18 profile
is more honest than a broad untested range.

## P0 implementation tasks

### P0-A: all slot invalidations are epoch breaks

Extend `pg.rs::SlotRow` and `observe_slot` to capture
`pg_replication_slots.invalidation_reason`. In `engine/epoch.rs::verdict`, a non-null
reason must produce an epoch break. Preserve the server reason in the control plane
and metric label/detail: a stable generic `slot_invalidated` plus the PostgreSQL
reason is sufficient, or use stable variants for all four documented reasons.

Keep current policy behavior:

- reset disabled: ingestion stops and public shape APIs give typed/named 503 until
  `POST /epoch/reset`;
- default auto-reset: durably retire old shape streams, replace the slot, bind a new
  epoch, and require a client subscription to create materialized state again.

Do not treat a `START_REPLICATION` error as a retryable transport error while old
shape state remains available. This is exactly the unknown-gap case the epoch layer
already exists to prevent.

### P0-B: generated-column admission and wire schema

The problem is observable with a PG18 table such as:

```sql
CREATE TABLE issues (
  id bigint primary key,
  title text not null,
  title_len int GENERATED ALWAYS AS (length(title)) STORED,
  title_upper text GENERATED ALWAYS AS (upper(title)) VIRTUAL
);
```

With the default publication, pgoutput carries neither generated value. With
`publish_generated_columns = stored`, it carries `title_len` but still not
`title_upper`. PG18 makes generated columns virtual by default, and PostgreSQL
documents that only stored generated columns can be logically replicated. The local
PG18.6 smoke confirms the production symptom: snapshot/backfill returns computed
`title_upper = 'TWO'`, then a live update emits `title_upper: null`.

Implement the narrow, safe policy:

1. Reject a tracked table with a **virtual** generated column at boot, before any
   shape/stream is created. PG18 does not put it on the logical wire. This is the
   recommended first-release behavior.
2. Support a stored generated column only when the effective publication has
   `publish_generated_columns = stored`. When Circuits owns a new publication,
   create it with that option; when adopting one, require it and name the offending
   table/publication. The conservative alternative—reject all generated columns—is
   acceptable only if explicit and fail-closed.
3. Build `TableDef`, `TableSchema`, fingerprint, backfill projection, and tuple map
   from exactly the publishable set. In particular distinguish `attgenerated='s'`
   from `'v'`; the current `attgenerated = '' OR $3` is insufficient. Avoid the
   tempting partial fix of altering the fingerprint alone: it leaves
   `TableSchema::row_from_json` filling omitted fields with NULL.
4. Fail before serving if a primary-key/replica-identity field is not publishable.
   PostgreSQL requires generated replica-identity fields to be explicitly published
   for UPDATE/DELETE. [CREATE PUBLICATION](https://www.postgresql.org/docs/18/sql-createpublication.html)

### P0-C: TLS must cover SQL and replication equally

Require `sslmode=verify-full` (or an equivalent explicit production setting), trusted
CA, and hostname validation. Use a single parsed connection configuration to feed
both `tokio-postgres` and `pgwire-replication`, including SNI override/CA path and
client certificate/key if mTLS is supported. Cleartext `sslmode=disable` can remain
only in an explicit local/test profile.

Local evidence of the gap:

- `pg.rs::connect` passes `tokio_postgres::NoTls`: boot/introspection, pooled
  query-backs, snapshot/backfill, and subset queries are all cleartext.
- `replication.rs::replication_config` reduces the URL to host/user/password/database
  then sets `TlsConfig::default()`. The crate is declared with
  `default-features = false, features = ["scram"]`, so no Rustls TLS feature is
  enabled and its default mode is disabled.
- The only URL test verifies `?sslmode=disable` tolerance
  (`apps/engine/src/config.rs`); `docs/fleet-conformance.md` and
  `docker/compose.electric.yaml` use it for local compatibility.

Fail rather than silently downgrading when one of the query or walsender paths cannot
verify TLS. A successful boot SQL connection is not enough if the live feed is plain
TCP.

## P1: preflight, test infrastructure, documentation

No PG18 publication syntax requires a normal-path rewrite: the engine-created
`CREATE PUBLICATION ... FOR ALL TABLES` path remains valid. Add production preflight
or enforce documented operator proof for:

- logical WAL, `max_replication_slots`, and `max_wal_senders` sized for both logical
  and physical replication; [official configuration guidance](https://www.postgresql.org/docs/18/logical-replication-config.html)
- coverage of every tracked table and all I/U/D/T operations, with no row filter or
  partial column list that can make a shape stale;
- `output_plugin_libraries` allowing `pgoutput` when overridden;
- primary key, replica identity, and generated-column admission requirements.

`pg.rs::inspect_publication` currently rejects detected per-table column lists only;
it does not establish full hand-managed publication coverage. That is production
hardening. The reviewed first profile uses a bootstrap-owned explicit-table publication, not
`FOR ALL TABLES`, so every selected relation/operation/column/filter/partition setting is part of
admission and the immutable fingerprint.

Pin all real environments to 18:

- `docker/compose.yaml`, `docker/compose.electric.yaml`, and
  `tutorials/compose.yaml` currently specify `postgres:16`.
- `.github/workflows/ci.yml` uses the newest preinstalled `/usr/lib/postgresql/*/bin`
  rather than installing/asserting 18.
- `vitest.global-setup.ts`, `examples/linearlite/start.ts`,
  `examples/web/start.ts`, and benchmark launchers run host `initdb`/`pg_ctl` with no
  major-version assertion.

The mandatory integration lane should use a pinned PG18 image/binary and assert
`SHOW server_version_num` begins with `1800`. Update all PG16 and “10+” docs to the
profile above, including TLS/SCRAM, generated columns, idle invalidation, and the
promotion-reset contract.

## TDD: public end-to-end tests to write first

Tests must run the real boundary: **Postgres 18 → engine process → durable streams →
public client/control API**, and compare materialized state to `SELECT` from the same
Postgres database. Do not pin decoder structs, catalog records, or circuit shape;
those are refactorable implementation details.

### Mandatory PG18 lane

Provision a real, disposable PG18 cluster with logical WAL and assert version before
engine setup. Add these to `packages/conformance`'s existing process harness.

| Case | Black-box acceptance criterion |
|---|---|
| **Baseline live shape** | Create table/shape; insert, update, delete matching and nonmatching rows, including a multi-row transaction. After each transaction boundary, client materialization equals `SELECT`. Restart engine, repeat a write: no duplicate/missing row and no new shape generation. |
| **Snapshot/live fence** | Stall creation/backfill at its fence while another session commits a row. Release it: final client state is exactly the oracle, neither missing nor duplicated. This preserves `SnapshotGate` semantics on PG18. |
| **Stored generated column** | Create publication with `publish_generated_columns=stored`; shape both projects and filters stored generation. Update source so it enters, changes within, and leaves the shape. Every public stream state equals oracle and never replaces a PostgreSQL value with NULL. |
| **Generated negatives** | (a) stored generated field under publication `none`; (b) virtual generated field, including a requested projection/predicate. Engine fails before live serving with a stable actionable error. A successful but stale/null stream fails the test. If implementation elects to support a case, it must instead pass the oracle test. |
| **Idle invalidation** | Set short `idle_replication_slot_timeout`; create a shape; stop engine; wait/force checkpoint; verify `invalidation_reason='idle_timeout'`. Restart with reset off: named epoch failure/no stale emission. Restart with reset on: old stream closes, re-subscription creates new state that equals oracle. |
| **Plugin allowlist** | Exclude `pgoutput` in a disposable PG18 server's `output_plugin_libraries`. Setup fails closed and never becomes ready/creates a supposedly live shape. |

### Mandatory production transport lane

Run PG18 with TLS, a test CA, `hostssl` `pg_hba.conf`, a DNS server name, and SCRAM
credentials.

| Case | Black-box acceptance criterion |
|---|---|
| **Verified TLS end to end** | Give setup/admin, pool/backfill/query-back and walsender distinct `application_name`s. Exercise each after readiness and require a corresponding `pg_stat_ssl.ssl=true` backend before client/oracle convergence. |
| **No downgrade** | After the other connector paths are healthy, independently target each named path with wrong CA, wrong hostname/SAN, rotation failure, and cleartext against `hostssl`; readiness/freshness fences and no live state advances. |
| **TLS reconnect** | Break/restart the database replication connection while a shape is live, restore it, and assert verified reconnection plus oracle convergence without unannounced epoch reset. |

### Required first-release promotion test (not seamless)

Run two PG18 primary/physical-standby variants and route the engine to the promoted node: one without
a synchronized slot, and one with a synchronized, usable same-name `pgoutput` failover slot. Create a
shape, commit marker rows, promote, redirect, then observe the public stream. The second variant is
essential: otherwise a missing slot can trigger reset without proving the timeline policy.

Acceptance:

1. Old epoch is never silently continued across promotion.
2. Engine reports slot/epoch break, closes old streams, then either stays fail-closed
   pending operator reset or completes documented auto-reset.
3. A fresh subscription converges to the new primary's `SELECT` without mixing epochs.

This proves the launch promise “promotion requires rehydrate”; a unit test of a
timeline integer does not.

## PG18 failover slots: future profile only

PG18 does **not** alter the first-release topology. Offer a future
`SEAMLESS_PG_FAILOVER` profile only after a separate ADR and all of these:

1. Create/adopt logical slots with `failover=true` (documented fifth parameter to
   `pg_create_logical_replication_slot` or replication-protocol `FAILOVER`).
   [Functions](https://www.postgresql.org/docs/18/functions-admin.html), [protocol](https://www.postgresql.org/docs/18/protocol-replication.html)
2. Configure named physical standby slots in primary `synchronized_standby_slots`,
   and standby `sync_replication_slots=true` with correct `primary_conninfo`.
3. Add readiness/observability for a target's synced, non-temporary,
   non-invalidated logical slot. A physical replica connection or changed timeline is
   not sufficient evidence.
4. Make active-engine leadership/routing explicit and prove a monotonic durable
   checkpoint/LSN boundary before serving after promotion.
5. Define unplanned promotion before synchronization as epoch reset, never as a
   best-effort seamless continuation.

The future E2E gate is stricter: after PostgreSQL reports its documented
`failover_ready` condition, promote standby and redirect engine without changing a
client handle. Marker rows before/after promotion must appear once, ordered, with no
snapshot/reseed and final state equal to new-primary `SELECT`. Repeat with a not-ready
standby and require the explicit epoch-reset fallback.

## Delivery order

1. Add failing real-PG18 baseline, generated-column, and idle-slot tests.
2. Fix P0-A and P0-B until they pass.
3. Add failing TLS lane; implement P0-C until it passes.
4. Pin compose/CI/demo/test binaries to PG18 and update the support contract.
5. Add promotion-reset E2E to the release gate.
6. Decide seamless PG18 failover independently; do not block the single-primary PG18
   launch on it.

Existing Rust/unit, TypeScript/conformance, Electric oracle, and browser demo checks
remain necessary. This is an additional compatibility layer, not a replacement.
