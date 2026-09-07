# LANE STATUS

Lane: `circuits-engine-sources-run1`

Branch: `feat/sources-table-mode`

Status: implementation complete; draft PR is open and CI/review polling is in progress.

Done:

- Added opt-in table/file sources discovery, secret resolution, idempotent reconciliation, per-source workers/storage, source-scoped HTTP forwarding, status routes, refresh, and polling.
- Added configuration, reconciliation, resolver, HTTP, and real-PostgreSQL lifecycle tests.
- Added sources-table documentation and the ignored CI lifecycle gate.

Verification:

- `cargo fmt --all -- --check`
- `cargo test -p electric-circuits-engine --features test-support -j 2 -- --test-threads=2`
- `pnpm typecheck`
- `pnpm test` (69 files, 311 tests)
- `sources_table` ignored lifecycle test is configured for CI but was not run locally because `ELECTRIC_CIRCUITS_TEST_PG_URL` is not set in this environment.

PR: https://github.com/mwildehahn/electric-circuits/pull/20

In progress: monitor CI and reviewer feedback.
