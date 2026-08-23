# Research log

As of 2026-08-23.

## Baseline established

- The local Electric Circuits checkout is clean on `main` at `474577a`, tracking the `mwildehahn/electric-circuits` fork.
- The fork has a concentrated stabilization series after the earlier upstream-derived work: schema-qualified identity, schema-drift retirement, slot epochs, segmented change-log retention, bounded transaction ingest, streamed backfills, graceful shutdown, durable catalog acknowledgement, retirement completion, and identified/idempotent/leased subscriptions.
- The repository already contains an upstream issue triage at `docs/notes/2026-08-21-upstream-issue-triage.md`. That note is useful historical evidence but cannot substitute for a fresh GitHub issue inventory because fork fixes landed after its stated comparison point.
- `../electric-sync-swift` is clean on `main` at tag `0.1.12`. It targets Swift 6.1, iOS 18, and macOS 15. Its library target is dependency-free; GRDB is test-only.
- The Swift package is not a thin Shape HTTP decoder. It contains persistent cursor/ownership semantics, shape replica modes, subset/DNF compilation, move-out handling, recovery/admission logic, transport abstraction, retries/circuit breaking, session/runtime injection, and extensive regression tests.

## Early framing

1. The fastest compatibility experiment is likely the Electric-compatible `/v1/shape` surface, because it minimizes server and Swift protocol changes.
2. The likely long-term design is different: a Circuits-native client can use identified subscription leases and a simpler predicate AST, while subset pagination and aggregates should remain distinct capabilities rather than being forced through Electric Shape semantics.
3. “Electric Circuits is production ready” and “a Swift client can consume it” are separate gates. The former includes server durability, recovery, security, operations, capacity, and API stability; the latter adds mobile lifecycle, resumability, local-store atomicity, transport cancellation, and release compatibility.
4. The adapt-versus-new-library decision should be based on retained behavior and migration risk, not file count alone. Reusing provider/storage abstractions may be valuable even if Electric-specific DNF and cursor compatibility code is removed.

## Questions to resolve

- Is Electric Circuits officially designated as Electric’s successor, or is that a local/fork roadmap assumption?
- Which open upstream issues remain true for this fork after the stabilization commits?
- Is the native durable-stream endpoint intended as a stable public/mobile boundary, or must a control-plane gateway own authentication and stream proxying?
- What is the exact resume contract for native streams after retirement, lease lapse, schema drift, deployment, and epoch reset?
- Does the application need only live shapes, or also Circuits-native subsets and aggregates in its first migration milestone?
- Which `electric-sync-swift` public APIs are already depended on by the app and therefore require a compatibility facade or deprecation window?

## Resolution

The research and twelve differential reviews are complete. The reviewed execution contract is
[`18-production-readiness-spec-reviewed.md`](18-production-readiness-spec-reviewed.md); review
disposition is [`20-differential-review-disposition.md`](20-differential-review-disposition.md) plus
the PG18/E2E follow-up
[`25-pg18-e2e-differential-review-disposition.md`](25-pg18-e2e-differential-review-disposition.md).

- Primary sources do not identify Circuits as the successor to current Electric; fork ownership is a
  launch decision, not an upstream dependency.
- Native DS access is not accepted as a public/mobile boundary. The first release uses an authenticated
  template gateway with durable principal/feed ownership and proxied reads.
- Resume after retirement, drift, lease lapse or epoch loss is a typed generation reset/full rehydrate
  unless the same-generation continuation proof succeeds.
- The native Swift core is materialized shapes with explicit delivery/checkpoint semantics. Aggregate,
  subset, transaction atomicity and replica sink are separately selected capabilities.
- The inspected production-app candidate contains a materially customized vendored ElectricSync
  subtree; sibling `../electric-sync-swift` tests are separate evidence unless provenance is proved.
  `CMP-000` freezes the exact production revision before compatibility claims.
- PostgreSQL 18 is the sole first-production DB profile. A real PG18.6 engine/DS smoke passed ordinary
  and stored-generated snapshot/live behavior but exposed a P0 virtual-generated-column divergence:
  backfill computed the value while the live event emitted `null`.
- The spec contains 169 unique task IDs, 36 exact conditional-edge rows, and no literal or
  profile-expanded dependency cycles. The TDD maps define 69 stable black-box scenarios. Advancement
  uses fixed operation/fault-cut budgets rather than calendar monitoring.
