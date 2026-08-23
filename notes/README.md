# Electric Circuits production-readiness and Swift migration study

Status: **research and reviewed execution specification complete**

As-of date: 2026-08-23

The canonical deliverable is
[`18-production-readiness-spec-reviewed.md`](18-production-readiness-spec-reviewed.md). It defines 169
unique, acyclic, subagent-sized work packets, profile-specific gates, fixed-operation qualification
criteria, and an execution/merge protocol. The E2E/TDD maps define 69 stable high-level scenario IDs:
40 PostgreSQL/server/gateway/boundedness contracts and 29 Swift/package/app contracts. The initial synthesis
[`16-production-readiness-and-swift-migration-spec.md`](16-production-readiness-and-swift-migration-spec.md)
is retained as historical input and is superseded.

The independent hardening results and their task-by-task dispositions are in [`reviews/`](reviews/),
[`20-differential-review-disposition.md`](20-differential-review-disposition.md), and
[`25-pg18-e2e-differential-review-disposition.md`](25-pg18-e2e-differential-review-disposition.md).
Current command/test evidence is in [`17-validation-baseline.md`](17-validation-baseline.md).

## Main conclusions

- Public primary sources do not establish Circuits as the successor to current Electric. Current
  Electric is active; Circuits is a separate, young Electric-compatible project. Shipping therefore
  requires explicit long-term ownership of this fork.
- The fork contains substantial correctness and durability engineering, but is not production-ready
  under the defined gates: security/tenant boundary, durable-stream ownership/restore, boundedness,
  packaging, capacity and API/release stability remain open.
- The first server profile should be intentionally narrow: one PostgreSQL 18.x primary, one engine,
  one file-backed singleton durable-streams instance, proxy-only authenticated template gateway,
  verified TLS/SCRAM, offline backups, same-primary replacement, and full epoch reset after PG
  promotion. Virtual generated columns are rejected; stored generated columns must be published.
- Use a restricted `COMPAT_V1` app-provider lane for eligible eager/full/single-owner feeds. Build a
  separate `ElectricCircuitsSwift` for the long-term native surface. Do not make subsets, aggregates,
  transaction atomicity or a replica sink part of native core unless the app inventory selects them.
- Qualification must target the production app's materially customized vendored ElectricSync subtree,
  not assume sibling `../electric-sync-swift` tests represent it. Its current 200-message response
  chunk/checkpoint boundary receives explicit 199/200/201/400/maximum crash tests.
- Rollout advancement is based on fixed mutations, lifecycle/reset operations, per-template event
  floors and enumerated fault cuts—not elapsed-day monitoring.

## Execution packet inventory

| Workstream | Task prefixes | Packets |
| --- | --- | ---: |
| Planning, governance, product and release | `PLAN`, `GOV`, `NATIVE-ADR`, `RLS` | 8 |
| Security, protocol, gateway registry and admission | `SEC`, `PROTO`, `GWR`, `ADM` | 29 |
| Durable storage, engine and leadership | `DST`, `DSR`, `STO`, `ENG`, `LEAD` | 26 |
| PostgreSQL 18, deployment and operations | `PG18`, `PGR`, `OPS` | 22 |
| Capacity, test contracts and qualification | `CAP`, `TST`, `E2E` | 44 |
| Compatibility/native clients, app integration and migration | `CMP`, `SWF`, `APP`, `TSC`, `MIG` | 40 |
| **Total** |  | **169** |

Only `PLAN-001` is merge-ready in bootstrap wave 0. It materializes the authoritative manifest,
conditional profile closure, artifact ownership and scenario hashes; all later assignment is taken
from its generated ready set rather than from section order.

## Evidence and analysis notes

| Note | Scope |
| --- | --- |
| [`00-research-log.md`](00-research-log.md) | Research baseline and framing |
| [`01-upstream-project-status.md`](01-upstream-project-status.md) | Public upstream maturity, release and repository status |
| [`02-upstream-open-issues.md`](02-upstream-open-issues.md) | Public open-issue inventory and production impact |
| [`03-fork-upstream-delta.md`](03-fork-upstream-delta.md) | Local/parent/public fork graph and commit delta |
| [`04-fork-production-readiness.md`](04-fork-production-readiness.md) | Server correctness and remaining readiness gaps |
| [`05-operations-and-sre-readiness.md`](05-operations-and-sre-readiness.md) | Deployment, backup, recovery, observability and runbooks |
| [`06-circuits-wire-protocol.md`](06-circuits-wire-protocol.md) | Native and Electric-compatible wire behavior |
| [`07-electric-sync-swift-architecture.md`](07-electric-sync-swift-architecture.md) | Existing Swift package architecture and retained semantics |
| [`08-swift-compatibility-gap.md`](08-swift-compatibility-gap.md) | ElectricSync↔Circuits compatibility matrix |
| [`09-swift-library-strategy.md`](09-swift-library-strategy.md) | Adapt-versus-new-library decision |
| [`10-deprecation-and-successor-context.md`](10-deprecation-and-successor-context.md) | Primary-source Electric/Circuits project history |
| [`11-test-and-rollout-strategy.md`](11-test-and-rollout-strategy.md) | Test, shadow, cutover and rollback strategy |
| [`12-security-and-multitenancy.md`](12-security-and-multitenancy.md) | Threat model and gateway/tenant gaps |
| [`13-typescript-client-reference.md`](13-typescript-client-reference.md) | Existing TS client behavior usable as a reference |
| [`14-performance-and-capacity.md`](14-performance-and-capacity.md) | Boundedness, performance and capacity analysis |
| [`15-fork-open-issues.md`](15-fork-open-issues.md) | Parent-fork issue inventory and local applicability |
| [`16-production-readiness-and-swift-migration-spec.md`](16-production-readiness-and-swift-migration-spec.md) | Superseded first synthesis draft |
| [`17-validation-baseline.md`](17-validation-baseline.md) | Commands run, pass/fail/blocker results and cleanup |
| [`18-production-readiness-spec-reviewed.md`](18-production-readiness-spec-reviewed.md) | **Canonical reviewed production specification** |
| [`20-differential-review-disposition.md`](20-differential-review-disposition.md) | Review finding→task/exclusion traceability |
| [`21-postgres18-support.md`](21-postgres18-support.md) | PostgreSQL 18 compatibility audit, real-PG18 smoke and implementation blockers |
| [`22-server-e2e-tdd-map.md`](22-server-e2e-tdd-map.md) | Black-box server acceptance suites, deterministic harness and fault cuts |
| [`23-swift-app-e2e-tdd-map.md`](23-swift-app-e2e-tdd-map.md) | Swift/package/real-app scenario inventory and TDD order |
| [`24-postgres18-and-e2e-tdd-addendum.md`](24-postgres18-and-e2e-tdd-addendum.md) | **Integrated PG18 decision, stable E2E cases and new execution packets** |
| [`25-pg18-e2e-differential-review-disposition.md`](25-pg18-e2e-differential-review-disposition.md) | **PG18/E2E differential finding→task/exclusion traceability** |

## Independent differential reviews

| Review | Lens |
| --- | --- |
| [`01-server-correctness-differential.md`](reviews/01-server-correctness-differential.md) | Sequencing, fencing, catalog, lifecycle, subsets and transaction claims |
| [`02-security-gateway-differential.md`](reviews/02-security-gateway-differential.md) | Public boundary, tenant isolation, auth, transport, audit and mobile security |
| [`03-operations-durability-differential.md`](reviews/03-operations-durability-differential.md) | DS/PG restore, topology, purge, catalog boot, upgrades and disk |
| [`04-performance-boundedness-differential.md`](reviews/04-performance-boundedness-differential.md) | Queues, large transactions, retained state, capacity harness and statistics |
| [`05-swift-native-differential.md`](reviews/05-swift-native-differential.md) | Native Swift API, concurrency, persistence, backpressure, lifecycle and packaging |
| [`06-migration-compatibility-differential.md`](reviews/06-migration-compatibility-differential.md) | Real-app baseline, `/v1` eligibility, cache ownership, comparison and rollback |
| [`07-testing-release-dag-differential.md`](reviews/07-testing-release-dag-differential.md) | Dependency integrity, profiles, evidence gates and production rollout |
| [`08-red-team-omissions-differential.md`](reviews/08-red-team-omissions-differential.md) | Cross-domain omissions and unsafe assumptions |
| [`09-postgres18-differential.md`](reviews/09-postgres18-differential.md) | PG18 generated columns, slot continuity, publication/RLS, TLS and maintenance |
| [`10-e2e-tdd-differential.md`](reviews/10-e2e-tdd-differential.md) | Causal fences, transaction claims, public cuts, oracles and missing image scenarios |
| [`11-swift-app-e2e-differential.md`](reviews/11-swift-app-e2e-differential.md) | Real vendored app baseline, response checkpointing, native isolation and migration TDD |
| [`12-pg18-e2e-dag-redteam.md`](reviews/12-pg18-e2e-dag-redteam.md) | Task authority, profile closure, restore owners, qualification ordering and evidence integrity |

## Execution evidence convention

When implementation starts, every work packet writes `notes/execution/<task-id>.md` with starting
SHAs, owned scope, decisions, changed artifacts, commands/results and residual risk. `PLAN-001` first
materializes and validates the machine-readable task/profile graph; the Markdown is the human-readable
contract.
