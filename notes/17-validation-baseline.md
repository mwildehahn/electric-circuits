# Validation baseline

As of 2026-08-25 (America/Los_Angeles), the current integrated recovery candidate before this
evidence-note commit is `f977f10`. It contains the merged durable-stream provider, PostgreSQL 18
launcher baseline, native Axum `/v1` hardening, and the LinearLite task seeder/client UUID changes.
This is regression evidence for that candidate; it is not production-release qualification.

## Results

| Check | Result | Evidence / limitation |
| --- | --- | --- |
| `cargo fmt --check` | **Pass** | Completed with exit 0 on the integrated candidate. |
| `pnpm typecheck` | **Pass** | `tsc --noEmit -p tsconfig.json` completed with exit 0. |
| `pnpm test:node` | **Pass: 8/8** | Includes the static PostgreSQL image/launcher contract: Compose and tutorials use `postgres:18.6`, and ephemeral launchers resolve PostgreSQL 18 tools explicitly. |
| `pnpm engine:test` | **Pass** | 393 library unit tests plus 69 runnable integration tests passed (462 total); 3 real-Postgres RLS tests remained explicitly ignored because `ELECTRIC_CIRCUITS_TEST_PG_URL` was unset. |
| `ELECTRIC_CIRCUITS_ENGINE_PREBUILT=1 pnpm test` | **Pass: 63 files / 304 tests** | Full Vitest/conformance run completed in 873.90 seconds. It includes PG18 promotion, schema drift, WAL recovery, pgxsinkit provider contract, native subscriptions, negative controls, fuzz/oracle comparisons, and the LinearLite seeder tests. |
| Native API focused review gates | **Pass** | Rust HTTP endpoint tests passed 21/21; API REST/server tests passed 2 files / 6 tests. Independent Sol-high review passed the typed 400 mapping for unknown table/output columns and the canonical-Axum-vs-gateway dispatch contract. |
| Testing skill validation | **Pass** | `uv run --with pyyaml python /Users/bozilabs/.codex/skills/.system/skill-creator/scripts/quick_validate.py .agents/skills/electric-circuits-testing` completed successfully. |
| Electric external oracle | **Not rerun on this exact candidate** | The full repository suite includes the in-repo oracle comparisons, but the separate `electric-conformance/run.sh oracle` lane was not rerun. No exact-candidate external-oracle claim is made. |

## Resolved prior baseline failure

The 2026-08-22 baseline recorded a deterministic force-purge mismatch: the engine record was gone
when `DELETE /shapes/{id}?purge=true` returned, but the backing durable stream could still exist. The
current integrated history includes the terminal-retirement completion fix, and the full retention
and retirement-completion conformance suites now pass, including immediate purge visibility,
long-poll release, crash recovery, and non-reuse of a still-retiring shape ID.

## What this baseline does not establish

- It does not substitute for the generated production-readiness profile closure or immutable
  qualification evidence required by `notes/18-production-readiness-spec-reviewed.md`.
- No production gateway/authentication, TLS, backup/restore, capacity, or multi-node failover profile
  is claimed by this run.
- The native API's payload-limit `413` response is still a documented non-blocking hardening item: an
  oversized Axum body currently uses the framework's plain-text envelope rather than the native JSON
  error envelope.
- The Indexed Today/Calendar iOS prototype is being qualified separately in its own repository and
  worktree; this server baseline is not evidence that the app integration is complete.
