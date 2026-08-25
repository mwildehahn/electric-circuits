# Differential review disposition

Date: updated 2026-08-23. Canonical target:
[`18-production-readiness-spec-reviewed.md`](18-production-readiness-spec-reviewed.md).

Eight independent GPT-5.6-sol/high agents reviewed the first synthesis draft from different failure
domains. Every P0/P1 finding was either assigned to a concrete task in the canonical spec or turned
into an explicit unsupported release capability. This document records that disposition; it is not a
second source of requirements.

Four later PostgreSQL 18/E2E/TDD reviews and the resulting task splits are disposed separately in
[`25-pg18-e2e-differential-review-disposition.md`](25-pg18-e2e-differential-review-disposition.md).

## Review-by-review disposition

| Review | Principal findings accepted | Canonical disposition |
| --- | --- | --- |
| [Server correctness](reviews/01-server-correctness-differential.md) | Four cycles; DS-behind-slot restore gap; unbounded catalog; impossible universal transaction marker; purge contract mismatch; transaction-sized sequencer state; subset fence inputs; retained derived state; lifecycle ownership | `PLAN-001`, `DSR-001`–`DSR-003`, `STO-001`, `ENG-014`, `ENG-007A`, `ENG-001`/`ENG-001A`, `ENG-009A`, `SEC-002B`; txn atomicity moved to optional `PROTO-003B`/`ENG-002` profile. |
| [Security/gateway](reviews/02-security-gateway-differential.md) | Public raw query contract contradicted tenant policy; proxy/direct-read fork; no durable principal/claim/revocation boundary; unsplit listeners/operator auth/TLS/secrets; missing HTTP/audit/privacy/supply-chain/mobile protection | Public template/result contract in `SEC-000`/`PROTO-001A`; private contract in `PROTO-001B`; proxy-only `SEC-004`; registry/revocation `SEC-002A`–`SEC-002C`; policy `SEC-003`; split service/operator/transport/security tasks `SEC-005A`–`SEC-010`, `SEC-008A`/`SEC-008B`, `SWF-012`. |
| [Operations/durability](reviews/03-operations-durability-differential.md) | Unsafe restore, false advisory-lock fencing, async purge acknowledgement, privileged runtime provisioning, partial/empty catalog boot, generic-HA DS packaging, weak config/upgrade/telemetry/artifact/disk contracts | Narrow singleton topology `LEAD-001`; restore manifest/frontier/reset `DSR-*`; purge `ENG-014`; PG bootstrap/preflight `OPS-003A`/`OPS-003B`; fail-closed catalog `ENG-015`; owned DS `DST-001`; strict config `ENG-012`; upgrade/operations/artifacts `OPS-002`–`OPS-007`, `RLS-001`; reserve accounting `ENG-010`. |
| [Performance/boundedness](reviews/04-performance-boundedness-differential.md) | End-to-end large transaction unbounded; disk acceptance impossible without accounting/compaction/reserve; live retained state growth; create starvation; missing queues and HTTP limits; unusable load harness/statistics; real-calendar correctness waits | Resource schema/admission `CAP-001A`/`ADM-001`; spill journal `ENG-007A`; queues/admission/materialization/derived state/disk `ENG-007`–`ENG-010`; HTTP resources `ENG-006A`; DS accounting `DST-001`; compaction `STO-001`; open-loop driver and capacity discovery `CAP-000`–`CAP-005`; fixed operations and virtual clocks in completion rules. |
| [Swift native](reviews/05-swift-native-differential.md) | Raw query API bypassed auth; delivery acknowledgement/persistence owner missing; ambiguous lifecycle; dropping stream buffer; incorrect subset identity; scalar inference; actor reentrancy; sink ownership; mobile lifecycle/protection; insufficient Apple/package tests | Template-driven Swift API and delivery ADR `SWF-000`; independent package `SWF-001`; schema codec `SWF-002`; fixture/URLSession transports `SWF-003A`/`SWF-003B`; actor/lossless pull stream `SWF-004`/`SWF-005A`; optional spillable txn `SWF-005B`; fold/sink/subset/lifecycle/security/release `SWF-006`–`SWF-013`; profile-specific `TST-002N`, `TST-006N`, `TST-008N`. |
| [Migration/compatibility](reviews/06-migration-compatibility-differential.md) | Wrong package/app baseline; `/v1` eligibility too broad; provider/cache ownership unresolved; no comparison fence; unsafe normalization/rollback; lanes coupled; codec missing | Freeze real app `CMP-000`; exhaustive inventory `CMP-001`; ownership `APP-OWN-001`; narrow eligibility `CMP-002`; codec `CMP-002B`; gateway/provider/cache/host `CMP-003`–`CMP-006`; common PG fence/comparator/fresh rollback `MIG-000`–`MIG-002B`; compatibility and native release profiles remain independent. |
| [Testing/release DAG](reviews/07-testing-release-dag-differential.md) | Cyclic/invalid waves; missing purge task; gates ignored exclusions; no owned production rollout; inconsistent count policy; missing harnesses; blocked suites could appear complete; packets too broad | Validator-generated graph `PLAN-001`; profile compiler `GOV-005`; purge `ENG-014`; pass/fail/blocked/N-A gate semantics; immutable evidence `RLS-001`; deterministic/cross-language/security/fault/device tests `TST-000`–`TST-012`; fixed-count rollout `MIG-004`–`MIG-009`; integration packets explicitly labelled and separately reviewed. |
| [Red-team omissions](reviews/08-red-team-omissions-differential.md) | Publication adoption can silently stale; restore and PG16 fencing unsound; deferred outputs break universal txn finality; lifecycle capabilities unbound; large transactions unbounded; DS unowned; TLS/least privilege incomplete; checkpoint identity required before optional sink; caching/CDN unowned | Publication completeness `ENG-017`/`SEC-003B`; restore/topology tasks above; txn atomicity optional and tier-qualified; durable registry `SEC-002B`; journal `ENG-007A`; DS ownership `DST-001`; TLS/PG jobs `SEC-006A`, `OPS-003A`/`OPS-003B`; core checkpoint contract `SWF-000`; direct capabilities and edge caching explicitly unsupported in release profiles. |

## Cross-review changes to the first draft

1. **One release became multiple profiles.** `COMPAT_V1` and `NATIVE_CORE` can ship independently;
   aggregate, subset, transaction atomicity and replica sink are optional closures. Disabled features
   fail at configuration/protocol admission.
2. **The public API became template-driven.** Clients cannot choose tables, predicates, projections,
   shape IDs, claims or DS paths. A durable gateway registry owns the principal→feed→internal-claim
   binding, and all launch reads are proxied.
3. **The first topology became intentionally non-HA.** One PG18 primary, one engine and one file-backed
   singleton DS are supported. Same-primary replacement requires confirmed termination; PG promotion
   forces epoch reset/rehydration. Advisory locks are not advertised as stale-writer fencing.
4. **Restore became a pre-readiness proof.** Quiesced DS backups carry a complete frontier manifest;
   DS behind an advanced slot cannot resume in the same epoch. Catalog application is transactional
   and fail-closed.
5. **Boundedness covers committed work and long-lived state.** A spillable complete-transaction/output
   journal replaces transaction rejection; queue, request, snapshot, derived-state, catalog, active
   stream, disk and control-reserve tasks now have separate owners.
6. **Source-transaction atomicity is no longer a core claim.** Event-level delivery is the native
   baseline. Transaction finality is negotiated only for tiers whose deferred output coordinator has
   passed `ENG-002`.
7. **Swift persistence semantics precede implementation.** Core subscriptions always have explicit
   checkpoint/replay semantics; a transactional replica sink is optional. Backpressure is a bounded
   suspending pull stream, not a dropping continuation buffer.
8. **Migration comparison is causally fenced.** Both paths must consume a common PostgreSQL sentinel;
   opaque offsets are never compared and tag/ownership mismatches are not normalized away. Rollback
   cache is exposed only when its fence is current or after cold rebootstrap.
9. **Qualification uses counts, not calendar waiting.** Workloads record offered/admitted/committed/
   applied/rejected counts, per-template floors, virtual-time boundaries, raw evidence and 100 runs per
   enumerated fault cut. Rollout stages have concrete mutation/session/lifecycle/reset floors.

## Alternatives deliberately excluded from the first release

- arbitrary public predicates/tables/projections;
- public raw engine, tRPC, admin, durable-stream or Postgres access;
- direct signed DS capabilities, CDN or edge-cache semantics;
- seamless PG18 failover, multi-engine active/active, or generic DS HA;
- online volume-copy backups;
- universal or cross-stream source-transaction atomicity;
- hot compiled-circuit changes;
- compatibility for ElectricSync DNF/tag ownership, on-demand, progressive, order/limit/subset or SSE;
- a dual-backend refactor of ElectricSync; and
- optional native modules without a production inventory consumer.

## Spec-integrity check

The current canonical Markdown contains 169 unique task IDs after the PG18/E2E follow-up. A dependency extraction over every
`Depends` block reports zero duplicate IDs, zero unknown dependencies, zero direct or transitive
cycles, and 39 literal topological fronts. `PLAN-001` still
requires the implementation team to check in the authoritative machine-readable graph, conditional
profile dependencies and invalid-graph mutation fixtures before implementation scheduling begins.
