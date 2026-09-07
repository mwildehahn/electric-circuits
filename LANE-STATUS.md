# LANE STATUS

Lane: `circuits-engine-sources-run1`

Branch: `feat/sources-table-mode`

Status: addressing independent xhigh review of PR #20 (FIX FIRST). Implementation of the review findings is complete locally; pushing and polling CI.

Done:

- Source-scoped `/_admin/*` now 404s; host `/admin/refresh` remains the only admin route.
- Explicit refresh retries failed desired rows at the same revision; healthy unchanged rows stay no-ops.
- `source_id` must be a single safe path component before storage dirs are built.
- Poll task is owned and joined; shutdown short-circuits control I/O, reconcile, and new starts.
- Resolver errors are classified (`resolve failed: <prefix> <class>`) with no name tail, URL, or underlying string.
- Focused tests cover those paths, concurrent refresh serialization, and the ignored lifecycle test now fails closed if `ELECTRIC_CIRCUITS_TEST_PG_URL` is missing.

Verification:

- `cargo fmt --all -- --check`
- `cargo test -p electric-circuits-engine --features test-support -j 2 -- --test-threads=2`

PR: https://github.com/mwildehahn/electric-circuits/pull/20

In progress: push, PR comment per finding, poll CI until green.
