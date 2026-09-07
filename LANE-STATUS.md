# LANE STATUS

Lane: `circuits-engine-sources-run1`

Branch: `feat/sources-table-mode`

Status: review findings fixed; PR checks are green.

Done:

- Source-scoped `/_admin/*` 404s; host `/admin/refresh` is the only admin route in sources-table mode.
- Explicit refresh retries failed desired rows at the same revision; healthy unchanged rows stay no-ops.
- `source_id` must be a single safe path component before storage dirs are built.
- Poll task is owned and joined; shutdown short-circuits control I/O, reconcile, and new starts.
- Resolver errors are classified (`resolve failed: <prefix> <class>`) with no name tail, URL, or underlying string.
- Focused tests cover those paths, concurrent refresh serialization, and the ignored lifecycle test fails closed if `ELECTRIC_CIRCUITS_TEST_PG_URL` is missing.

Verification:

- `cargo fmt --all -- --check`
- `cargo test -p electric-circuits-engine --features test-support -j 2 -- --test-threads=2`
- PR CI green, including image jobs and the test job (sources-table ignored lifecycle included).

Final implementation commit: `5ee56482db983df64121e893bf7d281da6993b39`

PR: https://github.com/mwildehahn/electric-circuits/pull/20

In progress: none.
