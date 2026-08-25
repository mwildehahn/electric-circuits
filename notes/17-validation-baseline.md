# Validation baseline

As of 2026-08-22 (America/Los_Angeles), against local Circuits `474577a` and
`../electric-sync-swift` `0.1.12`. This records fresh execution evidence for the research/spec turn;
no production implementation was changed.

## Results

| Check | Result | Evidence / limitation |
| --- | --- | --- |
| `cargo fmt --check` | **Pass** | Completed with exit 0. |
| `pnpm typecheck` | **Pass** | The first attempt could not start because workspace dependencies were absent (`tsc: command not found`). `pnpm install --frozen-lockfile` completed, then typecheck exited 0. The lockfile was not changed. |
| `pnpm engine:test` | **Pass** | 426 Rust tests listed/executed across unit and integration targets; zero failures. |
| `ELECTRIC_CIRCUITS_ENGINE_PREBUILT=1 pnpm test` | **Fail: 284/285 passed** | 53/54 files passed. `conformance-retention.test.ts` expected the backing stream to be 404 immediately after `DELETE /shapes/{id}?purge=true`; the engine record was 404 but the stream returned 200. |
| Focused retention rerun | **Same deterministic failure** | `ELECTRIC_CIRCUITS_ENGINE_PREBUILT=1 pnpm exec vitest run packages/conformance/src/conformance-retention.test.ts --reporter=verbose` passed 6/7 and repeated the exact force-purge assertion failure at line 188. |
| Electric external oracle | **Not runnable in current environment** | `ASDF_ELIXIR_VERSION=1.18.4-otp-28 ASDF_ERLANG_VERSION=28.1 ./electric-conformance/run.sh oracle` built the release engine, then exited 127 at `mix deps.get`: `mix: command not found`. The harness copied five fixtures into `../electric` before failing; only those known copies were restored/deleted, and the sibling checkout is clean again. |
| Swift dependency boundary | **Pass** | `../electric-sync-swift/Scripts/check-dependency-boundaries.sh` passed. |
| `swift test` | **Pass** | 351 tests in 23 suites passed with zero failures. |

## Force-purge contract mismatch

This is not treated as a flaky test:

- `apps/engine/src/engine/lifecycle.rs::purge_shape_inner` deliberately removes the engine record,
  waits for durable `Dropped`, then spawns `finish_purge`; its documentation says the stream may still
  be deleting when the HTTP call returns.
- `packages/conformance/src/conformance-retention.test.ts` and the HTTP handler documentation describe
  `purge=true` as immediate/full teardown and assert a 404 stream immediately after the response.

The implementation, endpoint promise, and test must choose one contract. A production API must not
answer ambiguously: either wait for `Retired`/confirmed delete before returning success, or return an
accepted/pending result and provide a durable completion/status contract. The reviewed specification
chooses terminal success only after retirement completion in `ENG-014`.

## What this baseline does not establish

- No security penetration, production TLS/gateway, backup/restore, failover, capacity, or migration
  workload was run.
- No Electric external conformance result is claimed because Elixir/Mix is absent.
- A green Rust/Swift baseline does not override the deterministic Vitest failure or any static
  production-readiness finding.
