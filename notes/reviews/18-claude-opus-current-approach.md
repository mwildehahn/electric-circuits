# Claude Opus review: current approach and open issues

Date: 2026-08-23  
Model: Claude Opus via `claude -p --model opus`  
Authentication: Claude device authentication (`claude auth status`: `loggedIn: true`, `authMethod: claude.ai`, `apiProvider: firstParty`)  
Mode: read-only plan; no repository edits; no `--max-turns` or `--max-budget-usd` flags  
Reviewed HEAD at invocation: `56ff1184693b3215a0e73a1523c807eee0f6b74f`

## Report

### Worker workflow

The scheduler cannot currently issue a packet. `scripts/readiness-plan.ts:411` compares checked-in controller state against the live git HEAD, while `scripts/readiness-plan.test.ts` hardcodes `3f452e4…`/`520751e…`. The test result reported by Opus was 6 pass / 4 fail, including `stale_controller_state` and `invalid_red_artifact`; `npx tsx scripts/readiness-plan.ts ready` returned `controller_state_required`.

No controller exists. `validateLease` checks TTL/heartbeat/generation predicates, but nothing issues, renews, or revokes leases. `request_mailbox`/`phase_file`/`stop_file` appear as required key names and test fixtures, not as a live controller; no `refs/leases/**` were found.

The reported ledger count was 4 of 169 tasks integrated over 27 attempts (the ledger predates the current implementation wave). Opus characterized most failed attempts as infrastructure/protocol failures rather than engineering defects: missed heartbeats to a nonexistent server, lease expiry during review, and isolated-worktree `pnpm typecheck` failures caused by absent linked workspace packages. It also reported incomplete handoff/resolution artifacts and many evidence worktrees under `/private/tmp`.

The 507-row gate matrix is largely cosmetic: `readiness-plan.ts:500-503` maps task prefixes to a few whole-repository commands plus one real test invocation.

### Production readiness

Opus re-ran the retention conformance test and reported 7/7 passing after the ENG-014 purge fix, while noting that the frozen validation baseline still records that test as failing.

It judged the current wave to have insufficient admissible ledger evidence: ENG-003/006/014/015 dependencies and red-artifact requirements are not represented in the checked-in readiness ledger, `scenario_ids` are empty on all 169 tasks, and recent commits lack task IDs/packet hashes. It specifically called out that the merged ENG-014 baseline repair lacks a retroactive packet and independent-review record in the readiness ledger. This is a provenance/qualification issue, not evidence that the code change is incorrect.

It also reported that `pnpm test` collects `scripts/*.test.ts` files using `node:test`, producing three suite failures with zero assertions. CI reportedly triggers only on `main`, while the current branch is ahead and has not been CI-validated.

### Missing E2E/TDD cases

1. The startup-preflight change (`8b63db5`) lacks a wiring-level busy-slot boot test; the existing assertion can pass with the ordering reverted.
2. The publication/RLS tests (`4b66aad`) are all `#[ignore]`, and the normal cargo lane does not run them with a real Postgres URL.
3. The replication setup change (`1e8e81f`) tests timeout helpers but not `client.abort()` or first-frame reinjection; a regression could drop the first replication message.
4. No conformance-level test was added by the recent commits; passing tests are mostly in-crate fakes or pure functions.
5. The acceptance tier described by the readiness plan is absent: no `e2e-scenarios.json`, scenario IDs, source commit markers, or `drainedThrough` receipts.
6. The PG18 virtual-generated-column case (`PG18-E2E-004`) remains an unverified P0 hypothesis: snapshot value versus live `null` behavior.

### Postgres 18

Compose is pinned to Postgres 18.6 and the volume path is correct, but Opus found that the test path does not use compose: CI/global setup uses the highest host major and `initdb` from `PATH`. There is no explicit `SHOW server_version` qualification check.

### Five bounded next tasks proposed by Opus

1. Make PLAN-001's gate hermetic by injecting `checkoutIdentity`, checking in controller state, and adding `readiness:*` scripts. Treat this as a proper PLAN-001 packet.
2. Make declared gates execute: split `scripts/**` out of vitest into `test:scripts`; add it and `readiness:validate` to CI; trigger CI for `readiness/**`; add an ignored Rust lane with `ELECTRIC_CIRCUITS_TEST_PG_URL` for RLS tests.
3. Regenerate the TST-000 baseline at current HEAD after #2, recording ENG-014 as cleared and any remaining test-runner failure as a new blocker.
4. Add the three wiring/conformance gaps: busy-slot boot, first-frame reinjection, and `finish_create` rollback after a closed flip channel.
5. Pin runtime Postgres to 18.x, then retry PG18-000, beginning with a retained-digest reproduction of `PG18-E2E-004`.

## Interpretation for our implementation workflow

- Keep the worker/reviewer/DONE/REVIEWED protocol for code changes; it is useful for scoped TDD and independent review.
- Separate implementation correctness from qualification evidence. A PASS in an isolated worktree does not by itself satisfy the readiness ledger's provenance or acceptance-gate requirements.
- Do not reopen already integrated fixes merely because their historical packets are absent. Instead, add explicit retroactive provenance or mark them `non_protocol_provenance`, and add high-level tests where the behavior is still unproven.
- The highest-value immediate work is to repair the declared test/readiness gates and add wiring-level E2E tests. This is concrete infrastructure work, not a request to wait for production observation.
- ENG-004 remains intentionally scoped to replayed-TRUNCATE circuit fencing; per-dependent seed persistence and subquery restore semantics are still open.

## Caveats

This is an external read-only critique generated from the repository state at the invocation HEAD. Its ledger counts and commit references may lag the current branch because the readiness artifacts are historical. Each finding should be rechecked against the current files and current test commands before becoming a new implementation packet.
