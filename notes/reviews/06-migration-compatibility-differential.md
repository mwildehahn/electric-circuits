# Migration and compatibility differential review

**Disposition:** changes required before the migration plan is executable. The two-lane direction is sound, but the current task graph, compatibility eligibility, comparison fence, cache ownership, and rollback contracts do not yet support a safe production cutover.

**Evidence boundary:** reviewed `notes/08-swift-compatibility-gap.md`, `09-swift-library-strategy.md`, `10-deprecation-and-successor-context.md`, `11-test-and-rollout-strategy.md`, and `16-production-readiness-and-swift-migration-spec.md`; the current Circuits `/v1/shape`, protocol, output, and client surfaces; `../electric-sync-swift` at `0.1.12` (`6bdde65`); and the available production app repository `../indexed-mighty-prod-ecs-proof`. For the production-app baseline I inspected `origin/main` at `e1f7339520`, because the checked-out app branch is 916 commits behind it. That baseline pins `electric-sync-swift` exactly to `0.1.8` and contains 32 model-specific Electric collection/provider files. This version distinction is itself a blocker below.

## Ranked findings

### P0 — The task graph contains dependency cycles and cannot be delegated as written

Three explicit cycles make a topological execution impossible:

- `PROTO-003` depends on `ENG-002`, while `ENG-002` depends on `PROTO-003`.
- `ENG-010` depends on `OPS-002`, while `OPS-002` depends on `ENG-010`.
- `ENG-013` depends on `OPS-004`, while `OPS-004` depends on `ENG-013`.

These are not harmless cross-references: each task is described as a blocker whose acceptance is required by the production gates. Agents cannot infer which half is the design contract and which half is the implementation.

**Required edit:** add `PLAN-001 — Make the production task graph acyclic`, and split the cycles at contract boundaries:

- `PROTO-003A` specifies transaction framing from `PROTO-001`; `ENG-002` implements it; `PROTO-003B` supplies executable fixtures/negotiation against the implementation.
- `ENG-010A` specifies durable accounting and recovery invariants; `OPS-002A` proves the storage backup/restore primitives; `ENG-010B` implements enforcement; `OPS-002B` executes the end-to-end restore proof.
- `OPS-004A` specifies topology, fencing, and failover states; `ENG-013` implements the database fence; `OPS-004B` executes failover.

**Fixed-operation acceptance:** parse every `Depends on` edge in the spec, resolve ranges to concrete task IDs, and topologically sort the complete graph in CI. Acceptance is zero cycles, zero unknown IDs, zero self-dependencies, and every G0–G10 prerequisite reachable from a named root. A mutation that restores each of the three current back-edges must fail the check.

### P0 — The compatibility audit is against the wrong Swift package baseline

The production app at the inspected `origin/main` pins `electric-sync-swift` `0.1.8` in its resolved files, SwiftPM manifests, and Tuist manifests. The sibling library inspected by the notes is `0.1.12`; `0.1.8..0.1.12` changes nine files by 5,340 insertions and 324 deletions, concentrated in collection, replica, recovery, and client behavior. Conclusions derived from `0.1.12` cannot be attributed to the shipped `0.1.8` API or recovery semantics. The checked-out app branch also vendors a newer local `ElectricSync` package and is far behind `origin/main`, so it is useful implementation evidence but not a stable release baseline.

**Required new task:** `CMP-000 — Freeze the app and Swift compatibility baseline`, owned jointly by the app and compatibility leads, before `CMP-001`–`CMP-005`.

**Fixed-operation acceptance:** check in a manifest containing the app commit, package version/commit, Circuits commit, schema version, and semantic epoch. Build and execute all compatibility fixtures from a clean checkout at exactly those revisions. Every Swift symbol or behavior cited by CMP tasks must be proved present at `0.1.8` or the plan must add an explicit, independently reversible `0.1.8 -> chosen-version` app upgrade. CI must fail when any pinned revision changes without regenerating the inventory and compatibility corpus.

### P0 — `/v1/shape` eligibility is materially narrower than the plan says

The current compatibility proposal treats modes as eligible when they do not depend on tags, `changes_only`, `order`, or `limit`. The actual app and package behavior is more constrained:

- On-demand starts at offset `now` with `log=changes_only`. Circuits `/v1/shape` exposes neither `log` nor equivalent semantics.
- Progressive queries fetch an ordered/limited subset snapshot and then keep a broader live shape. `/v1/shape` has no subset, order, or limit surface, so a progressive call site is not unchanged-compatible even when its predicate is simple.
- The production app's collection topology uses DNF for several models, and its protocol semantic epoch is the tagged-shape epoch. The Swift library's ownership tracker uses `tags`, `removedTags`, `activeConditions`, and move-in/move-out semantics to preserve overlapping disjunct ownership. Circuits deliberately emits absolute row changes and has no corresponding tag protocol.
- `/v1/shape` is a generic table/where endpoint. App providers are model-specific adapters over authenticated OpenAPI shape routes; they translate structured filters, preserve response-field presence, decode generated model types, and often implement SSE. This is not a base-URL substitution.

The first compatibility cohort is therefore limited to eager, statically simple, full-shape subscriptions with exactly one authoritative feed/ownership domain, no ordering/limit/subset requirement, acceptable full-shape cardinality, a compatible primary-key/value codec, and no UI requirement for source-transaction atomicity. DNF, on-demand, and current progressive paths must be ineligible until redesigned or moved to the native lane. The fact that the Electric conformance delta is described as “only tag fields” does not make tags normalizable for the production app: a final row-map comparison can hide a wrong future deletion.

**Required edits:** rewrite `CMP-001` to generate a checked-in call-site/registration inventory from the exact app revision, and rewrite `CMP-002` around an explicit eligibility predicate. Do not use “100% observed production shapes” as the inventory boundary. The manifest must include every registered model, every statically discoverable call site/wrapper, effective sync mode, topology, predicate/template ID, ordering/limit, transport, primary-key codec, owner feed, cardinality bound, projection/delete behavior, and disposition with reason. Add a compile/runtime admission guard that rejects an unmapped or ineligible template before sending a request.

**Fixed-operation acceptance:** the generator must account for all 32 registered model adapters in the pinned app baseline plus every discovered call site and wrapper expansion; one added unmapped registration and one added unmapped call site must each fail CI. For every eligible template, run 100 deterministic journals containing insert, value update, predicate move-in, predicate move-out, delete, reconnect, handle loss, and app restart; compare the canonical rows after every operation, not just at quiescence. For every unsupported mode/capability combination, execute at least 10 attempts and prove zero Circuits requests were emitted and the reason was recorded. Add a DNF overlapping-owner trace in which removing one disjunct must not delete the row; any normalizer that drops the ownership distinction must fail.

### P0 — `CMP-003` has not selected a feasible provider boundary

Directly exposing generic `/v1/shape` to the app would disclose table/column/predicate structure, uses the engine's query-secret convention rather than the app's bearer-authenticated OpenAPI routes, and leaves every model-specific response decoder to be replaced. Conversely, making each existing model provider construct raw `/v1` SQL/AST would duplicate server policy in the client. The current task asks whether an existing provider can merely target a different URL, despite the local surfaces already demonstrating that it cannot.

**Required edit:** make `CMP-003` explicitly deliver two components:

1. an app-side `CircuitsV1HTTPProvider` conforming to the pinned Swift package's actual provider contract; and
2. an authenticated, server-owned compatibility gateway mapping allowlisted template IDs and typed parameters to internal `/v1/shape` requests and model response schemas.

The gateway, not the app, owns SQL/AST construction and Circuits credentials. The task must decide long-poll versus SSE fallback, 204 behavior, restart/handle-409 behavior, field-presence preservation, scalar conversion, composite keys, cancellation, and retry classification. `CMP-002` should depend only on the existing `/v1` contract characterization; do not make that characterization wait on unrelated native-client protocol work in `ENG-003`.

**Fixed-operation acceptance:** for every eligible template, replay a checked-in corpus through the real app provider, gateway, and engine consisting of at least 25 responses each for snapshot, live data, 204 timeout, malformed payload, 401/403, 409 must-refetch, 429, 5xx, cancellation, and handle expiry/restart. Verify byte-level request allowlisting, typed decode, field presence, key parsing, retry class, and resulting row operations. Fuzz 10,000 rejected template/parameter combinations and prove none can select a non-allowlisted table, column, or predicate.

### P0 — Cache generation is underspecified and cannot be delegated to a generic compatibility task

The app does not have a disposable “Electric cache” whose rows can all be swapped by changing metadata. Electric writes feed canonical GRDB domain tables observed by the UI; those tables may also contain optimistic/local writes. The library maintains fetch metadata and, for tagged/DNF shapes, durable per-row ownership. Model providers also encode partial projection, deletion, and dependent-cleanup rules. A second metadata generation without row isolation can mix old-provider and candidate rows while still presenting a coherent-looking final map.

This requires a per-model product/data ownership decision: shadow database or tables versus generation columns; which queries are generation-filtered; how optimistic writes overlay both generations; who owns deletes; how projections merge; what happens to dependent rows; and how rollback obtains a fresh source. Those choices affect every user-visible query and are not safely delegable to a generic library or migration agent.

**Required new task:** `APP-OWN-001 — Define per-model row ownership and generation isolation`, owned by an app data engineer and product owner. It blocks `CMP-004`, `MIG-002`, and any cutover.

**Fixed-operation acceptance:** the checked-in ownership manifest maps every registered model and every writer/reader of its destination tables. For each eligible template, run 100 schedules that interleave legacy sync, Circuits sync, optimistic insert/update/delete, login generation change, crash, retry, and rollback at every promotion state. Every user-visible read must select exactly one committed remote generation plus the documented optimistic overlay; zero candidate rows may appear before promotion, zero legacy-only rows after promotion, and no delete may remove a row still owned by another feed. A source mutation adding an unclassified table writer or reader must fail CI.

### P0 — The shadow comparator has no common fence and permits unsafe normalization

`MIG-001` asks for a comparison at a “caught-up point” and a causal report containing LSN/xid/seq and both offsets. That point is not currently observable across the two systems. Electric offsets and Circuits positions are different opaque domains; `/v1/shape` does not expose a source transaction identity, and its current up-to-date control cannot establish that a named PostgreSQL commit has been applied by both backends. Polling until row maps happen to become equal is circular and can compare early or pass after both systems make the same stale omission.

The proposed tag normalization is also unsafe unless it is per-template and proved irrelevant to ownership. Finally, a production report containing raw rows, predicates, handles, offsets, and transaction identifiers conflicts with the spec's own production-data minimization requirements.

**Required new task:** `MIG-000 — Define a cross-backend comparison fence`, before `MIG-001` and `CMP-005`. Use a named source transaction sentinel/control relation processed by both services, or an authenticated server-side ingestion barrier with equivalent proof. Do not compare until both backends attest processing past that same commit.

**Required edit to `MIG-001`:** use a typed, per-template normalizer allowlist; forbid catch-all ignored fields. Compare operation traces and ownership state for any topology that can overlap, not only final rows. Split artifacts into synthetic/preproduction mode, where fixture-safe causal values may be retained, and production mode, where row values, predicates, handles, offsets, xid/LSN/seq are replaced by keyed hashes or request IDs with controlled server-side lookup.

**Fixed-operation acceptance:** run 10,000 deterministic journals with independent delay, reordering, reconnect, and replay injected into each backend. A deliberately delayed backend must block the fence and must never be reported equal or divergent early. Mutations that remove or alter each of insert, value, move-in, move-out, delete, reset, ownership, and generation events must all be detected. Every production-mode artifact must pass automated secret/PII/raw-row scanning; the synthetic mode must retain enough fixture-safe evidence to reproduce every injected divergence.

### P0 — The rollback plan can intentionally expose stale data

Phase 4 describes retaining the legacy cache and metadata read-only after Circuits becomes active. If legacy ingestion is stopped, that generation becomes stale immediately. `CMP-006`'s “one-operation rollback” and `MIG-003`'s restore-prior-owner language could then make stale rows visible. The general prose mentions forcing an upstream snapshot when reuse is unsafe, but no per-template contract chooses between a warm standby and a cold, fenced rebootstrap, nor does the RTO begin only after freshness is proved.

**Required new task:** `MIG-002B — Define the rollback freshness contract`, dependent on `APP-OWN-001` and blocking `MIG-003`–`MIG-005`. Each template must choose either:

- **warm rollback:** keep the legacy provider and generation continuously caught up, budget the duplicate load, and fence promotion; or
- **cold rollback:** keep the old generation invisible and block exposure until a new upstream `offset=-1` snapshot/live fence completes into a fresh generation.

Rollback must never make a merely retained generation authoritative.

**Fixed-operation acceptance:** for each eligible template, cut over, apply 100,000 writes while the legacy generation is cold or warm as declared, and trigger rollback at 100 deterministic cut points spanning drain, close, generation promotion, crash, and restart. At every user-visible read there must be one fresh authoritative generation, no stale-generation exposure, and no mixed ownership. The cold path must complete a new snapshot plus common fence within the declared RTO; the warm path must prove continuous catch-up and remain within the doubled-load capacity budget.

### P1 — The gates collapse two lanes into one release and make the new library unconditional

The notes correctly distinguish an incremental compatibility lane from an independent native end state. The production spec then defines the first production release as including both Electric-compatible and native shape/aggregate clients, and G7 requires all `CMP-001`–`CMP-006` and `SWF-001`–`SWF-013`. That prevents shipping a validated compatibility cohort until a complete new native library is also built. It removes the risk-reduction benefit of two independent lanes.

An independent library is justified as the long-term architecture: the native API should not inherit Electric tags, DNF topology, tRPC internals, or GRDB policy. Its immediate product scope is not yet justified, however. The plan chooses native shapes and aggregates before the call-site inventory establishes a production template needing each surface; `SWF-008` may therefore be mandatory speculative work.

**Required edits:** define two independently shippable profiles:

- `COMPAT-RC`: common governance/security/operations/engine prerequisites plus CMP, eligible-template testing, and migration/rollback tasks.
- `NATIVE-RC`: common prerequisites plus native protocol, SWF, native integration, and only the native surfaces selected by a product ADR.

Make `SWF-001` depend on `CMP-001` and a `NATIVE-ADR-001` naming at least one production template/use case, owner, package repository, minimum platform/compiler, release/version policy, and reason compatibility mode cannot serve it. Make aggregate and subset modules conditional on named inventory demand and engine readiness. Neither profile may silently satisfy the other's acceptance.

**Fixed-operation acceptance:** produce two acyclic gate matrices and run the release evaluator against four fixtures: compatibility-only pass, native-only pass, both pass, and one-lane failure. Each lane must ship and roll back without linking, initializing, or mutating the other lane. The native ADR must bind every required module to at least one inventory row; a module with zero consumers is removed from the first-release gate.

### P1 — Native delivery acknowledgement and sink policy are unresolved

`SWF-005` says offsets advance only after sink/consumer acknowledgement, while `SWF-006` proposes an `AsyncSequence`-style consumer surface without an acknowledgement operation. Yielding an element is not proof that a caller durably applied it. Persisting before application loses data after a crash; persisting after an unacknowledged yield can duplicate non-idempotent side effects. `SWF-007` also asks a general library to define overlapping ownership, projection merge, and deletion policy that belongs to the app's data model.

**Required new task:** `SWF-000 — Decide native delivery and acknowledgement semantics`, before `SWF-004`–`SWF-007`. Specify single versus multiple consumer ownership, manual acknowledgement, batching, cancellation, cursor persistence, replay, and whether materialized observation is distinct from a transactional sink. The library may provide a reference sink and ownership primitives, but `SWF-007` must delegate model merge/delete/projection policy to the `APP-OWN-001` adapter rather than claiming a universal policy.

**Fixed-operation acceptance:** for 10,000 envelopes, inject process termination before yield, after yield/before acknowledgement, during sink transaction, after sink commit/before cursor persistence, and after cursor persistence. The documented contract must produce no loss; duplicates must be either impossible or explicitly delivered and idempotently absorbed by the reference sink. Run each crash point with cancellation, retry, reset, and a two-consumer contention attempt; a second unauthorized cursor owner must be rejected deterministically.

### P1 — Key, scalar, projection, and batch semantics need an explicit compatibility codec task

Circuits `/v1` currently derives keys with its own single/composite-PK string encoding and renders PostgreSQL scalar values through its adapter. The app providers decode generated, model-specific payloads and preserve whether fields were absent. Silent coercion of numeric/bool/null values, a composite-key mismatch, or treating an omitted projection as null can corrupt GRDB state while passing simple JSON fixtures. A `/v1` response batch is atomic at the Swift apply boundary but does not by itself expose source PostgreSQL transaction boundaries.

**Required edit:** split `CMP-002` into wire/lifecycle characterization and `CMP-002B — Prove app codec equivalence`. Classify every eligible model's primary key, supported PostgreSQL types, nullability, default/generated fields, projection behavior, and transaction-observer requirement. No fallback key parser or scalar coercion may be admitted without a canonical fixture.

**Fixed-operation acceptance:** for every eligible column and key component, round-trip minimum/maximum/zero values, null, empty string, delimiter/escape characters, non-ASCII, timestamp/time-zone boundaries, decimal precision, arrays/JSON where present, and absent-versus-null projections through PostgreSQL -> Circuits -> gateway -> pinned Swift decoder -> GRDB. Execute 10,000 generated composite keys with zero collisions and exact delete targeting. Templates whose observers require source-transaction atomicity must be rejected from the compatibility cohort until the wire can prove it.

## Required spec change set

The minimum safe edit set is:

1. Add `PLAN-001`, `CMP-000`, `APP-OWN-001`, `MIG-000`, `MIG-002B`, `NATIVE-ADR-001`, and `SWF-000` with the dependencies and operation-count acceptance above.
2. Rewrite `CMP-001` as a generated, revision-pinned inventory and admission manifest; explicitly classify DNF, on-demand, progressive, tags, ownership, projection, codec, and transaction requirements.
3. Rewrite `CMP-002`/add `CMP-002B` to characterize the current `/v1` independently of the future native protocol and prove model codecs.
4. Make `CMP-003` an authenticated allowlisted gateway plus pinned-package provider implementation, not a base-URL experiment.
5. Make `CMP-004` and `MIG-002` depend on the app-owned row/generation contract; make `MIG-003`–`MIG-005` depend on rollback freshness.
6. Add the common comparison fence and typed normalization/privacy modes to `MIG-001`.
7. Split G7 and the release scope into `COMPAT-RC` and `NATIVE-RC`; gate native modules on inventory-backed use cases.
8. Restrict `SWF-007` to mechanisms and a reference sink; keep model ownership/merge policy in the app adapter.

Until these edits land, the safe interpretation of the plan is: no production call site is compatibility-eligible by default; DNF, on-demand, and progressive are explicitly ineligible; no shadow equality is meaningful without a common source fence; and no rollback may expose a retained but unfenced legacy generation.
