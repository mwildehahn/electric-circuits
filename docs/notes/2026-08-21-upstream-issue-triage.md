# Upstream issue triage (electric-sql/electric-circuits #3–#17) against the fork

Verified against `develop` at `acb3347` (upstream `main` `b784aaf` + fork commits). All fourteen
open upstream issues were filed on 2026-07-06 in one production-readiness sweep; two days later
upstream PR #22 (sequencer + streaming pgoutput + durable catalog) resolved or reshaped about half of
them without closing anything. This note records what is actually open, what the fork will fix
before feature work, and in what order. Decisions are in `docs/adr/`.

## Issue by issue

| # | Upstream title | State in the fork | Fork plan |
|---|---|---|---|
| 17 | Migrate ingest to streaming pgoutput, proto v2+ | Streaming pgoutput is in (`replication.rs`, `pgoutput.rs`) but via `pgwire-replication` 0.3.2, which hardcodes `proto_version '1'` (`worker.rs:187`); 0.4.0 is still v1-only; no upstream work on v2 anywhere | v1 stays; deferred — ADR-0003 |
| 3 | Shape id reuse after restart | `next_shape_id` restored from the catalog fold (`engine/catalog.rs:128`) | resolved by #22 |
| 4 | Restart freezes extended-API shapes | catalog restore re-registers shapes; subquery shapes dropped loudly | resolved by #22 |
| 8 | Persistent catalog + lazy recovery | catalog exists (`meta/catalog`); no orphan `shape/*` GC; no incomplete-backfill discard event | orphan GC folded into the retirement work |
| 9 | Retention lifecycle | closed (#21, rebuilt in #22) | — |
| 13 | PG connection management | `pg::Pool` + semaphore done; backfills still materialise `Vec<Row>` (`pg.rs:412,586`); reconnect flat 500 ms (`replication.rs:61`); no error taxonomy | backoff (ADR-0004), taxonomy, streamed backfills, slow-backfill knob — slice 6 |
| 15 | Auth / CORS / debug isolation | `ELECTRIC_SECRET` on `/v1/shape` + OPTIONS preflight (#19); no control-plane auth, no utility port | deferred (engine is cluster-internal behind the consumer's control plane) |
| 16 | Graceful shutdown / readiness / metrics | `/v1/health` state machine exists; no SIGTERM handling in `main.rs`; StatsD slot-WAL gauges exist | SIGTERM + readiness — slice 6 |
| 5 | Failover / slot loss | `ensure_slot` only at boot; reconnect never re-checks; no system-id/timeline → reboot after slot loss silently creates a fresh slot at head | ADR-0004 — slice 3 |
| 7 | DDL / TRUNCATE leave shapes stale | `R` messages used only to map relid→columns; `tuple_to_map` skips columns not in the boot schema (`replication.rs:357`); TRUNCATE / replica-identity regression only log | ADR-0005 — slice 2 |
| 6 | Unbounded memory on large transactions | `TxnBuf.envs` holds the whole transaction until `Commit`; `bytes` is StatsD-only; no cap, no spill | ADR-0003 — slice 5 |
| 12 | DS disk growth | architecture changed: one global `changes` log (`lib.rs:54`) grows forever; engine never trims; DS 0.1.x has whole-stream TTL only | ADR-0006 — slice 4 |
| 10 | `/v1/shape` HTTP caching | still `no-store` | out of scope — ADR-0001 |
| 11 | Extended-API caching via the DS CDN protocol | Rust DS server adopted (#25); client cursor/304 not done | deferred |
| 14 | TLS to Postgres | `NoTls` everywhere | deferred (in-cluster) |

Also in scope, not an upstream issue: schema-qualified tables (identity is a bare `String` end to end;
the pgoutput namespace is decoded at `pgoutput.rs:64` and discarded at `replication.rs:233`;
`electric.rs:761` strips the prefix) — ADR-0002, slice 2.

## On protocol v2

The `Relation` message is identical from v1 through v4, so schema-qualified identity is independent
of the protocol version. v2 does not remove receiver-side buffering for this engine (one commit must
reach the change log atomically; a streamed transaction cannot be appended before its commit frame)
and would require forking the replication client. See ADR-0003.

## Order of work

1. Retirement primitive: close-then-delete in `ds.rs` and every evict/purge site (ADR-0007).
2. Table identity + schema drift: `TableRef`, always-qualified wire/catalog, decoder keeps namespace,
   type OIDs and replica identity, reconciler, per-table retirement, TRUNCATE, identity re-assert
   (ADR-0002, ADR-0005). Conformance: same table name in two schemas stays distinct; `ADD COLUMN`,
   `TRUNCATE`, identity regression each retire exactly that table's shapes.
3. Slot epoch: `SlotBound`, re-check on reconnect, auto-reset default + refuse flag, jittered backoff
   (ADR-0004).
4. Change-log rotation: segments, `(segment, offset)`, close-then-continue pointer, evict-before-delete,
   retention knobs (ADR-0006).
5. Large transactions: per-transaction cap → spill → chunked appends, ack after the last (ADR-0003).
6. Ops: SIGTERM, readiness (`/ready` split from the liveness `/health`), boot-time
   fatal-vs-retryable taxonomy, streamed backfills, off-by-default slow-backfill timeout, and the
   replication-slot gauges made engine-owned so `/metrics` carries them without StatsD.
7. Feature work.

## Deferred, with reasons

- Protocol v2 (#17): latency-only benefit here; large client cost. ADR-0003.
- `/v1/shape` caching (#10), compat-adapter defects: not on the native path. ADR-0001.
- Control-plane auth, utility port (#15), TLS (#14): the engine is cluster-internal in the first
  deployment. Revisit when it is exposed.
- DS prefix trimming (#12 as written): replaceable by segment rotation now; revisit if durable-streams
  grows the primitive. ADR-0006.
- Timeline/failover detection beyond the system identifier (#5): single primary for now. ADR-0004.

## Findings worth relaying upstream

- #3, #4, #8 (mostly), #17 (v1 half): resolved by #22, still open.
- #17: v2 does not remove receiver-side buffering for a commit-atomic change log; the client crate
  has no v2 support.
- #12: the `table/*` framing predates the single `changes` log.
- `electric.rs:761` strips a non-`public` schema qualifier and answers with the `public` table's rows.

## Follow-ups surfaced while implementing slices 1–6

Recorded here so they are not lost; none blocks the order of work above.

- **TRUNCATE replay window (ADR-0005).** A crash between a `TRUNCATE`'s append and its slot
  acknowledgement (≤ 1 s) re-retires that table's shapes at the next boot, including shapes created
  after boot that already reflect the truncation. Exact handling needs the shape's `SnapshotGate` on
  its catalog record (or a sequencer round-trip during retirement). Cost today: one spurious resync.
- **Circuit-tier drift exit has no end-to-end test.** `circuit_needs_rebuild` is unit-tested; no lane
  boots with `ELECTRIC_CIRCUITS_DBSP_COUNTS`, triggers drift, and asserts exit 75 → restart → recover.
- **Publication column-list refusal is untested** (`pg::inspect_publication`): the harness always
  derives `<slot>_pub` itself. `prattrs` is PG15+; it is only queried for a hand-made publication.
- **Stale `/v1/shape` handle after the delete lands** still answers 500 (the closed-stream case now
  answers `409 must-refetch`); needs a typed "stream gone" read error in `ds.rs`.
- **`packages/client` does not yet re-subscribe on `stream-closed` / 404 / 410** (ADR-0007 states
  the contract as *must*); the engine side is done.
- **A table dropped and re-created** is untracked until restart (documented in
  `docs/deployment-postgres.md`).
- **The replication ingestor's connection has no connect timeout.** `pg::connect` (boot, pool) now
  applies a 10 s `connect_timeout`; the walsender connection is built by `pgwire-replication` from
  its own `ReplicationConfig`. Not boot-blocking (its task joins the shutdown token), but a
  firewalled host costs a wedged walsender connect rather than a fast failure.
- **`ELECTRIC_CIRCUITS_DS_URL` is not parse-validated at config time** the way
  `ELECTRIC_CIRCUITS_PG_URL` now is; an unusable one surfaces as a fatal `is_builder` error on first
  use (still exit 78, just later).
- **`wal_level` ≠ `logical` has no conformance test** (the harness cluster is always logical; changing
  it needs a Postgres restart). Covered by the classifier unit tests and the explicit `SHOW wal_level`
  call site only.
- **A `tokio_postgres` error with no SQLSTATE and no io source** (e.g. a server demanding a password
  the URL lacks) is retried forever rather than exiting 78 — `Kind` is private and matching the
  `Display` string was rejected. Visible: `/ready` = `waiting` and every attempt is logged.
- **`ELECTRIC_PROMETHEUS_PORT` is still accepted-but-unimplemented**; `/metrics/prometheus` is now a
  complete scrape target, which makes the missing dedicated listener slightly more visible.
