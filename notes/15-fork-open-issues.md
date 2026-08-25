# Fork-tracker open-work inventory — 2026-08-22

This is an as-of-2026-08-22 snapshot of the two relevant GitHub namespaces,
checked against local `HEAD` [`474577a`](https://github.com/mwildehahn/electric-circuits/commit/474577a088b95c746bd9ab2c8e4b6552a72f151f).
It deliberately does **not** treat an open parent-tracker item as proof that
the local fork still lacks its fix.

## Namespace and source method

The checked-out remote is `mwildehahn/electric-circuits`. GitHub's repository
metadata says it is a fork of `pgxsinkit/electric-circuits`, which is itself a
fork of `electric-sql/electric-circuits`. Thus `mwildehahn#N` and
`pgxsinkit#N` are different issue namespaces; this note concerns the first two
only. In particular, the active tickets below are **parent-fork**
`pgxsinkit/electric-circuits` tickets, not tickets on the checked-out remote.

Primary-source pagination was read on 2026-08-22 using GitHub REST v3. The
issues endpoint includes pull requests, so the separate pulls endpoint was
also queried and deduplicated by number.

| Tracker | Open issues query, pages 1–2 | Open pulls query, pages 1–2 | Result |
| --- | --- | --- | --- |
| [`mwildehahn/electric-circuits`](https://github.com/mwildehahn/electric-circuits) | [p1](https://api.github.com/repos/mwildehahn/electric-circuits/issues?state=open&per_page=100&page=1), [p2](https://api.github.com/repos/mwildehahn/electric-circuits/issues?state=open&per_page=100&page=2) | [p1](https://api.github.com/repos/mwildehahn/electric-circuits/pulls?state=open&per_page=100&page=1), [p2](https://api.github.com/repos/mwildehahn/electric-circuits/pulls?state=open&per_page=100&page=2) | 0 issues; 0 PRs. |
| [`pgxsinkit/electric-circuits`](https://github.com/pgxsinkit/electric-circuits) | [p1](https://api.github.com/repos/pgxsinkit/electric-circuits/issues?state=open&per_page=100&page=1), [p2](https://api.github.com/repos/pgxsinkit/electric-circuits/issues?state=open&per_page=100&page=2) | [p1](https://api.github.com/repos/pgxsinkit/electric-circuits/pulls?state=open&per_page=100&page=1), [p2](https://api.github.com/repos/pgxsinkit/electric-circuits/pulls?state=open&per_page=100&page=2) | 10 issues plus 5 draft PRs; page 2 is empty for both lists. |

The repository-metadata sources are [the local fork](https://api.github.com/repos/mwildehahn/electric-circuits) and [its direct parent](https://api.github.com/repos/pgxsinkit/electric-circuits).
The parent's REST `open_issues_count` is 15 because it includes its five open
pull requests.

## Parent issues: status at local `474577a`

“Fixed” means the cited fixing commit is an ancestor of `474577a`, current
source still contains the behavior, and the named regression test exists.
“Partial” means an enabling or adjacent portion is present but the ticket's
requested outcome is still absent. “Leaves open” means the current source
still has the exact gap described by the issue.

| Parent issue | Local status and evidence | Classification | Dependencies / next action |
| --- | --- | --- | --- |
| [#4 — `subset({limit: 0})` never ends](https://github.com/pgxsinkit/electric-circuits/issues/4) | **Fixed.** [`6fdd9b0`](https://github.com/mwildehahn/electric-circuits/commit/6fdd9b0964ced62c8c7365fc6128e7b02f152296) says `Fixes ...#4`; `packages/client/src/subset.ts` now sets `ended = limit === 0 || rows.length < limit`, and `conformance-subset-positioning.test.ts` has the public limit-zero case. | **Obsolete** (close). | Supersedes the intentionally-red [PR #9](https://github.com/pgxsinkit/electric-circuits/pull/9); no outstanding implementation dependency. |
| [#5 — `subset()` cannot page through NULL sort keys](https://github.com/pgxsinkit/electric-circuits/issues/5) | **Fixed.** The same [`6fdd9b0`](https://github.com/mwildehahn/electric-circuits/commit/6fdd9b0964ced62c8c7365fc6128e7b02f152296) explicitly fixes #5. `subset.ts` now mirrors Postgres NULL ordering and builds `IS NULL`/`IS NOT NULL` cursor branches; conformance covers ascending and descending NULL pages. | **Obsolete** (close). | Supersedes intentionally-red [PR #10](https://github.com/pgxsinkit/electric-circuits/pull/10); no outstanding implementation dependency. |
| [#11 — reconciler does not discover new/re-created matching tables](https://github.com/pgxsinkit/electric-circuits/issues/11) | **Leaves open.** `Engine::reconcile_schemas` snapshots only `state.tables` and fingerprints only that set; `handle_dropped_table` explicitly says a re-created table is not picked up until restart. The deployment guide likewise still directs operators to restart after adding a table. | **Follow-up.** The documented restart rule avoids silent stale delivery, but dynamic table-selector users still need an interruption. | Re-resolve `ELECTRIC_CIRCUITS_PG_TABLES` on each tick or add `POST /tables/reload`; then run introspection, identity setup, schema registration and decoder/circuit input installation safely mid-stream. A conformance lane should cover add and drop/re-create. |
| [#12 — replayed TRUNCATE can retire shapes created after restart](https://github.com/pgxsinkit/electric-circuits/issues/12) | **Leaves open.** [`e19d42e`](https://github.com/mwildehahn/electric-circuits/commit/e19d42e64bb421a36b8903b2da8bb0db0fc5a75a) correctly introduced fail-closed TRUNCATE retirement, but `handle_truncate` still unconditionally calls `retire_dependents` for every replay. It does not compare a shape seed/snapshot gate with the triggering transaction. | **Follow-up** (rare, self-correcting availability/resync bug; not data loss). | Persist or otherwise recover a shape seed gate/position and fence TRUNCATE retirement by it; add crash-before-slot-ack coverage. Builds on the existing catalog and `SnapshotGate`, not a new external dependency. |
| [#13 — three drift/boot refusal paths lack E2E lanes](https://github.com/pgxsinkit/electric-circuits/issues/13) | **Leaves open as test debt.** The mechanisms exist: `circuit_needs_rebuild` has unit tests in `engine/drift.rs`, while `pg::inspect_publication` and `pg::check_wal_level` are called at boot. There is no conformance lane that respectively exercises counts-tier exit 75, a hand-made column-list publication exit 78, and a second non-logical-WAL cluster. | **Follow-up.** No evidence of a remaining production mechanism failure; evidence is incomplete. | Harness work: DBSP-counts drift/restart lane, manual publication setup, and an `initdb` cluster with `wal_level=replica`. |
| [#14 — walsender connect can hang for the kernel SYN timeout](https://github.com/pgxsinkit/electric-circuits/issues/14) | **Leaves open.** Ordinary Postgres connects use `pg::CONNECT_TIMEOUT` (10 s), but `replication.rs` still awaits `ReplicationClient::connect(cfg)` directly and the first `recv` is also unbounded. | **Follow-up** (availability/recovery hardening, not a correctness loss). | Wrap replication connect and first receive in `tokio::time::timeout`, probably reusing `pg::CONNECT_TIMEOUT`; test a blackholed endpoint and shutdown during it. |
| [#15 — DS URL validation and Prometheus-port behavior](https://github.com/pgxsinkit/electric-circuits/issues/15) | **Partially fixed.** Main now plainly logs that `ELECTRIC_PROMETHEUS_PORT` is not implemented and that `/metrics/prometheus` stays on the main port, so it is not silent. But `Config::resolve` merely copies `ELECTRIC_CIRCUITS_DS_URL` (unlike `parse_pg_url`), and there is neither a dedicated listener nor a boot refusal for the Prometheus port. | **Follow-up** (configuration ergonomics). | Parse/validate and redact the DS base URL during resolve; either implement the second listener or reject the setting. No cross-ticket dependency. |
| [#16 — no-SQLSTATE Postgres error may retry forever](https://github.com/pgxsinkit/electric-circuits/issues/16) | **Leaves open.** `pg::classify` still returns `Retryable` for no SQLSTATE, and `failure_name` acknowledges that a missing-password/config error can be in that bucket. [`56746ca`](https://github.com/mwildehahn/electric-circuits/commit/56746cad8db4b9f9b22d14c3ddc26d0dad98eec9) added the honest readiness/logging behavior but not a discriminator or bound. | **Follow-up** (operational misconfiguration can remain `waiting`; it does not serve wrong data). | Obtain a stable `tokio-postgres` error-kind discriminator, or bound repeated identical no-SQLSTATE failures and exit 78. |
| [#17 — published client does not recover from retired shape stream](https://github.com/pgxsinkit/electric-circuits/issues/17) | **Leaves open.** `packages/client/src/index.ts` opens `createStreamDB` once for `shape()`; it has renewal-time stream replacement but no handler for a reader's `stream-closed`/404/410. The engine half is present: retirement closes then deletes and DS distinguishes closed/gone. | **Conditional production blocker:** a deployed `@electric-circuits/client` shape can stop permanently after normal retention, drift, purge or epoch retirement. It is a follow-up only for the current pgxsinkit consumer, which does not use this package. | Depends on ADR-0007 signals already emitted by the engine. Add typed client transport classification, recreate with the same subscription, swap/reset the fold, and conformance-test all three terminal signals. |
| [#18 — compat `/v1/shape` returns 500 after deletion](https://github.com/pgxsinkit/electric-circuits/issues/18) | **Partially fixed, still open.** DS reads already create typed `StreamGone` for 404/410, and a closed stream produces 409 `must-refetch`. But `ApiError::from(anyhow::Error)` only special-cases `CreateRaced`; `StreamGone` falls through to 500, exactly as the issue reports. | **Follow-up** (compatibility surface outside ADR-0001's native-path scope). | Map `StreamGone` to the same 409 `must-refetch` response and add a stale-handle-after-delete adapter test. Depends only on the existing typed DS read error. |

## Parent draft pull requests

All five open parent PRs are explicitly “red test only — do not merge as-is.”
Their branches predate the local fixes. Their linked parent issues #1–#3 are
closed; #4–#5 remain open only because their tracker cleanup has not happened.
Each fixing commit below is an ancestor of `474577a`.

| Draft PR | What local HEAD proves | Classification / action |
| --- | --- | --- |
| [#6 — library-mode delete/update retraction red test](https://github.com/pgxsinkit/electric-circuits/pull/6) | [`207e67c`](https://github.com/mwildehahn/electric-circuits/commit/207e67cb2f9b76911cc5387c73554e3649a8b674) fixes parent #1 and adds `conformance-native-library-writes.test.ts`. | **Obsolete**; close as superseded rather than merge a stale red-test branch. |
| [#7 — bigint SUM precision red test](https://github.com/pgxsinkit/electric-circuits/pull/7) | [`d21dd37`](https://github.com/mwildehahn/electric-circuits/commit/d21dd372c1db52c8858af7be22ccff3a4c97d72e) fixes parent #2 with an exact `i128` accumulator and `conformance-native-aggregate.test.ts`. | **Obsolete**; close as superseded. |
| [#8 — subset initial offset red test](https://github.com/pgxsinkit/electric-circuits/pull/8) | [`6fdd9b0`](https://github.com/mwildehahn/electric-circuits/commit/6fdd9b0964ced62c8c7365fc6128e7b02f152296) fixes parent #3 and adds the offset-page conformance case. | **Obsolete**; close as superseded. |
| [#9 — subset limit-zero red test](https://github.com/pgxsinkit/electric-circuits/pull/9) | [`6fdd9b0`](https://github.com/mwildehahn/electric-circuits/commit/6fdd9b0964ced62c8c7365fc6128e7b02f152296) fixes #4 and the current conformance file contains its public-API regression. | **Obsolete**; close together with #4. |
| [#10 — subset NULL paging red test](https://github.com/pgxsinkit/electric-circuits/pull/10) | [`6fdd9b0`](https://github.com/mwildehahn/electric-circuits/commit/6fdd9b0964ced62c8c7365fc6128e7b02f152296) fixes #5 and the current conformance file covers both sort directions. | **Obsolete**; close together with #5. |

## Verification boundary

This was a source-and-history audit, not a claim that the entire engine suite
was rerun. The three targeted PR-regression files were selected for a direct
Vitest run, but this checkout has no installed `vitest` binary
(`ERR_PNPM_RECURSIVE_EXEC_FIRST_FAIL: Command "vitest" not found`), so no test
result is represented as green here. The implementation and test-file evidence
above is therefore supplemented by the fixing commits and ancestry, not by a
fresh local execution.
