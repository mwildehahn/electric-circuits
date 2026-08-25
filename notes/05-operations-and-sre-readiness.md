# Operations and SRE readiness audit

Audit scope: the deployable Docker/Compose path, engine/API configuration and operations
surfaces, Postgres logical replication, durable-streams lifecycle, and the documented Kubernetes
example. This is a code-and-documentation audit, not a penetration test or a recovery drill.

Status labels:

- **[Confirmed]** is implemented and evidenced by the cited code.
- **[Missing implementation]** is not present in this fork and is a launch blocker unless an
  external platform control demonstrably provides it.
- **[Missing documentation]** is needed to operate the implemented behavior safely.
- **[Documentation conflict]** describes material disagreement between sources.

## Executive assessment

The Rust engine is unusually careful about single-writer correctness: it waits for a durable catalog
before acknowledging shape lifecycle operations, protects a replication slot with a catalog epoch,
uses a separated readiness state, and drains safely on `SIGTERM`. Those are meaningful production
building blocks. The repository does **not** yet provide a production deployment package or complete
operations model. The supplied Compose file is a local-development stack: it defaults the durable
log to memory, publishes every service, uses a known database password, has no TLS or authentication
boundary, and gives Docker less shutdown time than the engine needs. The system should not be exposed
or launched as a multi-replica/HA service until the P0/P1 gates below are closed.

## Findings and evidence

### P0 — do not expose the supplied stack

1. **The Compose deployment is non-durable by default. [Confirmed]**

   `docker/compose.yaml:39-47` sets `DS_MEMORY` to `1` by default, and
   `docker/Dockerfile.ds:13-14` turns a non-empty value into `--durability memory`. This makes the
   catalog, segmented change log, and all shape streams disappear when `ds` restarts even though a
   `ds-data` volume is mounted. The comment calls this out, but `pnpm docker:up` remains an unsafe
   default for a production-looking command. A lost catalog also defeats the engine's otherwise
   correct epoch/restore contract.

   **Launch requirement:** make file/WAL durability the non-overridable production default; reject
   `DS_MEMORY` in a production profile; use a durable volume with tested snapshots and restoration.

2. **All public services and critical mutation streams are unauthenticated/plain HTTP. [Confirmed;
   Missing implementation]**

   Compose publishes Postgres, durable-streams, engine, and API on the host
   (`docker/compose.yaml:29-31,48-50,64-65,86-87`). The durable-streams endpoint is both the data
   plane and the authoritative catalog/change-log storage; the engine client performs unauthenticated
   `PUT`/`POST`/read requests (`apps/engine/src/ds.rs:287-300`). Thus anyone able to reach DS can
   read, append to, close, or delete materialized data depending on DS's HTTP behavior.

   `ELECTRIC_SECRET` guards only `GET /v1/shape` (`apps/engine/src/electric.rs:820-846` and
   `apps/engine/src/config.rs:497-505`). The native control-plane router has no authentication
   middleware and includes shape creation/release/purge, table writes, `/epoch/reset`, metrics reset,
   and data-bearing debug routes (`apps/engine/src/http.rs:23-71`). The tRPC router also declares
   ordinary public procedures, including `schema.define` and `ingest.write`
   (`apps/api/src/router.ts:44-100`). The code explicitly documents enabled trace/graph/state as
   unauthenticated (`apps/engine/src/http.rs:19-22`); those routes expose row-derived state.

   **Launch requirement:** keep Postgres, DS, engine control plane, and API private; expose only a
   separately authenticated gateway. Add service-to-service authentication plus authorization for
   tenant/control operations, or supply equivalent mTLS/network policy/identity controls. Disable
   `ELECTRIC_CIRCUITS_TRACE` outside a protected operator network. Treat `POST /epoch/reset`,
   `DELETE ...?purge=true`, `/metrics/reset`, and table-write endpoints as admin-only.

3. **There is no in-process TLS to Postgres or durable-streams. [Confirmed; Missing implementation]**

   Postgres is always connected with `NoTls` (`apps/engine/src/pg.rs:36-47`). The engine image
   explicitly describes its DS traffic as plain HTTP/no TLS backend (`docker/Dockerfile.engine:9-12`),
   and Compose configures `http://` URLs (`docker/compose.yaml:55-56`). A secret in the `/v1/shape`
   query string is also more prone to logging/proxy/referrer disclosure than an authorization header.

   **Launch requirement:** use encrypted, authenticated transport end-to-end or verified sidecar/
   mesh termination for PG, DS, API, and edge traffic; document certificate rotation and CA trust.
   Do not put `ELECTRIC_SECRET` URLs in access logs. The current code cannot itself satisfy a direct
   TLS-to-Postgres requirement.

4. **The example database is openly reachable with fixed credentials. [Confirmed]**

   `docker/compose.yaml:23-31` uses `postgres:16`, `POSTGRES_PASSWORD: password`, and publishes
   port 5432. This is acceptable only for a local demo. There is no production Secret reference,
   role separation, `pg_hba` hardening, or password/certificate rotation in Compose.

### P1 — correctness is solid for one active engine, but availability/operability is incomplete

5. **The implementation is deliberately single-active-engine; it is not HA. [Confirmed; Missing
   implementation]**

   A slot permits one walsender. The engine avoids a second ingestor
   (`apps/engine/src/engine/mod.rs:1144-1147,1285-1312`), and the Kubernetes example explicitly
   requires `replicas: 1` and explains that a successor waits on a busy slot
   (`docs/deployment-postgres.md:126-136`). The epoch ADR says timeline/failover handling beyond the
   system-identifier check is deferred for a single primary (`docs/adr/0004-slot-epoch-and-reset.md:34-40`; see
   also `apps/engine/src/engine/epoch.rs:399-406`). There is no leader election, fencing lease,
   standby/read replica strategy, DS replication protocol, or cross-zone failover design.

   **Launch requirement:** deploy exactly one active engine per slot/DS catalog and document the
   manual failover procedure. Do not configure an HPA or a multi-replica Deployment. HA needs a
   designed leader/fencing protocol and replicated durable-streams storage, then failover tests.

6. **Logical-replication setup is self-provisioning and privileged. [Confirmed]**

   Boot verifies `wal_level=logical` and refuses irreparable setup problems
   (`apps/engine/src/engine/mod.rs:1147-1159`; `apps/engine/src/pg.rs:229-245`). It creates or
   adopts `<slot>`, creates `<slot>_pub FOR ALL TABLES`, and changes each selected table to
   `REPLICA IDENTITY FULL` (`apps/engine/src/engine/mod.rs:1191-1211`; `apps/engine/src/pg.rs:586-723`).
   The deployment guide correctly calls for replication slots/walsenders and names ownership,
   `SELECT`, and replication permissions (`docs/deployment-postgres.md:26-51,391-392`).

   Risks requiring an explicit production decision:

   - `FOR ALL TABLES` has a larger capture/privilege blast radius than the selected list. The engine
     filters untracked relations after decoding, not at publication selection.
   - `REPLICA IDENTITY FULL` increases WAL for updates/deletes and needs `ACCESS EXCLUSIVE` locking
     when applied; migrate/change windows must account for it.
   - The guide says each table needs a single-column PK (`docs/deployment-postgres.md:46-47`), but
     deployment validation is not provided as a preflight script.

   **Missing documentation:** a least-privilege SQL bootstrap that states exactly which roles perform
   publication/slot/table alteration, how a managed provider delegates them, and how the application
   role is kept separate from the engine role.

7. **Slot-loss safety is implemented, but it needs a chosen operational policy and backfill budget.
   [Confirmed; Missing documentation]**

   `SlotBound { system_identifier, timeline_id, slot }` is stored in the durable catalog and checked
   before boot/reconnect; a missing/lost/foreign slot or different cluster creates an epoch break
   (`apps/engine/src/engine/epoch.rs:179-207,361-396`). The default auto-reset retires all shapes;
   `ELECTRIC_CIRCUITS_RESET_ON_SLOT_LOSS=false` fails closed until `POST /epoch/reset`
   (`docs/deployment-postgres.md:298-319`). This prevents silent data divergence.

   The automatic option can create an unbounded re-subscribe/backfill storm after a slot loss. The
   current default should not be accepted casually: choose false for systems where a controlled
   outage is safer than automatic load, or prove capacity for the resync. There is no documented
   operator approval/authorization/audit mechanism for the destructive reset endpoint.

8. **Catalog and pending-retirement recovery are robust, but backup/restore is absent. [Confirmed;
   Missing implementation; Missing documentation]**

   On startup, catalog folding restores durable checkpoints and requeues `Dropped` streams missing a
   `Retired` event (`apps/engine/src/engine/catalog.rs:825-866,878-908`). A catalog that cannot be
   read is deliberately fatal, preventing a new slot at WAL head from silently resuming old shapes
   (`apps/engine/src/engine/mod.rs:1217-1239`). This is the right failure mode.

   No code or runbook implements a DS backup, consistent snapshot, checksum/restore verification,
   retention policy, encryption, or DR exercise. A restore must preserve a mutually consistent set of
   `meta/catalog`, `changes/*`, and retained `shape/*` streams; restoring only one can produce either
   boot refusal or an incorrect resume. There is also no supported catalog migration: pre-ADR formats
   refuse boot and instruct the operator to reset DS data (`apps/engine/src/engine/catalog.rs:469-530`).

   **Documentation conflict:** `docs/deployment-postgres.md:331-337` still says native deletes answer
   before their catalog record lands. The current handler instead awaits the durable release/purge
   operation (`apps/engine/src/http.rs:555-581`), and its handler documentation says so
   (`apps/engine/src/http.rs:563-569`). An incident runbook based on the older text would misread a
   blocked delete during a DS outage.

9. **Disk controls exist but do not form a complete capacity control plane. [Confirmed; Missing
   implementation]**

   Change logs segment by size/age and delete only after durable progress and dormant-shape pins
   permit it. The operating guide provides an initial sizing formula and warns that disabling both
   rotation criteria is unbounded (`docs/deployment-postgres.md:203-214`). Transaction spill is
   bounded/probed at boot, but downstream transaction handling still scales with the largest commit
   (`docs/deployment-postgres.md:215-236`). Shape count/TTL retention exists; the optional shape-disk
   budget intentionally undercounts pre-restart streams because DS offers no per-stream sizes
   (`apps/engine/src/retention.rs:21-34,56-107`; `apps/engine/src/ds.rs:237-241`).

   No hard DS filesystem quota, free-space admission control, storage-pressure readiness change, or
   persistent exact per-stream accounting is implemented. The Compose DS volume has no explicit
   size/IOPS class. Postgres WAL retention is observable but not controlled by the engine.

10. **Health and orderly engine shutdown are strong; Compose/API/DS orchestration is not. [Confirmed;
    Missing implementation]**

    `/health` is liveness and `/ready` becomes 200 only after slot verification, catalog restore, and
    ingest start (`apps/engine/src/http.rs:94-127`; `apps/engine/src/engine/mod.rs:1125-1142`). On
    `SIGTERM`, readiness becomes 503 first, then the ingestor/sequencer/catalog reach safe points
    within a bounded grace (`apps/engine/src/shutdown.rs:1-57`; `apps/engine/src/main.rs:255-336`).
    The Kubernetes sample aligns a 25 s engine grace with a 30 s pod grace and gives correct probe
    semantics (`docs/deployment-postgres.md:118-187`).

    In contrast, Compose gives no `stop_grace_period`; Docker's normal 10 s stop timeout is shorter
    than the default 25 s engine grace. Its engine healthcheck uses only `/health`
    (`docker/Dockerfile.engine:34-36`), so it is healthy while still waiting for Postgres/catalog.
    `api` starts when engine is merely started, has no healthcheck, and only handles `SIGTERM`
    (`docker/compose.yaml:79-92`; `docker/api-server.ts:14-17`). The DS health check only verifies
    a root HTTP response (`docker/compose.yaml:51-55`), not durable write/read readiness.

    **Launch requirement:** use a production orchestrator with `/ready` routing, liveness `/health`,
    termination grace greater than engine grace plus margin, DS/API readiness, startup ordering, and
    a tested pod/node termination drill. Never use the Docker healthcheck as routing readiness.

11. **Configuration validation is partial and has important silent/no-op cases. [Confirmed;
    Missing implementation]**

    The resolver rejects malformed PG URLs, table selectors, unsafe transaction/backfill byte budgets,
    and inconsistent shutdown durations (`apps/engine/src/config.rs:176-246,332-380`); spill storage
    is probed before boot (`apps/engine/src/main.rs:79-86`). That is good operational ergonomics.

    But `ELECTRIC_CIRCUITS_DS_URL` is only required as a non-empty string and is not parsed/validated
    at resolution (`apps/engine/src/config.rs:188-193`; `apps/engine/src/main.rs:72-77`). Several
    retention settings use `parse().ok().unwrap_or(default)`, silently accepting invalid values
    (`apps/engine/src/retention.rs:79-107`). Unknown `ELECTRIC_*` variables are intentionally logged
    as accepted no-ops (`apps/engine/src/config.rs:408-451`), and
    `ELECTRIC_PROMETHEUS_PORT` is parsed but explicitly unimplemented
    (`apps/engine/src/main.rs:65-70`).

    **Launch requirement:** add a `--check-config`/preflight that resolves all values strictly,
    validates DS reachability/durability and filesystem access, checks the PG role/settings/slot
    capacity/table PKs, and fails closed on production-only no-op variables.

12. **Observability is useful but not production-complete. [Confirmed; Missing implementation;
    Documentation conflict]**

    The engine exports JSON counters/histograms and Prometheus text on its main port
    (`apps/engine/src/http.rs:784-815`), including replication retained-WAL/flush-lag/active gauges
    sampled every 10 seconds (`apps/engine/src/metrics.rs:287-367`). It also supports StatsD and
    structured `tracing` diagnostics. Counters cover catalog retries, pending retirements, spills,
    segment retention, schema/epoch events, and shutdown (`apps/engine/src/metrics.rs:99-171,212-251`).

    There are no checked-in Prometheus scrape configs, dashboards, alert rules, SLOs, log schema/
    correlation IDs, distributed tracing exporter, or alert-routing/runbook links. `/metrics/reset`
    is an unauthenticated mutation when the control port is reachable. `ELECTRIC_PROMETHEUS_PORT`
    does **not** create a listener, despite fleet documentation describing one
    (`docs/fleet-conformance.md:69-72,139-147` versus `apps/engine/src/main.rs:65-70`).

    Minimum alerts are listed in the runbook below; their thresholds must be set from observed WAL
    rate, free space, resync capacity, and SLOs—not copied blindly.

13. **Image supply-chain and runtime hardening are incomplete. [Confirmed; Missing implementation]**

    The engine and Node runtime images run as root (no `USER` after their runtime `FROM`s in
    `docker/Dockerfile.engine:17-36` and `docker/Dockerfile.node:9-17`); only the fleet image drops
    to `node` (`docker/Dockerfile.electric:61-66`). Images use mutable base tags rather than digests
    (`docker/compose.yaml:21`, Dockerfiles), and CI builds/pushes engine/node/electric but not a DS
    image (`.github/workflows/docker.yml:27-35`), although production Compose needs `Dockerfile.ds`.
    There is no SBOM, vulnerability scanning, signing/attestation, admission policy, dropped Linux
    capabilities, read-only root filesystem, or resource limits.

    **Documentation conflict:** `docker/README.md:79-87` describes published images but omits the
    separately required DS image; CI confirms it is not published. A deployment cannot reproduce the
    Compose DS image solely from pinned registry artifacts.

14. **Schema migration behavior is intentionally safe but requires an application runbook. [Confirmed;
    Missing documentation]**

    DDL/`TRUNCATE`/replica-identity regression retires affected shapes; unresolved schemas park the
    table rather than serving stale data (`docs/deployment-postgres.md:356-381`). Counts-pipeline
    table changes cause exit 75 after retirements, requiring a restart (`docs/deployment-postgres.md:368-370`).
    Adding/recreating a table requires a restart (`docs/deployment-postgres.md:199-202,388-390`).

    There is no migration orchestration guide specifying client UX, traffic throttling, the exit-75
    restart action, ordering with application migrations, or a tested rollback policy. A database
    rollback/restore can trigger an epoch break; it is not a transparent rollback of live streams.

15. **Fault facilities test correctness but are not a production chaos programme. [Confirmed;
    Missing documentation]**

    Test-only negative-control faults are environment-controlled and logged at boot
    (`apps/engine/src/fault.rs:1-34`; `apps/engine/src/main.rs:88-91`), and fleet testing uses a
    toxiproxy PG path (`docs/fleet-conformance.md:24-29`). There is no repeatable production-like
    fault matrix for DS unavailability, partial DS persistence loss, Postgres promotion/restore, slot
    WAL loss, disk-full, long transactions, network partitions, forced termination, or backup restore.

## Production launch gates

All P0 items and the following gates should be signed off before customer traffic:

- [ ] A versioned deployment definition exists (Kubernetes/nomad/etc.), not the supplied Compose
  file; it sets CPU/memory/ephemeral-storage requests and limits, no-file limits, non-root users,
  capability drop, network policies, and an explicit termination grace.
- [ ] DS runs in file/WAL durability on replicated/persistent storage. `DS_MEMORY` is prohibited.
  Its backup, retention, encryption, restore, and integrity-verification procedures have passed a
  full restore test that includes catalog, changes, and shape streams.
- [ ] All externally reachable traffic has TLS and authenticated authorization. DS, PG, control,
  debug, and metrics routes are private; admin recovery is separately authorized and audited.
- [ ] A least-privilege PG bootstrap is approved: `wal_level=logical`, `max_replication_slots`,
  `max_wal_senders`, `max_slot_wal_keep_size`, publication/slot ownership, table ownership for
  replica identity, and application-vs-engine roles. Capacity reserves one slot/walsender per active
  engine plus operational headroom.
- [ ] Exactly one active engine owns each `<PG cluster, slot, DS catalog>` tuple. The deployment
  prevents surge/HPA replicas and documents manual successor takeover. The HA claim is explicitly
  **single-primary/manual failover**, not active-active.
- [ ] `ELECTRIC_CIRCUITS_RESET_ON_SLOT_LOSS` is chosen deliberately. If true, load testing proves
  the full resubscribe/backfill storm; if false, the on-call reset approval and client communication
  path are rehearsed.
- [ ] `/ready` is the traffic gate and `/health` the liveness gate. API and DS have equivalent
  readiness checks. Termination grace exceeds `ELECTRIC_CIRCUITS_SHUTDOWN_GRACE_SECS`; the ready
  drain covers the real load-balancer probe interval/failure threshold.
- [ ] Alert rules, dashboard ownership, paging thresholds, and the runbooks below have an on-call
  owner. Metrics are scraped from `/metrics/prometheus` on the main protected port until the separate
  Prometheus listener is actually implemented.
- [ ] Capacity test results cover peak live subscriptions, write/WAL throughput, largest transaction,
  concurrent backfills, DS IOPS/space, PG connections, and a slot-loss resync. Size source PG WAL for
  the maximum detection + repair + planned-engine-downtime interval.
- [ ] Upgrade and rollback are rehearsed against a clone of real catalog/DS data. Do not treat a
  major PG upgrade, PITR, or catalog-format rejection as an in-place rollback.

## Concrete production runbook checklist

### Before first deployment or a change

1. Record release image digests, engine version, DS version, PG version, schema migration ID, slot
   name, publication name, DS snapshot ID, and config checksum in the change record.
2. Validate storage: DS is durable, encrypted/backed up, has alerting on free space/IO latency, and
   has room for retained change segments, all shape streams, compaction overhead, and recovery headroom.
   Validate separate engine spill storage for the largest expected transaction.
3. Validate Postgres using an approved operator account: `SHOW wal_level`; slot/walsender limits;
   disk/free WAL headroom; `pg_replication_slots` for the named slot; publication contents; selected
   tables' primary keys and `relreplident`; and required grants. Do not manually drop/recreate a live
   slot.
4. Validate network policy and identity: no public DS/PG/control endpoint, TLS/mTLS paths tested,
   secrets are mounted from the secret manager, and no proxy logs full query strings.
5. Run strict preflight/config validation (until implemented, perform this manually), then deploy one
   engine only. Wait for `GET /ready` to return `200 {"status":"active"}`; do not infer readiness
   from a bound port or `/health`.
6. Smoke test one create/update/delete and one client reconnection; confirm increasing process metrics
   and a live stream update. Capture `/replication/lsn`, `/metrics`, and DS/PG health baselines.

### Normal monitoring and alerts

1. **Page:** `/ready` non-200 after the agreed startup/maintenance window; engine exit 74, 75, 78,
   or 70; `epoch_breaks_total` changes; `schema_unresolved_total` changes; or epoch state is broken.
2. **Page before data loss:** `replication_slot_retained_wal_bytes` approaches the smaller of
   `max_slot_wal_keep_size` and remaining PG disk budget; `replication_slot_active=0` outside a
   planned handover; rising confirmed-flush lag with sustained writes.
3. **Page:** DS write/read errors, `catalog_append_retries_total` rising for more than a short outage,
   `retirements_pending` non-zero beyond its retry budget, DS filesystem free-space/IO latency breach,
   or backup failure.
4. **Ticket/investigate:** `changes_segments_retained` trends above its capacity model,
   `retention_pressure` increases, transaction spills/chunking increase unexpectedly, subscription
   lapses increase, or 5xx/latency/SLO error budget crosses threshold.
5. Correlate alerts with PG `pg_replication_slots`, PG WAL/disk, engine structured logs, DS logs,
   rollout/restart history, and network/proxy events. Do not reset engine metrics during an incident.

### Planned engine rollout

1. Confirm DS and PG are healthy, catalog backup is current, no epoch break is present, and retained
   WAL has headroom. Freeze disruptive DDL for the handover window.
2. Remove traffic by readiness. Send one `SIGTERM`; do **not** send a second signal. Observe
   `/ready=503 shutting_down`, then clean exit 0. If exit 70 occurs, preserve logs and investigate;
   replay is expected but the forced shutdown is an incident signal.
3. Start the successor with the identical slot/catalog and one replica. It may wait while the old
   walsender releases the slot; do not start another contender or recreate the slot.
4. Require `/ready=200`, `replication_slot_active=1`, stable retained-WAL/flush lag, and a real
   change flowing to a test subscription before reopening traffic.
5. Roll back only to a version proven to understand the stored catalog format. If rollback cannot
   read the catalog, stop and use the tested migration/restore plan rather than deleting DS data.

### Postgres failover, restore, slot loss, or major upgrade

1. Declare an epoch incident; pause automated destructive actions and preserve PG/engine/DS evidence.
   Check `GET /replication/lsn` epoch state/reason and `pg_replication_slots`.
2. With auto-reset enabled, prepare for all shapes to be retired and clients to resubscribe; throttle
   client recovery/backfills and ensure PG/DS capacity before allowing the reset to complete.
3. With fail-closed policy, keep traffic unready/degraded. Obtain authorized approval, communicate the
   required resync, then invoke the protected `POST /epoch/reset` once. Verify a new `SlotBound`,
   active slot, and recovery metrics; restart if code 75 is expected for counts pipelines.
4. Never recreate the slot by hand while old shape streams/catalog records remain. A new slot cannot
   fill the unknown WAL gap.

### DS loss or restore

1. Stop engine traffic and preserve the affected DS volume. Do not let an empty/replaced DS instance
   masquerade as the old catalog.
2. Restore the tested, consistent DS backup set. Verify catalog readability, expected segment paths,
   and stream counts/checksums using the backup tool before starting the engine.
3. Start the single engine and require normal catalog restore/readiness. If the only viable recovery
   is a new DS deployment, treat it as a deliberate full resync/epoch procedure with customer impact;
   do not silently reuse the old slot.

### Schema migration / rollback

1. Announce expected retirement/resubscribe behavior for affected tables. Confirm whether a table is
   in `ELECTRIC_CIRCUITS_DBSP_COUNTS`; if so, plan for exit 75 and controlled restart.
2. Apply migration, inspect `/tables` for `unresolved`, monitor drift metrics and client must-refetch/
   404 behavior, then restart only as required. Do not suppress unresolved state by restarting blindly.
3. A database restore or structural rollback is an epoch/schema event, not merely an application
   deployment rollback. Use the epoch/DS restore procedure above.

## Recommended implementation backlog

1. Ship a production deployment package with DS durability on by default, strict production config
   validation, resource/security contexts, readiness, limits, network policies, and image digests.
2. Add mTLS/service auth plus tenant/admin authorization; split public shape, private control, and
   debug/admin surfaces. Remove query-token dependence or guarantee its redaction.
3. Implement and test DS backup/restore and catalog-format migration; publish RPO/RTO and a supported
   version compatibility matrix.
4. Add supported single-active failover automation (leader fencing) before claiming HA; separately
   design DS replication and PostgreSQL promotion semantics.
5. Publish alert rules/dashboards/SLOs and a real Prometheus listener or remove/refuse
   `ELECTRIC_PROMETHEUS_PORT`. Add DS and API telemetry and health contracts.
6. Make slot/publication/role validation a preflight command; add repeatable fault/DR drills to CI or
   release qualification.
