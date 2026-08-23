# Swift differential hardening review

Read-only comparison of the Circuits notes (`08`, `09`, `18`, `23`) against:

- `../electric-sync-swift` at `6bdde65a7c234371da829b0de24af12e00266fa8`
- `../indexed-mighty-prod-ecs-proof` at `2168965405d7d385f2a0c7b470ea035de1c6cc89`

The sibling checkouts were clean. No claims below are release evidence; they are differential
inputs that should be incorporated into the generated `PLAN-001` manifest and then proven on the
exact candidate.

## Executive disposition

The native-vs-compatibility decision in notes 08/09/18 is directionally correct: keep the existing
ElectricSync lane as a narrow compatibility bridge and build a separate dependency-free native
package. The current ECS app, however, is materially more expressive than the compatibility profile
allows. It has 28 model-specific providers, subset snapshot requests, typed predicates, progressive
and on-demand collections, order/limit queries, SSE providers, optimistic replay, and GRDB-owned
metadata/cache. A broad `COMPAT_V1` migration is therefore not evidenced; the first release must
either admit a small, explicitly enumerated subset of templates or select native/app-redesign tasks
for the rest. The existing sibling package is useful conformance input but cannot stand in for the
vendored ECS package.

## Ranked findings and required amendments

### P0-1 — The app inventory can invalidate the assumed compatibility-first rollout

**Evidence.** The ECS `ElectricCollectionBuilder` accepts `syncMode` (`eager`, `progressive`,
`onDemand`), `liveTransport`, `orderBy`, and `limit`
(`../indexed-mighty-prod-ecs-proof/ios/Index/LocalPackages/Services/Sources/Services/Collections/ElectricCollectionBuilder.swift:53-62`).
The application uses progressive/on-demand and ordered/limited collections in
`ios/Index/AuthenticatedContentView.swift` and `LocalPackages/AppRouting/*`; its model providers
emit subset snapshots via `ShapeSubsetRequestSupport.swift:5-76`. `Record+ElectricCollection.swift:28-60`
uses a subset POST path, while `:96-110` explicitly opens an SSE path. `CalendarEvent` has a
typed-vs-subset decision (`CalendarEvent+ElectricCollection.swift:169-182`). There are 28
model-specific `*+ElectricCollection.swift` providers, not one generic URL provider.

This conflicts with any reading of `COMPAT_V1` as a replacement for current app behavior: note 18
explicitly rejects DNF/tags, on-demand/changes-only, progressive, order/limit, subset, SSE and
transaction-observer semantics. Notes/FINDINGS in the ECS checkout also contain an older statement
that the app does not expose `subset__*`; the checked-out code now does, so that document is stale
relative to the pinned app tree.

**DAG amendment.** Add `CMP-001A` (generated executable-call-site census and capability ledger),
depending on `CMP-000` and `CMP-001`, which records counts and source locations for each mode,
subset/order/limit/SSE/tag/ownership operation and emits one row per model/consumer. Make
`CMP-002` consume this ledger, not prose. Add explicit `APP-REDESIGN-001` (or native module edge)
for every rejected call site. For `NATIVE_SUBSET` and `NATIVE_AGGREGATE`, add app integration
tasks (`APP-NATIVE-SUBSET-001`, `APP-NATIVE-AGGREGATE-001`) or make the inventory explicitly say
package-only/no app consumer; current `E2E-003U`/`E2E-003A` have no selected app adapter edge.

**Gate amendment.** G7a and `E2E-003CR/CQ` must require a non-empty admitted-template manifest,
or a machine-readable `no_eligible_template` decision that blocks compatibility GA and points to
the native/redesign closure. A passing toy template must not authorize the 28-model app.

### P0-2 — 409/must-refetch currently leaves stale rows in the real GRDB cache

**Evidence.** The ECS findings identify a concrete correctness gap: HTTP 409 becomes `.truncate`,
`SyncBatch.apply` clears metadata/reset state, but the local synced rows remain because
`IndexElectricDataCacheProvider.clear` is intentionally a no-op
(`../indexed-mighty-prod-ecs-proof/notes/FINDINGS.md:240-261`, `:369-377`). The compatibility provider
also has per-model dependent cleanup and `ElectricSyncOptimisticReplayEventHandler` wiring, so
row removal cannot be abstracted as metadata reset alone.

Note 18's `CMP-004A`/`CMP-005` do require generation replacement and checkpoint safety, but they do
not bind that requirement to the existing no-op clear, per-model dependent cleanup, and optimistic
replay hooks in the vendored implementation; without that binding a generation label can pass while
old rows remain readable.

**Task amendment.** Split `CMP-004A` into checkpoint safety and `CMP-004B` (authoritative reset
semantics): on 409, retirement, or typed reset, atomically hide/delete the old synced generation,
preserve only explicitly registered local/optimistic overlays, invoke dependent cleanup, and mark
the new generation incomplete until its terminal control/application receipt. Depend
`CMP-005`, `E2E-003CR`, and `E2E-003CQ` on `CMP-004B`.

**E2E amendment.** Add `RESET-001` to note 23: seed a row that no longer matches after a forced
409, plus a local-only/optimistic dependent row; release the reset, kill at each cache transaction
cut, and assert stale synced rows are never readable, local-only policy is preserved, and the new
generation equals SQL at its application receipt.

### P0-3 — `snapshot-end` is not `up-to-date`; the app currently conflates them

**Evidence.** ECS `ElectricMessageCoder` emits `snapshot-end`/`subset-end`, and the checked-in
findings report that many HTTP clients set `isUpToDate` for either control, while Electric semantics
require `snapshot-end` to remain distinct until `up-to-date`
(`../indexed-mighty-prod-ecs-proof/notes/FINDINGS.md:330-338`, `:450-459`). Subset snapshot metadata
is explicitly produced in `ShapeHTTPClientProtocol.swift` and all model providers. This is a direct
boundary for the proposed Circuits adapter, not a hypothetical future feature.

**Task amendment.** Strengthen `CMP-002B` with a control-state corpus that proves
`snapshot-end`, `subset-end`, `up-to-date`, empty-page/204, and `must-refetch` are distinct. Add
`CMP-004B` dependency on that corpus. If compatibility mode deliberately excludes subsets, the
adapter must reject a subset request before network work rather than map `snapshot-end` to ready.

**E2E amendment.** Add `CONTROL-001` to note 23: hold a subset snapshot after `snapshot-end` but
before `up-to-date`, then crash/reconnect; readiness/checkpoint must not advance early and a later
live delete must not resurrect the snapshot row. Include a compatibility negative proving an
unsupported subset produces no gateway/engine request.

### P1-1 — Legacy and native resume identities need an explicit migration task

**Evidence.** Sibling ElectricSync persists `SyncState` fields `offset`, `handle`, `cursor`,
`isUpToDate`, and semantic epoch; ECS metadata is keyed by `(table, predicateHash)` and stores
fetch ranges as well as the state (`Shapes/ElectricSyncProviders.swift:1-140`). Native notes
correctly distinguish durable stream position, snapshot-page LSN, tombstones, page cursor, feed ID,
subscription, and cache epoch, but the DAG has no task whose sole boundary is migrating the old
metadata schema and rejecting mixed tokens.

**DAG amendment.** Add `SWF-014 — Versioned checkpoint/metadata migration` depending on `SWF-000`,
`SWF-002`, `SWF-004`, and `APP-OWN-001`, with edges to `MIG-001`, `APP-NATIVE-*`, and `E2E-004R/Q`.
It must define old `SyncState` retention/invalidation, native token tags, schema/key-codec
fingerprints, cold rebootstrap, and an atomic cache-epoch transition. Add a validator rule that a
v1 offset/handle/cursor can never deserialize as a native LSN/feed/cursor.

**E2E amendment.** Add `TOKEN-001`: persist every token at delivery-before-apply,
apply-before-checkpoint, schema mismatch, account switch, process kill, and rollback; restart must
either replay the same lane or perform a typed reset, never continue with a cross-lane token.

### P1-2 — Actor guidance is good for native, but compatibility/app lifecycle remains unchecked

**Evidence.** The sibling stream manager is `@unchecked Sendable` and synchronizes a mutable entry
map with a serial `DispatchQueue`; release schedules a delayed GC task and cancellation can race
with the task start (`electric-sync-swift/Sources/ElectricSync/ElectricCollectionStreamManager.swift:1-236`).
The sibling has a useful actor-based `DeduplicatedLoadSubset` with generation fencing
(`DeduplicatedLoadSubset.swift:1-160`). The ECS app additionally has `@unchecked Sendable` model
providers and many `Task.detached` call sites (for example `IndexApp.swift` and
`DataLoader/SuggestionLoader.swift`). These are exactly the ownership/cancellation edges that a
compatibility migration must preserve or replace.

**Task amendment.** Keep `SWF-004` actor-only for native, and add to `TST-006C` a compatibility
manager model covering acquire/release/GC, account teardown, duplicate subscriber counts, late
completion, and old-generation publish suppression. Add a `CMP-006` app-host gate for every
`@unchecked Sendable` provider and ordinary `Task.detached` sync call: either prove a Sendable
boundary and cancellation propagation or register it as an app-owned exception with a focused
test. Amend `SWF-010` to require repeated background activation, not only one resume.

**E2E amendment.** Extend `LIFE-002`, `LIFE-005`, and `LIFE-006` with: release during delayed GC,
account teardown while a provider response is held, two subscribers closing concurrently, and
repeated background/foreground cycles after the first preload. Assert no late old-generation row,
checkpoint, or claim publication.

### P1-3 — Auth/provider boundary must be app-owned, not inferred from generic URLSession

**Evidence.** The sibling package deliberately has no built-in auth/URLSession policy; ECS model
providers obtain `apiClientProvider` through dependency injection and some SSE providers obtain
tokens from `keychainClient` (`Record+ElectricCollection.swift:96-123`). The app's registry can
fall back to inert no-op providers in previews/tests and otherwise traps when no model provider is
registered (`ElectricCollectionBuilder.swift:73-104`). Native note 18 specifies bearer headers,
but compatibility note 23 correctly says the gateway owns template/auth/SQL construction.

**DAG amendment.** Add `CMP-003A — App credential/provider binding` depending on `SEC-002A`,
`SEC-006B`, `CMP-000`, and `CMP-003`; require the real API-client/keychain owner, refresh and
logout teardown, redirect policy, and no raw predicate/table/DS identity. Make `CMP-004` depend on
`CMP-003A`. For native, make `SWF-003B`/`SWF-012` require an injected credential provider but no
dependency on the ECS keychain implementation.

**E2E amendment.** `AUTH-001/002/003` must run through the actual ECS credential provider and
keychain/account teardown, not only a generic HTTP fake. Add an assertion that preview/test no-op
providers cannot satisfy a release profile (they must be rejected by the profile harness).

### P1-4 — Subset and SSE need explicit native/app ownership, not a silent compatibility fallback

**Evidence.** ECS's subset compiler supports raw SQL where/params/order/limit/offset
(`ShapeSubsetRequestSupport.swift:43-76`), while native Circuits supports a JSON predicate AST,
one order clause, LSN-fenced page/feed, tombstones, and keyset load-more. Existing app providers
also explicitly reject SSE+subset combinations (`Record+ElectricCollection.swift:96-110`) and
select typed calendar snapshots (`CalendarEvent+ElectricCollection.swift:169-182`). These are
capability branches, not one interchangeable API.

**DAG amendment.** Add `APP-NATIVE-SUBSET-001` (or a named redesign task) to own translation from
the app's `SubsetSQLCompiler` output to the native AST/one-order/keyset contract, including a
reject-before-network matrix for raw SQL, multi-order, offset-live, and unsupported composite
keys. Add it as a generated edge of `NATIVE_SUBSET`, `MIG-002`, `E2E-003U`, and `TST-003` whenever
the inventory selects a subset consumer. Add an analogous aggregate app task if any aggregate
consumer is selected.

### P1-5 — Transaction acknowledgement and observer semantics need an explicit app decision

**Evidence.** Sibling ElectricSync applies each response in a caller-supplied DB transaction but
does not expose a durable source-transaction acknowledgement API; ECS findings note no equivalent
of TanStack `awaitTxId`/`awaitMatch` (`notes/FINDINGS.md:340-349`). Circuits shape output currently
does not propagate a final `last` marker; note 08 therefore correctly limits native default to
event-level local transactions and gates source-transaction observers behind `NATIVE_TXN_ATOMIC`.

**Task amendment.** Add an explicit `APP-TXN-ADR-001` under `NATIVE-ADR-001`/`SWF-000` that records
whether any app observer requires source-transaction atomic visibility or only causal application
receipts. If atomic, require `PROTO-003B`, `ENG-002`, `SWF-005B`, and an app observer sink task;
otherwise add a public statement/test that intermediate event-level states are permitted. Do not
let a GRDB write transaction be mistaken for source-transaction acknowledgement.

**E2E amendment.** Extend `SYNC-002` with an explicit allowed-prefix assertion and add
`TXN-APP-001` for the selected atomic profile: hold final marker/application commit and verify
observers cannot see a partial transaction; for event-level profiles, verify only final fenced SQL
and checkpoint safety.

### P2-1 — Rollback tasks need app schema/migration and optimistic-overlay ownership

**Evidence.** ECS has GRDB migrations, local-only fields, dependent cleanup, and
`ElectricSyncOptimisticReplayEventHandler` passed by `ElectricCollectionBuilder` at lines 197-203.
Notes 23's ownership manifest asks for local-only fields and optimistic overlays, but `APP-OWN-001`
and `MIG-002B` do not require the app schema migration/version or replay handler to be included in
the rollback artifact.

**DAG amendment.** Expand `APP-OWN-001` manifest fields with DB schema/semantic epoch, local-only
columns, optimistic journal/replay handler, dependent cleanup, reader/observer source, and owner
release operation. Add `APP-MIG-001` (app DB/generation migration and rollback) depending on
`APP-OWN-001`, `CMP-000`, and `GOV-004`; make `MIG-002`, `MIG-002B`, `MIG-004`, and `OPS-009`
consume its hash. A cold rollback must rebootstrap the old schema/provider when that hash is absent
or stale.

**E2E amendment.** Extend `OWN-002`, `OWN-003`, `ROLL-001`, and `ROLL-002` with a local-only
column, optimistic mutation, dependent child row, schema migration before/after cutover, and crash
after replay but before ownership switch. The reader must expose one complete generation and no
cross-generation local field loss.

### P2-2 — Existing tests are valuable controls, not qualification

**Evidence.** `electric-sync-swift` has extensive Swift Testing for client, truncation, buffering,
subset dedupe, and replica-owner lifecycle (for example `ElectricSyncClientTests.swift`,
`SubscribeTruncateTests.swift`, `ElectricReplicaOwnerLifecycleTests.swift`), but they are package
tests with scripted providers. ECS has focused tests for GRDB provider, truncation, observer and
subscription lifecycle, but no evidence of the exact PG18→gateway→engine→real app path. Notes 18/23
already say sibling results are separate conformance; this review confirms that rule should be
machine-enforced.

**Gate amendment.** `TST-000`, `E2E-003CQ`, and `E2E-003NQ` should record a provenance tuple for
every Swift result (package/app commit, vendored subtree hash, gateway/engine/DS and PG18 digests).
The profile validator must reject a sibling-only or no-op-provider pass as `green_candidate` for an
app task. Keep sibling tests as `inherited_control`/focused mechanism evidence only.

## Concrete high-level E2E additions to note 23

The following cases are the smallest additions that close gaps found in the sibling trees:

| ID | Public contract | Profiles | Required task/gate edges |
| --- | --- | --- | --- |
| `RESET-001` | Forced 409/must-refetch hides stale synced rows, preserves declared local/optimistic data, and exposes the replacement only after its application receipt. | `COMPAT_V1`, `MIGRATION` | `CMP-004B`, `E2E-003CR/CQ`, `E2E-004R/Q` |
| `CONTROL-001` | `snapshot-end`/`subset-end` never imply `up-to-date`; delayed page/live/reset cannot advance readiness or resurrect deletes. | compatibility negatives; `NATIVE_SUBSET` | `CMP-002B`, `SWF-009`, `E2E-003CR`, `E2E-003U` |
| `TOKEN-001` | v1 offset/handle/cursor, native stream position, page LSN, feed ID, and cache epoch are tagged and never cross lanes after crash/schema/account changes. | all client/migration lanes | `SWF-014`, `MIG-001`, `E2E-003NR`, `E2E-004R` |
| `APP-LIFE-001` | Repeated background/foreground, delayed GC release, duplicate close, and account teardown leave one owner and no late old-generation publication. | `COMPAT_V1`, native app | `TST-006C`, `SWF-010`, `CMP-006`, `E2E-003CQ/NQ` |
| `APP-AUTH-001` | Real dependency-injected API/keychain credential refresh and logout fence all provider/SSE requests; preview/no-op clients cannot pass release. | `COMPAT_V1`, native app | `CMP-003A`, `AUTH-001/002/003`, `E2E-003CQ/NQ` |
| `SUBSET-APP-001` | App raw subset compiler capabilities translate only to admitted native AST/order/keyset forms; unsupported forms are rejected before network work. | `NATIVE_SUBSET` or redesign | `APP-NATIVE-SUBSET-001`, `SWF-009`, `E2E-003U` |
| `TXN-APP-001` | If selected, an app observer sees a complete source transaction only after final marker and sink/checkpoint commit; event-level lanes assert only causal receipt. | `NATIVE_TXN_ATOMIC` when selected | `APP-TXN-ADR-001`, `PROTO-003B`, `ENG-002`, `SWF-005B`, `E2E-003T` |
| `ROLL-APP-001` | Cutover/rollback carries DB schema epoch, local-only fields, optimistic replay and dependent cleanup; stale incumbent is held/rebootstrapped. | `MIGRATION` | `APP-MIG-001`, `MIG-002B`, `E2E-004R/Q`, `OPS-009` |

## Suggested generated dependency deltas

```text
CMP-000 -> CMP-001 -> CMP-001A -> CMP-002 -> CMP-002B
                              |\-> APP-REDESIGN-001 (each rejected call site)
                              |\-> APP-NATIVE-SUBSET-001 -> SWF-009 -> E2E-003U
                              |\-> APP-NATIVE-AGGREGATE-001 -> SWF-008 -> E2E-003A

SWF-000 -> SWF-002 -> SWF-014 -> SWF-004 -> SWF-006 -> APP-NATIVE-* -> E2E-003NQ
             |                       |
             +-> APP-TXN-ADR-001 ----+-> SWF-005B/APP-NATIVE-SINK-001 (only if atomic)

CMP-003 -> CMP-003A -> CMP-004 -> CMP-004A + CMP-004B -> CMP-005 -> CMP-006 -> E2E-003CQ
APP-OWN-001 -> APP-MIG-001 -> MIG-002/MIG-002B -> MIG-004 -> OPS-009
```

These are amendments to the canonical task graph; they should only become launchable after
`PLAN-001` generates identities, scenario hashes, ownership and exact gates.

## Source/identity notes

- Circuits checkout reviewed at `0f94a029dc82a29c6f0f36ff82d262f49572c232`.
- `electric-sync-swift` and `indexed-mighty-prod-ecs-proof` were clean at the SHAs above.
- `notes/FINDINGS.md` in the ECS checkout has at least one stale claim about subset API support;
  executable app code and the frozen app hash must outrank that prose during `CMP-000`.
- No sibling test result was promoted to Circuits evidence; all observations above are differential
  review inputs only.
