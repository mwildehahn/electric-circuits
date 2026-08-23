# Operations and durability differential review

**Review date:** 2026-08-22
**Scope:** production deployability, durable-streams (DS) persistence/backup/restore, Postgres logical
replication, leadership/failover, upgrades/rollback, configuration, observability/runbooks, containers,
Kubernetes, and release artifacts. This is a differential review of
`16-production-readiness-and-swift-migration-spec.md` against the current repository and the earlier
operations/readiness notes. It does not assert that the existing demo stack is a production stack.

## Verdict

The draft names most of the right work areas, but its operations plan is not executable yet. There are
direct dependency cycles, the proposed Postgres lock cannot fence an old process from DS, and the
backup task lacks the restore identity/frontier needed to detect a DS snapshot restored behind an
already-advanced replication slot. The current purge endpoint also returns a terminal success before
the promised stream teardown finishes; this is already a deterministic test failure, not a theoretical
edge case.

The first production release should support a deliberately narrow topology: one engine, one
file-backed DS process on one RWO volume, no automatic Postgres promotion, and quiesced/offline DS
backups. Automated active/passive failover, online backups, and promotion without a full epoch reset
should remain unsupported until downstream-enforced fencing and continuity proofs exist.

## Findings

### P0-01 — The task graph contains direct cycles and cannot be scheduled as written

**Evidence.** `ENG-010` depends on `OPS-002`, while `OPS-002` depends on `ENG-010`
(`notes/16...:603-619`, `698-715`). `ENG-013` depends on `OPS-004`, while `OPS-004` depends on
`ENG-013` (`657-675`, `736-753`). Outside the OPS section, `PROTO-003` depends on `ENG-002`, while
`ENG-002` depends on `PROTO-003` (`235-253`, `454-472`). Wave 1 nevertheless schedules `OPS-001`,
`OPS-002`, and `OPS-003` together (`1516`), even though the claimed protected deployment also needs
later TLS/configuration work: `SEC-006` depends on `OPS-001`, and `ENG-012` depends on both.

**Exact task changes.**

1. Make `ENG-010` depend only on `GOV-002`; define its output as the durable accounting, inventory,
   free-space reserve, and admission interfaces. Keep `OPS-002` dependent on `ENG-010`.
2. Make `ENG-013` depend on `GOV-002` plus a new `OPS-001A` deployment scaffold. Keep `OPS-004`
   dependent on `ENG-013`.
3. Make `PROTO-003` own the wire contract and depend on `PROTO-001`; make `ENG-002` depend on it.
4. Split `OPS-001` into `OPS-001A` (private service/image/chart scaffolding) and `OPS-001B` (the
   protected, qualified package). Make `SEC-006` and `ENG-012` depend on `OPS-001A`; make
   `OPS-001B` depend on the gateway, TLS, strict config, DS packaging, and image-hardening tasks.
5. Replace the hand-authored wave table with a machine-checked DAG generated from task metadata.

**Operation-count acceptance.** Parse every task edge on every release build; topologically sort the
whole graph and fail on any cycle or unknown task. Run 100 randomized ready-task scheduling simulations
and require every blocker to reach a terminal state without dependency override.

### P0-02 — A restored DS snapshot can silently fall behind the live Postgres slot

**Evidence.** The epoch binding stores only Postgres system identifier, timeline, slot name, and time
(`apps/engine/src/engine/epoch.rs:82-96`). Slot observation reads `confirmed_flush_lsn`
(`apps/engine/src/pg.rs:665-706`), but `epoch::verdict` does not compare it with any durable DS backup
generation or ingested frontier (`apps/engine/src/engine/epoch.rs:181-207`). Therefore this sequence is
accepted as the same epoch: engine appends changes and acknowledges WAL; DS is restored to an older
snapshot; the unchanged Postgres slot remains ahead; boot sees the same cluster and healthy slot and
resumes after a gap that no longer exists in DS. `OPS-002` says to “record the relationship,” but does
not define the record, atomicity boundary, or decision table.

**Exact task changes.** Replace the backup part of `OPS-002` with a concrete first-release protocol:

- Support **quiesced/offline backup only** initially: stop new creates/admin mutations at the gateway,
  drain and stop the engine, quiesce/stop DS after its WAL checkpoint, then snapshot the complete DS
  directory. Do not describe volume-copy-while-running as supported until DS supplies and tests a
  snapshot barrier.
- Write a checksummed restore manifest containing format version, DS store UUID, stack ID, engine and DS
  image digests, catalog tail/hash, current and closed change-log inventory, durable sequencer position
  and highwater, last complete source transaction LSN, slot binding, and every local/object-tier object.
- On restore, compare the manifest frontier with `confirmed_flush_lsn`, slot provenance, cluster ID,
  timeline, and catalog binding **before opening public/readiness listeners or mutating storage**. If PG
  has acknowledged beyond the backed-up DS frontier, require a matching PG restore or a typed epoch
  reset/full client refetch. Never call it a resumable restore.
- Define separately: whole-stack restore, DS-only restore, PG-only restore/PITR, missing slot, stale or
  advanced slot, foreign cluster, and partial object-tier restore. Unsupported combinations must fail
  with stable reason codes.

**Operation-count acceptance.** For each supported backup type, run 100 cut points at each of catalog
append, change append, transaction-end append, checkpoint publication, slot feedback, DS WAL
checkpoint/recycle, snapshot, and restore. For each restored instance, process 10,000,000 source
mutations and compare catalog fold, stream inventories, offsets/highwater, leases, and logical client
state with SQL. Test same/ahead/behind/missing/lost/foreign slots and 100 corruptions in every artifact
class; the only outcomes are exact resume or a named reset/refusal.

### P0-03 — A Postgres advisory lock cannot fence stale DS mutations

**Evidence.** `ENG-013` proposes a Postgres advisory lock or equivalent (`notes/16...:664-668`). Today a
busy slot is explicitly treated as intact (`epoch.rs:21-27`, `181-207`), and boot restores the catalog
for `Verdict::Busy` just like `Ok` (`apps/engine/src/engine/mod.rs:1245-1247`). The slot only excludes a
second replication walsender; it does not exclude the second control plane from catalog, shape, close,
or delete operations. More fundamentally, after PG partition, session termination, or pause/resume, an
old process can retain DS credentials and send delayed mutations even after a new process owns a PG
lock. A lock checked only by engine code is not a fence at the downstream side effect.

**Exact task changes.** Change `ENG-013` from “database-level lock” to “downstream-enforced monotonic
leadership fence.” The authority must issue a monotonically increasing term bound to stack/store/epoch.
DS must atomically reject every catalog/change/shape append, create, close, and delete carrying a stale
term. Slot feedback must also be conditional on the current term. Acquire and validate the fence before
catalog application, sequencer/retention/retirement tasks, mutating HTTP routes, or readiness. Treat the
Postgres advisory lock as leader election only, not as the fencing proof. If DS cannot enforce a token,
remove automated failover from the supported topology.

**Operation-count acceptance.** Run 100 repetitions each of simultaneous boot, SIGSTOP/SIGCONT beyond
lease expiry, one-way PG partition, one-way DS partition, delayed/reordered requests, leader kill at
every acquisition/promotion write, and stale request replay. Across at least 10,000,000 mutations, at
most one process is ready and DS rejects every stale-term mutation; the final logical state equals SQL
or carries an explicit full-refetch boundary.

### P0-04 — `DELETE ?purge=true` acknowledges completion before the stream is retired

**Evidence.** The HTTP contract says purge means “full teardown, stream deleted” and “force-drops ...
immediately” (`apps/engine/src/http.rs:541-561`), then returns `200 {"ok":true}` after
`purge_shape_durable` (`575-581`). The implementation says it does **not** wait for retirement
(`apps/engine/src/engine/lifecycle.rs:916-930`): it makes only `Dropped` durable, spawns
`finish_purge`, and returns after a catalog barrier (`967-990`). Close/delete and `Retired` happen later
(`996-1004`). The full and isolated retention suites currently reproduce engine-record 404 while the
backing stream remains 200. This makes scripts, audits, failover, and rollback unable to distinguish
terminal deletion from queued intent.

**Exact task changes.** Add `ENG-014 — Define and implement retirement-operation completion`, and make
`SEC-005`, `OPS-007`, and `OPS-009` depend on it. Choose one wire contract:

- synchronous: terminal 2xx only after close/delete succeeds and `Retired` is durable; cancellation
  must not cancel the process-owned operation, and an idempotent retry must join its completion; or
- asynchronous: return `202` with a durable operation ID and `retirement_pending`, expose an
  authenticated status endpoint, and return terminal success only after durable `Retired`.

Do not return terminal `200 {ok:true}` while DS can still serve the stream. State separately when the
engine registry becomes 404, when a tail is closed, when data is deleted, and when completion survives
restart.

**Operation-count acceptance.** Run 10,000 purges and 100 faults at each of `Dropped` enqueue/durable,
registry removal, stream close, stream delete, `Retired` enqueue/durable, response loss, and restart.
Every terminal 2xx implies engine 404, DS 404/410 (or a specified terminal closed state), and durable
`Retired`; every 202 remains queryable and converges after restart. No stream is readable after terminal
completion.

### P0-05 — “Least privilege” conflicts with runtime self-provisioning

**Evidence.** Every boot creates `FOR ALL TABLES` publication when absent, explicitly requiring
superuser (`apps/engine/src/pg.rs:709-723`), and runs `ALTER TABLE ... REPLICA IDENTITY FULL` for every
tracked table (`apps/engine/src/engine/mod.rs:1195-1211`, `pg.rs:551-558`). The same runtime also creates,
drops, and recreates slots (`pg.rs:586-627`). This is not a least-privilege runtime role, and `FOR ALL
TABLES` contradicts the production schema allowlist/projection boundary in `OPS-003`.

**Exact task changes.** Split `OPS-003` into an admin bootstrap/migration job and runtime preflight.
Bootstrap must create an explicit-table publication (never `FOR ALL TABLES` in production), set replica
identity under bounded locks, create the slot, and grant only the selected tables. Runtime roles should
have replication/use and read-query permissions but no table ownership, publication creation, or slot
drop; epoch reset must be a separate, explicitly authorized operation/credential. Make production mode
observe and refuse drift instead of silently performing privileged DDL. Publish the exact PG16 role and
managed-provider capability matrix.

**Operation-count acceptance.** Bootstrap a blank PG16 instance 100 times and require runs 2-100 to
produce no DDL/grant diff. Revoke owner/superuser/CREATE from runtime roles, rotate each credential 100
times, and process 10,000,000 writes across every allowed table. For 100 forbidden DDL, unselected-table,
slot-reset, and credential misuse attempts, require denial plus stable preflight/audit evidence without
losing allowed changes.

### P0-06 — A catalog-application failure is allowed to boot empty or partially restored

**Evidence.** Boot correctly treats an unreadable catalog as fatal (`apps/engine/src/engine/mod.rs:1217-1239`),
but after epoch verification it handles `apply_catalog` failure by logging “catalog restore failed
(continuing empty)” and continues to create the sequencer, spawn replication, and become active
(`1273-1321`). Application is stateful and can fail after some records have been installed, so the
result need not even be cleanly empty. This defeats the earlier fail-closed read and makes restore,
upgrade, and failover tests incapable of asserting that all acknowledged shapes are either restored or
intentionally retired.

**Exact task changes.** Add `ENG-015 — Make catalog application transactional and boot-fatal`, depended
on by `OPS-002`, `OPS-004`, and `OPS-005`. Refactor application into a side-effect-free validation/plan
phase and a commit phase. Validate every record, stream `HEAD`, schema generation, offset segment,
pending retirement, and required seed before registering anything. A failure before commit must leave no
engine mutation and keep readiness false. If atomic application cannot be implemented, terminate with a
stable restore error; the only continue-empty path must be a separate authorized epoch reset that first
durably drops/retires every catalog shape. A busy leader must likewise remain unready and must not apply
the catalog until it owns the downstream fence from P0-03.

**Operation-count acceptance.** Use catalogs with 10,000 active/dormant/aggregate/subquery shapes and
10,000 lifecycle/checkpoint events. Inject 100 failures at every validation and commit step and 100
crashes between each applied record. Every restart either restores the complete identical state or
remains unready with the original catalog unchanged; zero runs serve an empty/partial registry. Then
process 10,000,000 mutations after each successful restore and compare logical state with SQL.

### P1-06 — The production package must encode DS as a singleton stateful component, not generic HA

**Evidence.** The only deployment is demo Compose. It defaults DS to memory mode
(`docker/compose.yaml:37-55`), exposes PG, DS, engine, and API on host ports (`21-95`), uses plaintext
credentials/HTTP, and gates engine with a liveness-style root probe. `Dockerfile.ds` builds the pinned
0.1.5 server with default features and runs it as root (`docker/Dockerfile.ds:9-16`). No repository
contract proves a data-directory ownership lock, replicated DS service, online snapshot, or multi-writer
filesystem safety. A PDB cannot make this singleton highly available.

**Exact task changes.** In `OPS-001A/B`, specify exactly one DS pod and one engine for the first release;
DS must use one RWO PVC, `Recreate`/no-surge rollout, stable store UUID, explicit mount ownership, and a
startup refusal when the volume is already owned. State DS as a single point of failure with measured
RPO/RTO. Give DS and engine distinct liveness, readiness, and startup probes; use `/ready` for engine
traffic. Add non-root/read-only-rootfs/capability/seccomp settings, bounded writable temp/spill volumes,
default-deny policies, private services, TLS/auth identities, resource limits, termination ordering, and
PVC expansion procedure. Do not claim active/passive DS until replication exists.

**Operation-count acceptance.** Perform 100 clean installs, 100 no-surge engine/DS rollouts, 100 forced
terminations at each shutdown phase, 100 attempted second-DS mounts/starts, and 100 PVC expansion/rebind
cycles. Exactly one writer may own a store; policy probes must exercise every allowed and denied edge on
every install; 10,000,000 continuous writes must either converge or cross a documented refetch boundary.

### P1-07 — Production config cannot attest that DS is durable or reject many unsafe typos

**Evidence.** Engine config accepts unknown `ELECTRIC_*` variables as logged no-ops
(`apps/engine/src/config.rs:1-11`), accepts `ds_url` without parsing or store/durability attestation
(`184-203`), silently ignores malformed Prometheus ports/pool sizes (`260-278`), defaults diagnostic
trace endpoints on (`280-282`), and retention parsing silently substitutes defaults
(`apps/engine/src/retention.rs:79-107`). The Prometheus port is accepted while its listener is not
implemented (`apps/engine/src/main.rs:65-70`). No DS endpoint in this repository reports WAL versus
memory mode, store identity, layout version, recovery completion, or free-space reserve, so `ENG-012`
cannot presently prove its promised “reject memory DS.”

**Exact task changes.** Define an explicit production profile and exhaustive allowlist of environment
keys. Strictly parse all URLs, addresses, booleans, durations, sizes, and cross-field relationships;
unknown/no-op or malformed production keys are fatal. Add an authenticated DS preflight/readiness API
returning durability mode, store UUID, on-disk format/layout, recovery/checkpoint state, usable/free
bytes, and telemetry capability. Pin expected store UUID/stack ID in engine config. Implement the
dedicated metrics listener or reject the variable. Default debug/trace off, split admin/data/metrics
binds, and require TLS-enabled PG and DS clients (the current engine has neither: `apps/engine/src/pg.rs:36-47`,
`apps/engine/Cargo.toml:43-49`).

**Operation-count acceptance.** Generate tests from the configuration schema: one valid, one missing,
one malformed, and boundary−1/boundary/boundary+1 case per key, plus all pairwise unsafe combinations.
Perform 100 boots each against memory DS, a replaced store UUID, incomplete recovery, plaintext PG/DS,
public debug bind, missing spill, and every single-character manifest-key typo. All refuse before
readiness, catalog mutation, or slot feedback; 100 valid boots expose identical redacted config.

### P1-08 — Upgrade/rollback can silently skip catalog records and its acceptance is vacuous

**Evidence.** `CatalogEvent` has no explicit on-disk format envelope/version, and `fold_catalog`
silently continues when a well-formed JSON value cannot deserialize as a known event
(`apps/engine/src/engine/catalog.rs:825-866`, especially `858`). An unknown new-version event can thus
vanish from an older reader's state instead of failing closed. Root Rust and Node versions are both
`0.0.0` (`Cargo.toml:5-8`, `package.json:1-5`). `OPS-005` allows rollback to “fail before mutation” but
does not require testing rollback after the new binary has emitted new-format records; a preflight-only
failure could satisfy it without demonstrating useful rollback.

**Exact task changes.** Amend `GOV-004`/`OPS-005` to define a versioned storage envelope and compatibility
matrix. Unknown event type/version, missing event ID, invalid payload, and invalid ordering must be
boot-fatal with the exact stream offset; never skip. Check the whole catalog/change-log format before
any storage or PG mutation. Check in immutable N and N−1 fixtures and a resumable migration journal.
Acceptance must explicitly cover `N -> N+1 -> N+1 writes -> N`; if N cannot read those writes, declare
rollback unsupported and require an export/retire/reseed operation rather than calling pre-mutation
refusal rollback support.

**Operation-count acceptance.** Each fixture must contain at least 10,000 lifecycle/catalog events and
10,000 complete source transactions spanning every event/control type. Run 100 crashes at every
migration write boundary, 100 response-loss retries, 100 unknown-event injections, and 100 malformed or
reordered events. The fold is byte-for-byte/state-equivalent after resume, or boot refuses before any
mutation; zero events are silently skipped.

### P1-09 — PG16 promotion continuity is assumed where current epoch logic explicitly defers it

**Evidence.** The epoch records timeline but intentionally never acts on a change
(`apps/engine/src/engine/epoch.rs:21-27`, `399-407`; `pg.rs:630-635`). System identifier equality and a
same-named non-lost slot are currently enough to continue. That does not prove that a promoted PG16
standby's logical slot contains the exact acknowledged WAL history; slot synchronization/failover is a
provider/topology capability, not a consequence of the name surviving. `OPS-004` lists primary
failover but supplies no continuity proof or conservative default.

**Exact task changes.** Add a PG16 failover decision table to `OPS-004`: for the first release, every
primary promotion causes a typed epoch reset/full refetch unless a separately qualified provider-specific
mechanism proves slot provenance and required LSN availability across the old/new timelines. Persist the
proof in the epoch/restore manifest. Missing, stale, advanced, invalidated, or merely same-named slots
must not resume. Define whether application writes are paused during reset/backfill and the resulting
RPO/RTO/load limits.

**Operation-count acceptance.** Run 100 promotions at every enumerated point around source commit, DS
append, slot feedback, checkpoint, and failover. Test missing, stale, advanced, lost, and same-named
foreign-provenance slots. Across 10,000,000 writes, only proven continuity may resume; every other case
must emit one typed epoch/refetch boundary and converge with SQL within declared bounds.

### P1-10 — DS observability and runbook acceptance are not currently implementable or objective

**Evidence.** `OPS-006` requires DS fsync/WAL/disk/FD metrics (`notes/16...:773-792`), but the DS image
only `cargo install`s 0.1.5 with default features and passes host/port/data/memory flags
(`docker/Dockerfile.ds:9-16`); no repository packaging enables or validates the upstream optional
telemetry surface. The Compose DS probe only curls `/` (`docker/compose.yaml:51-55`). `OPS-007` asks an
“agent unfamiliar with the implementation” to execute every scenario (`notes/16...:794-810`), which is
a subjective human exercise rather than repeatable acceptance.

**Exact task changes.** Make `OPS-006` depend on a DS qualification/package subtask that pins its feature
set and exposes authenticated readiness and scrape endpoints. Require fsync latency/errors, WAL bytes
and checkpoint/recovery progress, disk usable/free/reserve, FD and connection use, append/read latency
and errors, and store identity; record units, label bounds, and alert thresholds. Rewrite `OPS-007` as a
versioned scenario manifest with noninteractive `diagnose`, `recover`, and `verify` scripts, stable exit
codes/status JSON, fixtures, safety interlocks, and exact final-state oracles. Prose can explain the
scripts but cannot be the acceptance mechanism.

**Operation-count acceptance.** Under 10,000,000 foreground writes, scrape every component once per
configured interval with zero missing required series and bounded labels. Trigger each alert 100 times,
including fsync error, stalled checkpoint, low/reserve/full disk, recovery failure, busy/lost slot,
catalog retry, retirement retry, queue saturation, and stale leader. Execute every nondestructive
runbook scenario 10 times and every destructive/failover scenario at 100 enumerated cut points; verify
machine-readable final state and that unresolved/broad targets are rejected 100/100 times.

### P1-11 — Release artifacts omit DS and do not bind deployment evidence to digests

**Evidence.** The image workflow matrix publishes engine, node, and combined Electric images, but not
the standalone DS image (`.github/workflows/docker.yml:23-68`). There is no production gateway image or
chart, image signing, SBOM/provenance/scanning step, or machine-readable digest manifest. Runtime bases
are mutable, the DS and engine images run as root, and the Node image copies the whole repository then
runs non-frozen `pnpm install` (`docker/Dockerfile.node:8-17`). CI runs local Rust/TS tests only
(`.github/workflows/ci.yml:12-54`), not the documented external Electric oracle lane. `OPS-009` does not
list the exact artifact bill of materials that its generic “missing artifact” gate will inspect.

**Exact task changes.** Add to `GOV-004` and `OPS-009` a closed artifact inventory: engine, qualified DS,
gateway (and API only if supported), Helm/Kustomize package, DB bootstrap job/SQL, migration and rollback
tools, runbook bundle, dashboards/alerts, protocol fixtures, SBOMs, vulnerability/license reports,
signatures, provenance, external Electric-oracle results, fault/capacity raw data, and a manifest binding
source/config/test evidence to immutable image/chart digests. Production manifests must consume those
digests, never floating tags. Pin build/runtime bases by digest, use locked/offline dependency installs,
and run as numeric non-root users with minimal contents.

**Operation-count acceptance.** Build from three clean checkouts, verify every signature/provenance edge
and install only from the evidence manifest. For each artifact type, perform 10 promotion attempts with
it missing, stale, unsigned, digest-mismatched, or failed; all must be rejected. Perform 100 clean
digest-pinned installs and 100 N/N−1 rollout/rollback attempts under continuous traffic totaling at
least 10,000,000 writes; state converges or receives the documented refetch boundary.

### P1-12 — Disk-full and corruption acceptance must cover every durable artifact and preserve reserve

**Evidence.** Engine-side shape-byte accounting undercounts streams written before restart because DS
has no per-stream size API (`apps/engine/src/retention.rs:27-31`). Catalog, segmented input, active and
dormant outputs, pending retirement, and DS WAL share the same durability domain, yet `OPS-002` only asks
to corrupt/truncate “one artifact” (`notes/16...:710-715`). A single easy corruption does not prove that
valid-looking row data, metadata, lane state, closed/current segments, or partially restored object-tier
content is detectable. It also does not reserve space for the `Dropped`, rotation, checkpoint, and
`Retired` records needed to fail safely.

**Exact task changes.** Expand `ENG-010`/`OPS-002` with a checksum tree and complete inventory for catalog,
current/closed changes, active/dormant shape streams, stream metadata, every WAL shard/lane marker, and
every object-tier manifest/object. Persist exact logical and physical byte accounting across restart.
Define separate admission and emergency reserves: stop new shapes/backfills before the reserve, pause PG
acknowledgement before unlanded changes, and preserve enough space for rotation/checkpoint/retirement or
use an independently provisioned metadata volume. Specify ENOSPC handling at append, fsync, checkpoint,
rotation, close, delete, and spill; “log and keep growing” is not recovery.

**Operation-count acceptance.** Flip/truncate/omit/duplicate/reorder 100 samples in every artifact class
and restore each onto an empty host; all corruption is detected before serving or resolves through a
named full reset. Apply hard quotas at 100 cut points in every write/fsync/checkpoint/rotation/retirement/
spill phase while processing 10,000,000 mutations. Restart must preserve accounting and no slot,
sequencer checkpoint, or response may advance past data that did not land.

## Required specification edits before implementation starts

1. Break the dependency cycles and publish the machine-checked DAG.
2. Narrow G3/G8 to offline DS backup, singleton DS/engine, and no continuity-preserving PG promotion for
   the first supported release.
3. Add `ENG-014` retirement-operation completion and resolve the existing purge 200-vs-pending contract.
4. Add `ENG-015` transactional/fail-closed catalog application; delete the “continuing empty” boot path.
5. Replace advisory-lock wording with downstream-enforced fencing, or remove automated failover from the
   supported topology.
6. Make DS store identity/frontier attestation, strict catalog decoding, and admin/runtime PG credential
   separation explicit prerequisites—not details deferred to runbooks.
7. Replace subjective/vague acceptance with the numeric fault, operation, boot, install, and corruption
   matrices above, and require their raw evidence in the release bundle.
