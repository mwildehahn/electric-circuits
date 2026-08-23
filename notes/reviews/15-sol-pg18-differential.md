# PG18 / production-spec differential hardening review

Date: 2026-08-23  
Review tree: `0f94a029dc82a29c6f0f36ff82d262f49572c232` (clean)  
Authorities reviewed: `notes/18-production-readiness-spec-reviewed.md`, `notes/21-postgres18-support.md`, `notes/22-server-e2e-tdd-map.md`, `notes/24-postgres18-and-e2e-tdd-addendum.md`  
Compared with: engine Postgres/epoch/replication code, Compose/tutorial/CI/test launchers, and the checked-in `docs/production/readiness-{tasks,gates}.json`.

## Local evidence

- `postgres:18` is available locally as OCI index/platform image
  `postgres@sha256:06cad38a5d9f5d24b4d83d86def30795d5e4b757fedbf5281172b576dedcd941`,
  `arm64/linux`, reporting PostgreSQL 18.6 (Debian 18.6-1.pgdg13+2). `docker manifest inspect`
  resolves the local `linux/arm64/v8` platform manifest to
  `sha256:772ab753f714afefc07b096906b4961e2bb576938c7d007beaa9b62d80680c48`. This is still an
  exploratory image observation, not release evidence (the digest/config/fixture bundle is not
  checked in).
- On that image, `pg_replication_slots` exposes `invalidation_reason`, `restart_lsn`,
  `confirmed_flush_lsn`, `wal_status`, `safe_wal_size`, `failover`, `two_phase`, and `synced`.
  `pg_settings` exposes `idle_replication_slot_timeout`, `output_plugin_libraries`,
  `synchronized_standby_slots`, and `sync_replication_slots`.
- On that image, an omitted generated-column kind is `attgenerated = 'v'`; publication defaults
  report `pubgencols = 'n'`, while `WITH (publish_generated_columns = stored)` reports `pubgencols = 's'`.
  A table with both stored and virtual generated columns therefore has a real publication/wire
  distinction that must be enforced at admission.
- `pnpm exec tsx scripts/readiness-plan.ts validate` passes. The generated manifest is acyclic and
  its PG18 task edges match the corresponding prose edges; this review found no validator/schema
  corruption.
- No engine/conformance run against the local PG18 image was counted as qualification: the current
  harness launches host binaries with trust auth and the checked-in deployment defaults remain
  PG16/cleartext. The review intentionally records metadata and code-path differentials only.

Reproducibility commands (read-only apart from disposable containers):

```sh
docker run --rm postgres:18 postgres --version
docker manifest inspect postgres:18
psql ... -c "select * from pg_replication_slots"
psql ... -c "select name,setting from pg_settings where name in
  ('idle_replication_slot_timeout','output_plugin_libraries',
   'synchronized_standby_slots','sync_replication_slots')"
pnpm exec tsx scripts/readiness-plan.ts validate
```

## Severity-ranked findings and amendments

### P0 — Production TLS contract is absent on all SQL and walsender paths

`apps/engine/src/pg.rs::connect` uses `tokio_postgres::NoTls`; `apps/engine/src/replication.rs::replication_config`
constructs a reduced URL and `TlsConfig::default()`. Compose uses cleartext URLs, including an
explicit `?sslmode=disable` in `docker/compose.electric.yaml`. This directly contradicts the
PG18 profile (verified TLS/SCRAM for setup/admin, pool/backfill/query-back and replication) and
`PG18-003A`/`PG18-003Q` acceptance.

Amend implementation and profile gates to parse one connection/TLS policy and feed it to every
connector, reject downgrade/unknown URL options, and prove `verify-full` identity with stable
`application_name` values and `pg_stat_ssl.ssl=true`. Keep cleartext only in an explicitly local
test profile. Add wrong CA, wrong SAN, rotation, reconnect, and channel-binding cases to the
immutable PG18 candidate gate; a successful setup connection alone is not evidence for walsender.

### P0 — Generated-column admission still allows a snapshot/live schema split

`apps/engine/src/pg.rs::fingerprints` includes every generated column whenever `pubgencols = 's'`
(`a.attgenerated = '' OR $3`), so virtual (`'v'`) columns are admitted as if they were wire fields.
`introspect_opt` separately reads all `information_schema.columns`, and the row mapper can fill an
omitted live field with JSON `null`. This is the exact silent divergence described in notes 21/24:
backfill can return the computed virtual value while pgoutput cannot carry it. The local PG18 image
confirms the metadata (`'v'`, publication `n`/`s` distinction), but no checked-in real-stack replay
proves the fix.

Amend `PG18-001A/B/Q` implementation/evidence to derive one canonical publishable-column set using
`attgenerated` plus the immutable publication. Reject virtual generated columns before any table,
shape, claim, or feed is served; reject stored columns omitted by the effective publication (or
explicitly reject all generated columns). Make introspection, fingerprint, backfill projection,
tuple decode, row mapping, identity/PK validation, drift, reactivation, and restore consume that
set. The unchanged `PG18-E2E-003/004/005` fixtures must fail on a missing-field-to-NULL mutant.

### P0 — Slot invalidation/incarnation/frontier checks are narrower than PG18

`SlotRow` only retains `active`, `active_pid`, `wal_status`, `confirmed_flush_lsn`, and `plugin`.
`observe_slot` ignores `invalidation_reason` and the slot type/database/temporary/failover/
two-phase/restart-LSN/sync fields that PG18 exposes. `engine/epoch.rs::verdict` breaks only on
`wal_status == 'lost'`; unknown/non-null invalidation reasons (including PG18 `idle_timeout`) and
same-name slot recreation with a usable-looking row are not distinguished. Existing tests explicitly
accept `timeline_changed` as `Ok { timeline_changed: true }` and only warn. `ensure_slot` can drop a
foreign-plugin slot and recreate it, while a bound same-name replacement can be adopted without a
durable source-frontier proof.

Amend `PG18-002A/B/C`, `OPS-003B`, and the epoch data model to persist the completely-landed source
frontier and validate slot incarnation/type/database/temporary/failover/two-phase/plugin, status,
invalidation reason, restart/confirmed LSNs, system identifier, and timeline. Any non-null or unknown
invalidation reason must latch a break before old-epoch serving. In the first profile every timeline
change (including a synchronized usable same-name failover slot) is an epoch break. A same-name slot
ahead of the landed frontier must fail closed/reset; behind/equal cases need the documented replay
high-water decision. Slot creation/replacement must occur only through the authorized reset path,
never as an implicit boot repair. Retain focused synthetic reason fixtures, but label only the
primary PG18 `idle_timeout`/`wal_removed` (and supported restart) runs as real PG18 evidence.

### P0 — Runtime publication/role behavior contradicts the bootstrap-owned immutable publication

`setup_postgres` calls `ensure_publication`, which creates `<slot>_pub FOR ALL TABLES` at runtime;
`ensure_replica_identity_full` mutates each application table at runtime. `inspect_publication`
rejects only per-table column lists and reads `pubgencols`; it does not enforce complete explicit
table/operation/partition coverage, publication immutability, RLS rejection, row-security behavior,
or runtime role privilege separation. This is incompatible with the canonical production topology
(bootstrap/admin-owned immutable explicit publication; non-superuser runtime; no tracked-table RLS;
`row_security=off` on walsender) and with `PG18-E2E-008`/`OPS-003A/B`.

Amend deployment and code so an authenticated bootstrap job creates/fingerprints the explicit
publication, slot and replica identity, and runtime preflight only adopts/verifies them. Runtime
roles must not create/alter publications, create/drop slots, own tracked tables, or silently alter
identity. Verify all tracked relations and I/U/D/T operations, row filters/column lists, partition
root/leaf policy, generated-column setting, RLS state and walsender `row_security=off`; fence public
traffic before any sanctioned publication/schema change. The harness marker relation must be in the
immutable test publication, excluded from public templates/results, rather than relying on the
current all-tables `__el_sync` arrangement.

There is also a prose authority conflict to remove: note 21 currently says the engine-created
`CREATE PUBLICATION ... FOR ALL TABLES` path needs no normal-path rewrite, while notes 18/24 and
`OPS-003A` require a bootstrap-owned immutable explicit publication. Once `PG18-000` is integrated,
the explicit publication/profile contract must be the sole production statement; retain `FOR ALL
TABLES` only as an explicitly non-production compatibility lane if desired.

### P0 — PG18 is not the version actually exercised by current deployment/test entry points

`docker/compose.yaml`, `docker/compose.electric.yaml`, and `tutorials/compose.yaml` use `postgres:16`;
`README.md`, `docs/getting-started.md`, `docs/live-queries-guide.md`, `packages/conformance/README.md`,
and `examples/web/README.md` advertise PG16 (or `Postgres 10+`). `.github/workflows/ci.yml` chooses
the highest host-installed PostgreSQL directory, and `vitest.global-setup.ts`, examples, and bench
launchers invoke host `initdb`/`pg_ctl` without a major-version assertion. A green current suite is
therefore not PG18 qualification.

Amend `PG18-000` and all launchers to select an exact PG18 minor/image or binary, assert
`server_version_num` (`1800xx`) before setup, and record OCI index, OS/architecture tuple, and
resolved platform-manifest digest. Pin those bytes in CI/Compose/demo/tutorial/bench fixtures;
production-mode PG17, PG16, and unapproved future majors must fail preflight. Keep a PG16
compatibility lane only if it is named as non-production and cannot satisfy the PG18 release gate.

### P0 — Generated PG18 gate commands cannot prove their own acceptance criteria

The checked-in `docs/production/readiness-gates.json` assigns `pnpm typecheck` to every PG18/PGR
author, merge, and final qualification gate (the generator in `scripts/readiness-plan.ts` maps
`/^PG18-|^PGR-/` to typecheck). Yet `PG18-001Q`, `PG18-002A/B/C`, `PG18-003A/Q`, `PG18-004`, and
`PGR-001` require real PG18 process/image, slot, publication, restore, TLS, promotion, and
minor-maintenance observations. A successful typecheck is not an inherited control for any of
those boundaries and could incorrectly mark the task gates green.

Amend PLAN-001's gate generator and regenerate the matrix so each PG18 gate invokes the exact
task-scoped PG18 fixture/acceptance command with pinned image/binary/config/provider inputs. Keep
typecheck as an adjacent direct gate, not the sole gate; require the evidence fields to include
PG18 server version, OCI index/platform/platform-manifest digest, source/config/profile hashes,
raw fixture artifacts and first-divergence diagnostics. `PG18-003Q:final_release_qualification`
must execute the full hash-pinned `PG18-E2E-001`–`014` public profile through the authenticated
gateway/materializer, not just compile TypeScript.

### P1 — Causal-fence acceptance is specified but current conformance `drainEngine` is not the final oracle

The current helper in `packages/conformance/src/harness.ts` increments a standalone `__el_sync`
counter, polls private `/replication/lsn`, change-log offsets, and `pendingFlips`, and then compares
the materializer. It is useful characterization, but it is not the `E2E-000A` contract: marker and
mutations are not guaranteed to be in one transaction, there is no public `server.drainedThrough`
receipt keyed by source marker, and there is no target `client.appliedTailAfter` cache/materialization
commit receipt. A later SQL query can therefore be mistaken for the source prefix.

Amend the acceptance harness before promotion: write a harness-only marker as the last statement in
the same transaction as mutations; expose adapter-specific server-drained and target-application
receipts keyed by principal/template/generation; hold/release direct and deferred work and cache
commit independently; fold the journal only through `SourceCommitID`. Keep `/replication/lsn`, tail
offsets and pending-work gauges as diagnostics, never as the data oracle. Preserve bad-adapter
mutations that prove each receipt stage can be false-green.

### P1 — Failover/promotion and provider restore claims are ahead of the implementation

Current ADR/code records a timeline change and continues (`docs/adr/0004-slot-epoch-and-reset.md`,
`engine/epoch.rs`), while current backup/restore paths do not persist the PG18 slot-incarnation and
source-frontier evidence required to decide resume versus whole-generation reset. The canonical
profile deliberately excludes seamless failover and requires reset on both missing-slot and
synchronized-slot promotion, plus provider PITR/restore qualification.

Amend `OPS-004`, `PGR-001`, `PG18-002C`, and `PG18-003Q` to gate all promotion/timeline transitions
before reads, retire old handles, and require a fresh generation/materialization. Record provider
artifact, system/timeline identity, slot incarnation, publication/RLS/generated-column definition,
and landed frontier in restore manifests. Treat same-name restored slots as untrusted; exact resume
is allowed only after the explicit frontier decision, otherwise reset before readiness.

### P1 — Conninfo/TLS parity and PG18 failover settings are currently silently discarded

`replication_config` extracts only host/port/user/password/database from the URL and ignores query
options such as `sslmode`, CA/SAN/channel binding, multi-host selection and target-session policy.
The query path and walsender path can consequently interpret one URL differently. PG18 exposes
`output_plugin_libraries`, `sync_replication_slots`, and `synchronized_standby_slots`; current code
does not preflight the plugin allowlist or failover settings.

Amend `PG18-E2E-007/013`, `OPS-003B`, and `PG18-003A` with a canonical parsed conninfo structure,
explicit multi-host/channel-binding policy, and parity tests for escaped credentials and unknown or
weaker settings. Fail closed when `pgoutput` is excluded or the selected TLS identity policy cannot
be applied. Keep failover-slot settings out of the first profile, but reject/diagnose accidental
`failover=true`/standby synchronization assumptions instead of inferring seamless support.

### P2 — Support documentation still presents development behavior as broad production support

`docs/deployment-postgres.md` says “Postgres 10+” and claims the engine creates a superuser
`FOR ALL TABLES` publication; `apps/engine/README.md` and deployment docs describe stored generated
columns but do not state the virtual/unpublished-storage rejection policy. Compose also defaults DS
to memory durability and exposes direct/visualizer/control routes. The canonical notes correctly
state that these are current-development facts, but the user-facing docs need an explicit
“development/compatibility only; not the PG18 production profile” banner until `PG18-000` closes.

Amend docs in the same PG18 support packet: state the exact first profile, TLS/SCRAM requirement,
bootstrap publication ownership, generated-column policy, reset-on-promotion behavior, and DS
durability/gateway boundary. Do not remove the useful PG16 compatibility lane without marking its
scope and evidence.

## No-finding / aligned areas

- The four canonical notes consistently make PG18 the only first-production profile, keep PG18
  failover slots/seamless promotion future-only, and call the exploratory PG18.6 generated-column
  result unverified rather than inherited qualification. This is the correct claim posture.
- The canonical fixture matrix includes positive stored-generated coverage, virtual/unpublished
  negatives, idle and WAL-loss invalidation, plugin allowlist, connector-specific TLS, missing and
  synchronized-slot promotion, same-name slot recreation/frontier cases, publication/RLS/identity
  mutation, and minor maintenance/import policy. Those scenarios are materially stronger than the
  current implementation and should remain unchanged.
- `pgoutput` protocol v1 remains an appropriate first-release choice; PG18 locally reports the
  bundled `pgoutput` plugin and the notes correctly defer protocol v2–v4 expansion. No amendment to
  the v1 spill design is required for PG18 compatibility.
- The checked-in readiness task graph and gate matrix validate structurally and contain the intended
  PG18 ordering (`PG18-000` → schema/slot qualification → candidate packaging → reset/restore/
  maintenance → public profile). Structural validation does not make the generic `pnpm typecheck`
  commands adequate qualification (see the P0 gate finding). `scenario_ids: []` is consistent with
  the declared pre-`E2E-000S` registry state, not evidence that scenarios are already qualified.
- Existing epoch tests correctly guard missing-slot, lost-WAL, foreign-plugin, cluster-identity,
  busy-slot, reset-on and reset-off behavior. They must be extended for the PG18 invalidation reason,
  frontier/incarnation and promotion-reset contracts; their current green state is not authority for
  those new behaviors.

## Recommended amendment order

1. Land the PG18 version/image/fixture identity and canonical publication/TLS/slot preflight packet;
   keep current broad lanes explicitly non-production.
2. Add genuine-red generated-column, invalidation/incarnation, and TLS connector E2E artifacts on the
   exact PG18 image; independently review them before implementation packets.
3. Implement canonical publishable schema, slot/frontier/timeline handling, bootstrap publication and
   shared TLS/conninfo; then run the unchanged focused and PG18 E2E matrices.
4. Replace characterization `drainEngine` evidence with E2E-000A causal receipts and qualify provider
   restore/promotion reset before any production support claim.
