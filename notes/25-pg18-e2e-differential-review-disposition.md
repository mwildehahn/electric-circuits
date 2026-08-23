# PostgreSQL 18 and E2E/TDD differential-review disposition

Date: 2026-08-23. Canonical target:
[`18-production-readiness-spec-reviewed.md`](18-production-readiness-spec-reviewed.md).

Four independent GPT-5.6-sol/high reviews hardened the PostgreSQL 18 and black-box E2E/TDD
extension. This note records the disposition of every P0/P1 finding. It is traceability, not a second
task registry; task definitions and dependencies exist only in the canonical specification and the
machine manifest produced by `PLAN-001`.

## Result

All P0/P1 findings were accepted as concrete spec corrections or explicit first-profile exclusions.
No review asked us to abandon PostgreSQL 18. The canonical document now contains **169 unique task
IDs**, zero duplicate IDs, zero unknown dependencies, and zero dependency cycles. Literal dependency
sorting starts with `PLAN-001` alone, followed by `GOV-001` and `TST-000` after the validator exists.

The most consequential corrections are:

- promotion/timeline changes reset the epoch even with a synchronized same-name failover slot;
- slot continuity is proved against the durable landed source frontier, never by slot name/plugin;
- publication safety is bootstrap-owned and fenced, not eventually inferred by polling;
- tracked-table RLS is rejected and replication is configured to fail instead of silently filter;
- one publishable-column schema feeds every snapshot/live/restore/drift path;
- every PostgreSQL connector receives an independent verified-TLS test;
- client causality ends at the actual target fold/cache commit, not an unrelated sentinel feed;
- core resume/checkpoint semantics are event/response-level; source-transaction observer atomicity is
  optional and per eligible stream only;
- the real vendored ElectricSync subtree and its 200-message response/checkpoint boundary are tested;
- native core has no GRDB/app/migration dependency; app materialization is a separate selected task;
- migration public acceptance is red before implementation and green before rehearsal;
- gateway-registry and PostgreSQL backup/restore stores now have explicit owners; and
- qualification is finite, hash-bound, profile-generated, and split between candidate artifacts and
  final release attestation.

## Review 09: PostgreSQL 18 differential

| Finding | Disposition in the canonical spec |
| --- | --- |
| P0-1 promotion test could pass on missing slot without testing timeline policy | Accepted. `PG18-E2E-009` covers a missing/unsynchronized slot and `PG18-E2E-010` covers a synchronized usable same-name slot; both reset. `PG18-002C`, `OPS-004`, `LEAD-001`, and `PG18-003Q` own implementation and qualification. Seamless failover remains a future profile. |
| P0-2 same-name slot recreation can skip source commits | Accepted. `PG18-002C` defines a continuity decision from durable landed source frontier plus slot database/type/temporary/failover/two-phase/restart/confirmed/invalidation properties. `PG18-E2E-011` covers ahead/equal/behind recreation; same name/plugin is never sufficient. |
| P0-3 live publication mutation cannot be made safe by later polling | Accepted. `ENG-017`, `OPS-003A`, and `OPS-003B` make the publication bootstrap-owned and immutable while ready. The sanctioned workflow fences readiness before DDL/DML and fingerprints the full effective definition. Polling is diagnostic only. `PG18-E2E-008` proves runtime denial and fenced authorized change. |
| P0-4 RLS can make SQL/query-back and pgoutput visibility disagree | Accepted. First profile rejects RLS-enabled tracked tables; the walsender uses `row_security=off` so a later policy fails rather than filters. `PG18-E2E-008`/`012` exercise live/down changes with an unaffected control table. |
| P0-5 generated-column admission covered boot but not all lifecycle paths | Accepted and split. `PG18-001A` owns the canonical table/publication publishable schema; `PG18-001B` applies it to every consumer; `PG18-001Q` proves stored/virtual/unpublished/identity cases through the engine adapter. `PG18-E2E-003`–`005` and `012` cover boot, live, down-time, drift, restore, and partition policy. |
| P1-1 real invalidation and synthetic reason coverage were conflated | Accepted. `PG18-002A` handles fail-closed observation and real supported fixtures; `PG18-002B` separately integrates durable reset. Real primary cases, standby/future cases, and focused decoder/unknown-reason cases carry distinct evidence labels. |
| P1-2 TLS could fail on the first connector without testing the others | Accepted. `SEC-006A`, `OPS-003B`, `PG18-003A`, and `PG18-E2E-007`/`013` name setup/admin, pool/backfill/query-back, and walsender connectors; each is observed via `pg_stat_ssl` and independently subjected to wrong CA/SAN/rotation after other paths are healthy. |
| P1-3 `18.x` lacked executable minor maintenance/rollback | Accepted. `PG18-004` and `PG18-E2E-014` qualify approved `18.N -> 18.N+1` maintenance and rollback/reset. PG16/17 major import is explicitly logical export/restore plus generation reset unless a future profile implements official slot-upgrade prerequisites. |
| P1-4 SourceCommitID did not causally reach each materializer | Accepted. `E2E-000A` defines `source.committed`, adapter-specific `server.drainedThrough`, and keyed `client.appliedTailAfter` receipts after a post-server-barrier target fold/cache commit. An independent sentinel feed is insufficient. |
| P1-5 early isolated adapter was confused with public qualification | Accepted. `PG18-001Q` is engine-adapter qualification; `PG18-003Q`, `E2E-001Q`, and client runners repeat selected cases through immutable gateway/client candidates. Evidence records the adapter and cannot be relabelled. |
| P2-1 slot-retention values were only observed | Accepted in `PG18-000`, `OPS-003B`, capacity/storage tasks, and the signed production config: exact values are selected relative to outage/recovery bounds and preflight rejects unsafe combinations. |

## Review 10: server E2E/TDD differential

| Finding | Disposition in the canonical spec |
| --- | --- |
| P0 E2E-DR-001 source fence stopped before the client/app observation | Accepted through the three-receipt `E2E-000A` contract and corresponding updates to notes 22–24, `MIG-000`, and client scenarios. SQL equality alone is an oracle, not an application receipt. |
| P0 E2E-DR-002 core transaction/replay semantics were overclaimed | Accepted. All profiles protect the server source-log checkpoint from an incomplete PG transaction and converge under safe duplicate replay. Only `NATIVE_TXN_ATOMIC` adds one complete observer batch per eligible stream; no cross-stream atomicity is claimed. |
| P1 E2E-DR-003 DAG did not enforce red-first behavior | Accepted. Server, gateway, compatibility, native, and migration cases have explicit red-contract tasks (`E2E-001R`, `002R`, `003CR`, `003NR`, `004R`) before implementation/qualification. A stacked red patch and unchanged contract hash are mandatory. |
| P1 E2E-DR-004 stable IDs encoded internals and release-image cuts were not all possible | Accepted. Cut manifests have external candidate-image cuts and adjacent same-SHA instrumented invariant cuts. Stable scenarios assert public semantics only; internal checkpoint/catalog hooks stay focused diagnostics. |
| P1 E2E-DR-005 note 22 exposed API/control surfaces inconsistent with gateway-only topology | Accepted. Production acceptance uses isolated client and management networks, publishes only the authenticated gateway, and labels direct-isolated adapters as non-promotion evidence. |
| P1 E2E-DR-006 oracle independence and schema/key fidelity were weak | Accepted. `E2E-000A` owns an independently authored journal/SQL oracle that imports no production compiler. `PROTO-001C`, `PROTO-001D`, and `TST-002V` own schema/scalar/field-presence fidelity and separate tagged Electric/native key grammars. |
| P1 E2E-DR-007 stable public-image scenarios were missing | Accepted as `SRV-E2E-001`–`013`, `GW-E2E-001`–`008`, `PG18-E2E-001`–`014`, client/app scenarios, and `BND-E2E-001`–`005`, with immutable runners `E2E-001Q`–`005`. |
| P2 E2E-DR-008 timer tests lacked a two-tier policy | Accepted. Lease/TTL/retry/retention correctness uses a virtual clock at boundary values; real protocol timeout/cancellation cases use external events and bounded deadlines. No sleep creates ordering. |

## Review 11: Swift/app E2E differential

| Finding | Disposition in the canonical spec |
| --- | --- |
| P0 wrong Swift baseline/test placement | Accepted. `CMP-000` freezes the materially customized vendored `ios/Index/LocalPackages/ElectricSync` subtree in the actual app. Sibling `../electric-sync-swift` tests are separate conformance unless exact provenance is proved. Tests live in the vendored package, app ServicesTests, and app-hosted lifecycle target as appropriate. |
| P0 common fence was not a client-application fence | Accepted via `E2E-000A`, `MIG-000`, and the Swift `CausalFence`; the receipt is keyed by principal/template/generation/backend and follows the actual cache transaction. |
| P0 200-message response chunk can checkpoint too early | Accepted as `COMPAT-001` and task `CMP-004A`: 199/200/201/400/maximum messages, every committed local-chunk cut, cancellation/kill/replay, deletes/move-outs, and missing-versus-NULL. A final response cursor cannot name uncommitted cache effects. |
| P0 native core was coupled to app/GRDB/migration | Accepted. `SWF-000`–`SWF-006` and `E2E-003NR/NQ` use a minimal in-memory view plus independent checkpoint store. `APP-NATIVE-CONSUMER-001` or `APP-NATIVE-SINK-001` is selected separately; core imports no GRDB/ElectricSync/app DB. |
| P1 SYNC-002/effect oracle required optional transaction atomicity | Accepted. Core/compat scenarios allow documented event/response prefixes and duplicate replay; exact callback order is diagnostic. Only `TXN-001`/`E2E-003T` requires one transaction observer batch. |
| P1 account/mobile/claim-release semantics were not profile accurate | Accepted. Real auth teardown defines account/logout privacy; rollback within a principal is separate. Native named release and compatibility lease expiry have distinct assertions, and app-host/device jobs own suspension/kill/protected-data behavior. |
| P1 compatibility eligibility could be proved with a convenient model | Accepted. `CMP-001`/`CMP-002` generate the actual eligible-template manifest; zero eligible templates is an explicit profile failure/N-A decision, not toy-model evidence. |
| P1 key/scalar cases mixed incompatible formats and unsupported server promises | Accepted. `PROTO-001C`/`001D` and `TST-002V` select only supported manifest types and keep Electric structured keys distinct from native opaque/composite keys. Unsupported types fail admission before a feed. |
| P1 migration acceptance followed rehearsal | Accepted. `E2E-004R` precedes `MIG-002`/`002B`/`003`; `E2E-004Q` follows implementation and precedes `MIG-004`/`005` rehearsal and `MIG-006` shadow. |
| P2 fault injection elevated internal awaits to release behavior | Accepted. Stable app E2E cuts use request/body/cache-transaction/process/network/credential/lease boundaries; implementation-specific actor/parser/statement awaits live in versioned focused tests. |

## Review 12: task-DAG red team

| Finding | Disposition in the canonical spec |
| --- | --- |
| P0 duplicate authoritative task registries | Accepted. Note 24 contains scenarios/rationale and a task-owner mapping only; section 6 explicitly makes duplicate task definitions invalid. The canonical spec plus generated manifest are the sole authorities. |
| P0 profile closure was prose and contradictory | Accepted. The manifest has exact `lane` and `features` axes; inherited common closures and conditional edges are machine expressions. A required inapplicable dependency invalidates the profile. Common Electric qualification is explicit without exposing `/v1/shape` in a native-only public profile. |
| P0 migration acceptance/rehearsal order | Accepted through the `E2E-004R` -> implementation -> `E2E-004Q` -> rehearsal ordering above. |
| P0 PG18 packet order/ownership | Accepted. Admission, consumers, engine qualification, invalidation observation, slot continuity, reset integration, candidate packaging, maintenance, and final public qualification are separate `PG18-*` packets with explicit owners. |
| P0 gateway registry unowned | Accepted as `GWR-001`/`GWR-002`. The first profile has exactly one gateway process; multi-replica quota/registry HA is a future profile. Restore tests cannot reconstruct authority by guessing from an engine ID or DS path. |
| P0 PostgreSQL backup/PITR had no producer | Accepted as `PGR-001`, feeding `DSR-002`, `OPS-002`, `TST-012`, `RLS-001`, and G8 evidence. |
| P1 duplicate test ownership | Accepted. The scenario registry names contract owner, implementation owner(s), integration runner, profile, public oracle, and contract hash. Qualification adapters cannot edit the scenario contract. |
| P1 optional native modules lacked exact evidence edges | Accepted as profile-conditioned `E2E-003T/S/A/U` runners and generated dependencies through `SWF-013`, capacity, final E2E, and `TST-003`. |
| P1 native app integration had no principal owner | Accepted as mutually selected `APP-NATIVE-CONSUMER-001` or `APP-NATIVE-SINK-001`, including credential/account integration. `SWF-007` is now only the reusable atomic sink contract. |
| P1 immutable candidate versus final release artifact cycle | Accepted. Candidate producers emit immutable content-addressed artifacts for qualification; `RLS-001` later assembles/signs/attests those exact bytes and may not rebuild them. |
| P1 evidence status could launder skips/under-runs | Accepted. Task outcome and nested observations are distinct; only the validator emits profile N/A. Skip/filter/zero-test/under-run/stale/wrong digest/config/profile/hash maps to fail, while an environmental blocker remains non-promotable. |
| P1 G10 mixed pre-exposure readiness and post-exposure evidence | Accepted as G10a lab, G10b shadow, G10c beta, G10d separate 10%/50%/100% authorizations, and later G10e decommission. Initial 100% GA requires G0–G9 and G10a–d; decommission is not a prerequisite. |
| P1 fixed-operation work could run forever or move the goalposts | Accepted. Every workload has exact attempt/offered budgets, global/per-operation deadlines, minimum downstream counts, deterministic stop, signed pre-run divergence allowlist, pinned cohort denominator, and synthetic-versus-human accounting. |
| P1 packets violated subagent boundary size | Accepted. Harness work is split `E2E-000S/A/B/I`; PG schema work `PG18-001A/B/Q`; capacity `CAP-003A/Q`; all cross-runtime qualification packets are labelled for an additional reviewer. |
| P2 bootstrap bypassed validator | Accepted. `PLAN-001` is the only wave-0 merge; `GOV-001`/`TST-000` now depend on it. |
| P2 revocation lacked a byte-commit definition | Accepted. Revocation stops admission, cancels and joins reads, invalidates generation, then acknowledges. Begun public headers/body are classified pre-barrier; otherwise zero bytes may be emitted. Gateway probes record first-byte order. |

## Deliberate first-profile exclusions retained

The reviews did not justify silently expanding scope. These remain unsupported until a future profile
adds its own task closure: seamless failover slots, multi-gateway/registry HA, arbitrary public SQL/
ASTs, raw engine or DS access, CDN/direct capabilities, tracked-table RLS, virtual generated columns,
hot circuit/table reload, cross-stream transaction atomicity, and optional native modules without a
named production consumer.

## Validation performed after disposition

- Canonical task parser: 169 headings, 169 unique IDs, zero duplicate definitions.
- Dependency parser: zero unknown IDs and zero direct/transitive cycles.
- Conditional-edge parser: 36 exact conditional rows expanded over `COMPAT_V1` plus all 16 legal
  native feature combinations; zero unknown IDs and zero cycles.
- Scenario-table parser: 69 unique stable IDs—40 PG18/server/gateway/boundedness and 29 Swift/app;
  zero duplicate definitions.
- Literal bootstrap fronts: wave 0 `PLAN-001`; wave 1 `GOV-001`, `TST-000`.
- Note 24 contains no authoritative duplicate task definitions.
- Local Markdown links across the index/canonical/PG18/E2E/disposition notes: zero missing targets;
  new-file whitespace checks pass.
- Local image recheck: digest
  `sha256:06cad38a5d9f5d24b4d83d86def30795d5e4b757fedbf5281172b576dedcd941`
  reports `postgres (PostgreSQL) 18.6 (Debian 18.6-1.pgdg13+2)`.

No product code changed during disposition, so the full product suites were not rerun for these
Markdown-only edits. The inherited command evidence and its existing blockers remain recorded in
[`17-validation-baseline.md`](17-validation-baseline.md); the separate real-PG18 smoke is recorded in
notes 21 and 24.

`PLAN-001` still must materialize the checked-in machine graph, conditional profile closure, artifact
ownership checks, scenario hashes, and mutation fixtures before implementation agents are scheduled.
