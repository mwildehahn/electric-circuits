# Repository test-harness audit

Date: 2026-08-23. Scope: executable repository state, compared with the intended acceptance
state in notes 18, 23, and 24. This is a read-only audit; “current” means a checked-in runner or
test, not a design promise in a note or `AGENTS.md`.

## Executive assessment

The repository already has a useful **process-level, black-box conformance harness** for the
engine's current native and extended TS surfaces:

```text
real PostgreSQL DML -> logical replication -> Rust engine -> durable-streams
  -> tRPC/TS client materialization -> set comparison with SQL SELECT on the source database
```

It has real child processes, real TCP, per-harness databases/slots, SQL comparisons, restart,
DDL, retention, storage-failure-proxy, and large-transaction scenarios. This is the right
starting seam for server E2E/TDD.

It is **not** the planned release-candidate acceptance system. No `packages/acceptance/`,
`docs/production/`, PG18 fixture, file-backed-DS candidate fixture, gateway, app/Swift runner,
scenario registry, contract hashes, fault-gate abstraction, or artifact collector exists. The
checked-in docker/demo defaults are PostgreSQL 16, and CI runs the host's newest installed PG
binary rather than asserting 18. Treat notes 18/23/24 as target architecture and scenario
inventory, not evidence that those scenarios are executable.

## What runs today

| Layer | Current executable surface | Command | What it proves / does not prove |
| --- | --- | --- | --- |
| Rust engine | One Cargo package; extensive inline/unit and Tokio tests. `tower` is the only engine dev-dependency, with Tokio `test-util` for virtual-time endpoint cases. | `cargo fmt --check`; `pnpm engine:test` (`cargo test -p electric-circuits-engine`) | Fast engine/internal and in-process HTTP coverage; no external PG/DS deployment proof by itself. |
| TS unit/protocol/client/oracle | Root Vitest collects workspace `*.test.ts`, including protocol, oracle, client, API and pipeline-viz pure tests. | `pnpm typecheck`; `pnpm test` | Typecheck covers `packages/*/src`, conformance scripts, API and docker TS only. It deliberately excludes `examples/**` and `apps/pipeline-viz`; Vitest is transpile-only. |
| Real-stack conformance | `packages/conformance/src/harness.ts` creates a database and logical slot, starts `DurableStreamTestServer`, the Rust engine process, tRPC API and the actual TS client. | `pnpm test:conformance`; `ELECTRIC_CIRCUITS_ENGINE_PREBUILT=1 pnpm test` | The strongest current high-level server regression lane. It validates engine behavior through PostgreSQL, network endpoints and materialized client state, but uses the repository test server and direct internal endpoint/API—not an immutable image, public gateway, or independent client implementation. |
| Seeded/fuzz differential | `conformance-fuzz`, `-fuzz-wide`, NULL/subquery matrices and counterexamples generate operations and shape predicates. Failing basic fuzz seeds are printed/replayable. | `pnpm test:fuzz`; `pnpm loop [N]`; `SEED=<n> pnpm exec vitest run packages/conformance/src/conformance-fuzz.test.ts` | Excellent regression amplifier. Defaults still include an unseeded base for normal fuzz, so CI failure is replayable only from its output; a fixed release corpus must set seeds and operation budget explicitly. |
| Electric adapter oracle | Elixir fixtures are copied into an Electric checkout and run with Electric's official client and `OracleHarness`/`ShapeChecker` against `/v1/shape`. | `./electric-conformance/run.sh oracle`, `property`, `subqueries`, or `all` | Independent external protocol/client harness. Not part of CI; needs Mix/Elixir and PG binaries; may clone/modify `../electric` test files. README records 13/15 subquery tests passing, with two tag-mechanism failures—not a fully green suite. |
| Docker | Compose starts PG16, DS, engine, API. Docker workflow only builds images. | `pnpm docker:build`; `pnpm docker:up`; `docker compose -f docker/compose.yaml up --build` | Developer/manual stack. `DS_MEMORY` defaults to `1`, so default compose is intentionally non-durable. There is no compose smoke/assertion or image E2E job. |
| Demo / visualizer | LinearLite starts a fresh local PG, DS, engine/API, Vite and optional pipeline visualizer. | `pnpm demo:linearlite` or `scripts/linearlite.sh start small` | Valuable manual browser smoke and visualization, not an automated browser suite. No Playwright dependency/config/test exists in this checkout; the `AGENTS.md` browser runbook relies on externally supplied browser tooling. |
| Bench/load | Bench and loadgen boot realistic stacks and emit observations. Fleet runner can exercise unmodified Electric benchmark scripts. | `pnpm bench`; `pnpm bench:fleet`; `pnpm --filter @electric-circuits/loadgen loadgen`; `... sweep` | Performance/load observation, not correctness acceptance: no fixed source journal + SQL oracle after a causal fence, no resource caps/admission assertions, and non-deterministic/time-based workloads. Fleet/load scripts can clone/use external repos or write reports/results. |

### Current CI is narrower than the local completion checklist

`.github/workflows/ci.yml` runs Rust 1.96 formatting and `cargo test`, installs Node 22/
pnpm, selects the highest installed `/usr/lib/postgresql/*/bin`, then runs `pnpm typecheck` and
`pnpm test`. It sets no PG-major contract and does not run:

- `electric-conformance/run.sh`;
- Docker compose, image smoke, demo/browser test, benchmark, loadgen, or fuzz loop;
- PG18-specific cases, gateway/app/Swift suites, candidate artifact/digest recording, or
  `cargo fmt` through the root scripted command (it invokes Cargo directly).

`.github/workflows/docker.yml` builds/pushes three images only. A successfully published image
therefore has not been exercised as a running image. The documented `AGENTS.md` full checklist
also asks for Electric's external oracle and browser demo verification, but those are not CI gates
and can be unavailable locally (the 2026-08-22 baseline records missing Mix).

## Oracle quality and barriers

### Independent-oracle status

1. **Strong current oracle for engine outcomes:** the Postgres-mode conformance harness writes to
   the source database and asks the same database `SELECT … WHERE …` for the expected relation.
   It compares declared values plus PK-set equality with the TS client materialization. This is
   independent of the engine's incremental evaluator, but it is not independent of PostgreSQL nor
   a historical source fence: it normally queries current SQL after the test's writes quiesce.
2. **Separate semantic oracle exists but is not the main real-stack oracle:**
   `packages/oracle#createOracle` is PGlite-backed and receives the same logical events. The
   real-Postgres `bootHarness` intentionally uses `createPgOracle`, not that PGlite implementation.
   Use PGlite/property unit tests to cross-check SQL/predicate semantics; do not describe the
   current process harness as dual-database differential testing.
3. **External Electric oracle:** `electric-conformance` is the closest current independent
   protocol/client oracle. It is excellent for `/v1/shape` compatibility, but environment-bound,
   not CI, and its documented tag exceptions must remain explicit rather than called “all green”.
4. **No public-product oracle:** there is no server-owned-template gateway, tenant/auth oracle,
   Swift/app cache oracle, image-level client materializer, or release profile manifest.

### Deterministic mechanisms already worth reusing

| Mechanism | Implementation | Appropriate use |
| --- | --- | --- |
| Process readiness | `ENGINE_BINDING` / `ENGINE_LISTENING` stdout markers, plus `/health`, `/ready`, `/v1/health` tests | Boot taxonomy and restart tests; replace log polling with a stable fixture API in acceptance. |
| Source-to-engine drain | `drainEngine()` updates per-DB `__el_sync` after test writes; waits for engine replication sync, sequencer `(segment, offset)` tail, then `pendingFlips == 0` | Best current causal barrier for no-concurrent-write server tests, including deferred subqueries. Promote it behind a `server.drainedThrough(sourceCommit)` receipt. |
| Controlled creation/query-back races | Real `ACCESS EXCLUSIVE` table locks plus `pg_stat_activity` lock-wait detection in `engine-native.ts` | Reliable way to park a real database read rather than guessing with a delay. Use for snapshot/live and DDL races. |
| DS fault boundary | Per-test HTTP reverse proxies in catalog/storage tests can return 503 or simulate a response lost after upstream commit | A solid prototype for `FaultGate`; generalize named, awaitable gates and request journals rather than duplicating proxies. |
| Process cuts | Harness exposes raw spawn, `SIGTERM`, `SIGKILL`, exit waits and restart against the same DB/DS/slot | Reusable for SRV restart/slot/drift scenarios. |
| Virtual time | Rust uses Tokio `start_paused`/`test-util` in focused cases | Keep as an internal invariant tier. The TS real-stack retention tests use real seconds instead. |
| Seeded schedules | Simulator uses seeded Faker; many matrices use fixed seeds; fuzz prints a failure seed | Persist seed, generated journal, first divergence and exact configs in a future acceptance runner. |

### What prevents these from being the notes-24 `CausalFence`

`drainEngine` is deliberately engine-specific: its middle receipt reads `/replication/lsn`, a
change-log tail and `pendingFlips`. It correctly avoids an empty-result false green in the existing
harness, but it does **not** establish that an actual public client/cache applied the target
template after a named same-transaction sentinel. It has no `SourceCommitID`, no per-client
application receipt, principal/template/generation binding, journal-prefix SQL query, or failure
artifact. The target design in notes 18/23/24 needs those three receipts:

```text
same source transaction (data + last sentinel)
  -> server drained through sentinel, including deferred work
  -> actual client/cache committed an observed tail after that server receipt
  -> SQL oracle restricted to the source journal prefix
```

Build this atop—not by replacing—`drainEngine`, lock helpers, raw stream fold, and current SQL
comparison.

## Stable-scenario coverage: current versus target

The scenario IDs in notes 18/23/24 are an intended stable registry. No source search finds a
`packages/acceptance`, a scenario-ID/contract-hash manifest, or files named for `PG18-E2E-*`,
`SRV-E2E-*`, `GW-E2E-*`, `SYNC-*`, `LIFE-*`, etc. The following is therefore a mapping of usable
prototypes, not a claim of fulfilled acceptance.

| Target family | Closest executable coverage now | Major missing piece |
| --- | --- | --- |
| `PG18-E2E-001` basic snapshot/live/restart | `conformance-postgres`, `-backfill`, `-restart`, `-concurrency` | Test PG binary is unspecified/host-selected; no PG18 assertion, same-transaction named journal, candidate image/file DS, or target-client receipt. |
| `PG18-E2E-002` snapshot/live fence | Concurrent writers and `lockTable` create/query-back fixtures provide useful primitives; `conformance-concurrency` has a timing race | Need a deliberately held repeatable-read snapshot with event gate and same-fence SQL proof—not `sleep(25)`. |
| `PG18-E2E-003..005, 007..014` | Real DDL, schema-drift, identity, slot/epoch and raw-DDL tests exist | No generated-column/publication/PG18/TLS/RLS/partition/failover/minor-upgrade fixture; current Docker is PG16. |
| `SRV-E2E-001, 005..007, 009..012` | Large transaction spill/chunk, catalog DS proxy, restart/shutdown, schema drift, retention and epoch tests are substantial direct-engine prototypes | No public contract fixture, durable candidate volume/image proof, named external cut matrix or raw evidence bundle. |
| `SRV-E2E-002..004, 008, 013` | Named subscriptions/sharing, purge/long-poll, dormant/evict/reactivate, retention tests | Some paths use timing sleeps; no opaque public generation/gateway ownership or exclusive RWO handoff deployment test. |
| `GW-E2E-*` | None (current engine/API/DS endpoints are intentionally direct and unauthenticated) | Implement gateway, credentials/template registry, recorder and network-isolation fixture before adding scenarios. |
| Swift/app `SYNC`, `LIFE`, `AUTH`, codec, ownership | None in this repository. LinearLite is a TS demo; docs describe a sibling Swift/app target. | Need separately owned Swift/app runner and real-cache fixture; never substitute the TS client/demo. |
| `BND-E2E-*` | Large-txn/backfill counter tests; `bench`, fleet, loadgen metrics sampling | Need fixed operation counts, exact admission/resource limits, SQL convergence after fence, fault repetitions and deterministic stop criteria. |

## Flake and isolation risks to account for

1. Vitest uses fork workers capped at four. Global setup creates one ephemeral PG, while each
   harness gets a distinct DB and globally unique slot (`pid` + timestamp + counter); this is a
   good baseline. It still has a shared fixed capacity of 80 slots/walsenders, random port selection
   with eight attempts, shared `/tmp` socket directory, shared Cargo target build and test-server
   process resources. Running multiple whole test commands in the same checkout/host increases
   collision/CPU pressure.
2. Many tests are event/poll based and sound, but a nontrivial set manufacture ordering with
   sleeps (25ms concurrent-create delay; 250ms long-poll parking; 300/500/2500/5000ms catalog
   windows; second-scale retention/reconciliation). Keep them in focused coverage for now; new
   acceptance cases must gate the actual request/lock/cut boundary and use deadlines only for
   diagnostics.
3. Timeouts are generous (global 60s; individual 90–300s; large transaction 180s). Four parallel
   engine/PG/DS stacks can make local/CI tests slow and expose resource flake. Do not blindly add
   an acceptance tier to root `pnpm test`; give it an exclusive job/namespace and artifact capture.
4. A test that uses current Postgres as oracle must stop unrelated writers or query the source
   journal only through its sentinel. Current per-harness isolation makes this adequate today, but
   future multi-client/gateway tests need an explicit source fence.
5. Docker has stable project name `electric-circuits`, fixed host ports by default and named
   `pg-data`/`ds-data`; it is unsafe for parallel acceptance jobs without a unique Compose project,
   port mapping and volume namespace. Its default DS memory mode also cannot validate restart
   durability.
6. `scripts/linearlite.sh` uses shared `/tmp` pid/log files and default ports; its stop path uses
   broad `pkill` patterns. It is a one-instance manual demo, not safe test infrastructure. The
   demo includes random port/fixture choices despite deterministic Faker seeding, and has no
   browser assertion harness.
7. `electric-conformance/run.sh` clones when absent and overwrites copied fixtures in a sibling
   Electric checkout; it must run in an isolated pinned checkout/worktree in CI, never against a
   developer's durable clone. It depends on external Mix/deps/network unless pre-provisioned.
8. Bench/loadgen are intentionally time-based and may write `results/`/report paths; fleet's
   external-target mode drops/recreates benchmark tables. They must be isolated and opt-in, not
   run against an unqualified or shared database.

## Exact command guidance for a TDD/testing skill

Use the narrowest regression command first, then expand. These are current commands, with their
preconditions stated rather than hidden.

```sh
# Fast local correctness/type gates
cargo fmt --check
pnpm typecheck
pnpm engine:test

# One high-level real-stack regression after a conformance-harness change
pnpm exec vitest run packages/conformance/src/conformance-<area>.test.ts

# The current repository-wide TS lane; PG server binaries initdb/pg_ctl must be on PATH.
# Set PREBUILT only after a successful engine build, to avoid parallel cargo builds.
ELECTRIC_CIRCUITS_ENGINE_PREBUILT=1 pnpm test

# Differential/fuzz: pin a seed before calling it reproducible.
SEED=90210 FUZZ_SEEDS=5 FUZZ_SHAPES=10 FUZZ_OPS=250 \
  pnpm exec vitest run packages/conformance/src/conformance-fuzz.test.ts
pnpm loop 50

# External Electric compatibility: isolate/pin ELECTRIC_DIR and provide Mix + PG binaries.
ELECTRIC_DIR=/absolute/throwaway/electric \
  ./electric-conformance/run.sh oracle

# Manual-only current surfaces (not acceptance gates)
pnpm docker:build
scripts/linearlite.sh start small
pnpm --filter @electric-circuits/loadgen loadgen
```

For a current engine change, `AGENTS.md`'s requested sequence is directionally correct—typecheck,
engine tests, full Vitest, then external Electric oracle and a demo browser smoke—but the skill
must report which external/browser steps actually ran and why. It must not claim those two ran just
because the internal suites were green. The baseline note currently records a deterministic full
Vitest retention failure and an unavailable Mix command; tests should be treated as live evidence,
not assumed-green documentation.

## Recommendations

### For the new TDD/testing skill

- Start every task with a **test-class decision**: pure Rust/TS unit, real-stack conformance,
  external Electric protocol, proposed acceptance, or manual demo. State the selected oracle,
  barrier and isolation scope before implementation.
- For current server behavior, prefer `bootHarness` + direct SQL + `drainEngine` +
  `waitForConvergence`/raw stream fold. Give every race a real lock, process, or HTTP-proxy gate;
  do not add a new `sleep` to make it pass.
- In every conformance test, create shape(s), write via real Postgres, run the drain barrier and
  compare normalized client state to SQL. For subqueries, the existing drain's pending-flip stage
  is mandatory. An empty client result before drain is not evidence.
- Seed generators and print/store the replay command plus the generated journal. A fuzz test that
  chooses an unrecorded random base is exploration, not release evidence.
- Use PGlite for fast semantic differential cases and the external Electric harness for protocol
  parity; label the source of truth accurately. Do not call the source-Postgres SQL comparison an
  independent database oracle.
- Add a separate acceptance runner only when it can own a unique PG18/DS volume/Compose namespace,
  source-commit journal, public client receipt, named `FaultGate`, and failure artifacts. It should
  consume the notes-24 scenario IDs and contract hashes, run serially/isolated, and never be
  smuggled into a unit test.
- Classify benchmark/loadgen/demo output as observation or smoke unless a test adds explicit
  oracle, fixed operation budget, caps and pass/fail assertions. Never use them as a substitute for
  an E2E regression.

### Suggested `AGENTS.md` wording corrections

Replace outcome-like wording with the following distinctions:

1. “`pnpm test` is the full TypeScript/conformance suite” is accurate, but add: “It boots a host
   PostgreSQL binary selected from PATH; it is not a PG18 or Docker-image acceptance lane.”
2. “Run Electric's own oracle suite” should say: “Required by this repository's human completion
   checklist when prerequisites exist, but not currently automated in CI; it requires a pinned,
   isolated Electric checkout and Mix/Elixir. Report it as blocked if unavailable.”
3. “Finish by driving the demo with Playwright MCP” should say: “Manual/external-browser smoke
   required for live UI/visualizer changes when browser tooling is available. The repository does
   not currently contain an automated browser test runner, so record the URL/actions/screenshot or
   the blocker.”
4. Add: “Docker compose defaults to PG16 and in-memory DS. It is a development stack, not proof of
   durable restart or the proposed PG18 production profile.”
5. Add: “Notes 18/23/24 define future stable acceptance scenarios. Until `packages/acceptance`,
   scenario registry, PG18 fixtures and gateway/app runners exist, map new tests to those IDs as
   prototypes and do not claim the scenario is qualified.”
6. Update the stated requirement “PostgreSQL 16 binaries on PATH” to version-neutral current
   reality or pin it: global setup just invokes `initdb`/`pg_ctl`; CI chooses the newest installed
   binary, while Docker/tutorials pin PG16. A PG18 contract cannot be inferred from any of those.

## Proposed first implementation slice (target state, not present)

1. Create an isolated `packages/acceptance` harness with explicit PG18 image/binary selection,
   `SHOW server_version_num` assertion, file-backed DS, unique namespace/volumes and process-log
   capture. Keep it outside root Vitest until it has an exclusive CI job.
2. Extract a reusable source-journal/sentinel and causal-fence wrapper around the existing
   `drainEngine` mechanics. Add a target TS client receipt first; make a deliberate delayed/faulted
   case prove that comparison refuses a pre-barrier state.
3. Port exactly one existing behavior as `PG18-E2E-001` and make it prove snapshot/live/restart at
   the new boundary. Then make `PG18-E2E-004` (virtual generated column rejection) red: notes 24
   identifies it as the immediate known PG18 blocker.
4. Promote the present HTTP DS proxy into named, awaitable fault gates and port `SRV-E2E-005` or
   `SRV-E2E-006`; preserve current focused tests as fast internal diagnostics.
5. Do not add gateway or Swift scenario labels to direct-engine tests. First create their actual
   public fixture/recorder and only then implement `GW-E2E-*` or notes-23 app scenarios.

