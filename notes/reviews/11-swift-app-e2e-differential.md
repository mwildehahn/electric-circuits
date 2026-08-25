# Swift/package/real-app E2E differential review

Date: 2026-08-23

Scope: independent differential review of `notes/23-swift-app-e2e-tdd-map.md`,
`notes/24-postgres18-and-e2e-tdd-addendum.md`, and
`notes/18-production-readiness-spec-reviewed.md`, checked against the current Circuits engine, the
sibling `../electric-sync-swift` package, and the available real app
`../indexed-mighty-prod-ecs-proof`. This is an inspection-only review. It changes no product code and
does not report test execution.

Inspected revisions:

- `electric-sync-swift`: `6bdde65a7c234371da829b0de24af12e00266fa8` (`0.1.12`).
- real app: `2168965405d7d385f2a0c7b470ea035de1c6cc89`.

## Verdict

The addendum correctly separates core checkpoint safety from optional source-transaction observer
atomicity, and the proposed real-stack/package/app layering is directionally right. It is not yet a
safe executable E2E plan. Four issues are release-blocking:

1. the plans test the wrong Swift baseline: the real app vendors a materially customized local
   `ElectricSync`, not the claimed upstream `0.1.8` package;
2. the common fence proves server ingestion, not target app-cache application;
3. current app-compatible application chunks responses at 200 messages while assigning the final
   response cursor to every message, creating an untested early-checkpoint/loss window; and
4. the native-core lane is polluted by app/GRDB/migration prerequisites and one crash matrix
   contradicts the atomic-sink contract.

The exact observer-operation trace in note 23 also accidentally promotes optional transaction
atomicity to compatibility/core acceptance. Account-switch, background, release, key-codec, and
schema cases need profile-specific oracles derived from the actual app and wire, not one shared
outcome.

## Severity-ranked findings and concrete corrections

### P0 — `CMP-000` and the proposed test placement target the wrong Swift baseline

Notes 18 and 23 say the candidate app pins ElectricSync `0.1.8` while the sibling checkout is
`0.1.12`. The inspected app has no such external pin. It declares `ElectricSync` through local path
dependencies (`ios/Index/LocalPackages/Services/Package.swift:35`, and many other packages) and
vendors the implementation at `ios/Index/LocalPackages/ElectricSync`. That source is materially
different from sibling `0.1.12`: most core files differ, each tree has files absent from the other,
and the vendored package has app-only dependencies (`Dependencies` and `IndexFoundation`). Evidence
from a new test target under sibling `../electric-sync-swift/Tests` therefore does not qualify the
code the app runs.

The placement is also not executable as written. Sibling `Package.swift` declares only
`ElectricSyncTests`; merely creating `Tests/ElectricSyncCircuitsE2ETests` does not make `swift test`
compile or run it. Model-specific OpenAPI providers, `ElectricMessageCoder`, GRDB writers, auth, and
scene lifecycle live in the app's `Services` and app targets, not in the generic sibling package.
Hostless `ServicesTests` cannot prove process termination, suspension, protected-data transitions,
or real `URLSession` background behavior.

**Required corrections:**

- Rewrite `CMP-000` to pin three separate artifacts: app commit, vendored ElectricSync subtree
  content hash, and any asserted upstream base/patch provenance. Do not infer provenance from a
  package version.
- Qualify the vendored app package for `COMPAT_V1`. If upstream qualification remains useful, call it
  a separate conformance result and prove the exact patch/rebase relationship before transferring
  evidence.
- Keep pure generic-library fixtures in sibling `ElectricSyncTests` only after declaring the target
  in `Package.swift`. Put provider/GRDB/account tests in the app's existing `ServicesTests`; put
  suspension, kill/relaunch, scene, keychain, protected-data, and UI-observer cases in an app-hosted
  XCUITest target/test plan.
- Use the canonical monorepo launcher name `packages/acceptance` from notes 18/24. Remove or alias
  note 23's competing `packages/swift-e2e-harness`; one process-control protocol and artifact layout
  must serve all runners.

### P0 — the common source fence is not yet a common client-application fence

All three documents define `SourceCommitID` by inserting a sentinel row in the same PostgreSQL
transaction and waiting until legacy and Circuits have consumed it. That is necessary but
insufficient for app-cache shadow comparison. If the sentinel uses a separate control table/feed,
its client task can apply and persist the sentinel while a target template's independent response,
GRDB transaction, observer queue, or reconnect replay is still behind. The engine's applied-LSN
barrier proves output was sequenced/landed; it does not prove each app cache committed all effects for
that template. A comparator can consequently compare different prefixes and label a timing race as
either equality or divergence.

**Required corrections:**

- Define a per-`(principal, template, generation, backend)` application fence, not only a backend
  ingestion fence.
- The fence must travel in the same ordered application lane as the target template and be persisted
  atomically after all prior target effects. A gateway-issued opaque feed fence tied to
  `SourceCommitID` is acceptable if its target effects and local checkpoint/fence commit together.
- If `COMPAT_V1` cannot carry that fence in the same response/application lane, comparison requires
  quiesced writes plus an explicit per-template caught-up boundary; a separately tailed sentinel is
  not evidence.
- `ReplicationBarrier.awaitApplied` should return individual receipts containing template,
  generation, source commit, backend position, local cache transaction/fence ID, and principal.
  `ShadowComparator` must reject missing, stale, cross-principal, or cross-generation receipts.
- Add the counterexample test: deliberately hold the candidate target-cache transaction after its
  sentinel feed applies. Comparison must remain blocked.

### P0 — current 200-message chunking can checkpoint a compatibility response early, but the E2E map never crosses the boundary

The real vendored client applies a response in chunks of 200, each in its own transaction
(`Collections.swift:77,395-440`). `SyncBatch.chunked` preserves `shouldPersistSyncState` on every
chunk while only delaying fetch-completion metadata to the final chunk
(`ElectricSyncClient.swift:53-83`). `SyncBatch.apply` writes the recorded offset/handle/cursor for
each chunk (`ElectricSyncClient.swift:279-293`). Meanwhile app `ElectricMessageCoder` assigns the
same response offset, handle, and cursor to every mapped message
(`ShapeHTTPClientProtocol.swift:203-234` and `389-425`). Therefore the first 200-row local
transaction can persist the response's final cursor. Termination before chunk 2 can resume after
unapplied rows and lose them.

`SYNC-002` uses four mutations, so it cannot see the existing boundary. `LIFE-006` names generic
cursor cuts but does not assert every app cache-transaction boundary of a multi-chunk compatibility
response.

**Required corrections:**

- Add an app-provider task before `E2E-003C` that makes response checkpoint ownership explicit. One
  compatible fix is to attach offset/handle/cursor only to a terminal message/control record, but the
  chosen behavior must be characterized against the incumbent before implementation.
- Add fixed 199, 200, 201, 400, and large-response cases. Terminate/cancel after every committed local
  chunk and assert the persisted compatibility checkpoint never names data not yet applied.
- Include deletes, move-outs, absent-vs-null updates, and duplicate response delivery across the
  boundary. Assert final cache equality and idempotent replay, not one observer transaction.
- Make this a hard `COMPAT_V1` admission condition per template. A template whose maximum response
  cannot be bounded below the chunk threshold is not exempt.

### P0 — `NATIVE_CORE` is not isolated in the dependency graph or test substrate

The profile contract says `NATIVE_CORE` is the independent event-level client with in-memory view
and caller checkpoint, while GRDB sink and transaction-atomic delivery are optional. The task graph
violates that separation:

- `SWF-000` depends on `APP-OWN-001`, pulling app cache ownership into the core acknowledgement ADR.
- Note 23 says all package suites use real SQLite/GRDB, so core acceptance can accidentally depend on
  the optional sink and hide checkpoint/view defects.
- `E2E-003N` depends on `MIG-000`; that path imports incumbent/app migration machinery into the
  supposedly independent native consumer app.
- `NATIVE-001` combines core cuts with “during sink transaction” and “after sink commit/before cursor
  persist.” The latter state must not exist under `SWF-007`, which stores the checkpoint inside the
  same atomic sink transaction.
- `SWF-013`, `TST-006N`, `TST-008N`, and `E2E-003N` do not list generated conditional dependencies on
  `SWF-005B`, `SWF-007`, `SWF-008`, `SWF-009`, and their selected tests. Optional code can therefore
  be published or exercised without its prerequisite evidence.

**Required corrections:**

- Remove `APP-OWN-001` from `SWF-000`; depend only on the native delivery ADR/base protocol. Attach
  `APP-OWN-001` to `SWF-007` and app migration adapters.
- Give core tests a minimal in-memory view plus an independent temporary-file or in-memory
  `CheckpointStore`. No GRDB import/link/runtime assertion belongs to `NATIVE_CORE`. GRDB belongs to
  `NATIVE_SINK` and app E2E.
- Make `E2E-003N` use the `E2E-000` real-stack `SourceCommitID` barrier directly. Only `E2E-004`
  migration acceptance should depend on `MIG-000`.
- Split `NATIVE-001` into core delivery/checkpoint replay and optional sink atomicity. Sink cuts are
  before transaction, during transaction, ambiguous commit/result loss, and after committed return;
  never a separate post-sink-commit cursor persist.
- Generate exact conditional edges from the selected profile manifest. Normalize note 23 aliases
  (`compat-v1`, `native-shape`, and so on) to canonical IDs (`COMPAT_V1`, `NATIVE_CORE`,
  `NATIVE_TXN_ATOMIC`, `NATIVE_SINK`, `NATIVE_AGG`, `NATIVE_SUBSET`) so release evidence cannot be
  filed under a different profile spelling.

### P1 — `SYNC-002` and the exact-effects oracle accidentally require optional transaction atomicity and internal behavior

Note 24 and `E2E-001` correctly state the split: every profile must not advance a durable checkpoint
past an incomplete source transaction; only `NATIVE_TXN_ATOMIC` promises one observer batch with no
intermediate public materialization. Note 23 conflicts with that rule. It requires an ordered model
operation/observer-notification trace equal to an allowed trace, says there is no “partial observer
transaction,” and gates a two-phase cache while “the transaction gate” is held.

The current compatibility app deliberately commits each 200-message chunk independently, so GRDB
observers may see source-post-commit prefixes. Requiring one exact operation/notification sequence
also tests implementation order, chunking, and callback scheduling rather than the public cache
contract. A harmless refactor could fail it; a wrong implementation that emits the expected callback
trace could pass.

**Required corrections:**

- For `COMPAT_V1` and `NATIVE_CORE`, assert: no pre-source-commit publication; no durable checkpoint
  beyond incomplete source work; generation/principal isolation; every visible state is an allowed
  state/prefix under the declared cache policy; and eventual fenced final cache equality.
- Require exactly one public observer batch/no intermediate materialization only for
  `NATIVE_TXN_ATOMIC`.
- Keep normalized model-operation traces as diagnostic artifacts or focused adapter unit-model tests,
  not cross-implementation E2E equality. Compare public cache rows, ownership metadata, checkpoint,
  generation, observer-visible states, and resource outcomes.
- Reword `SYNC-002`'s “transaction gate” as a source-commit gate for all profiles; add a distinct
  optional observer-release gate only to `NATIVE_TXN_ATOMIC`.

### P1 — account, mobile lifecycle, and claim-release oracles are not profile/app accurate

The available app has stronger destructive logout semantics than the generic matrix assumes.
`AuthService.signOut` calls `beginTeardown`, clears the whole `IndexDatabase`, removes keychain user,
auth, private-space and shape-session values, and clears app-group defaults
(`ServicesAuthLive/Auth/AuthService.swift:557-624`). An A-to-B account transition therefore cannot
assume co-resident A/B generations or that a warm incumbent rollback cache survives logout. Current
scene observers stop work outside `.active`, but unit hooks such as `setForegroundActive` do not prove
OS suspension, kill, protected-data locking, or relaunch.

There is also a transport distinction: compatibility long-poll cancellation stops the local request,
but the Electric adapter claim may remain server-side until its lease/TTL; native named release can
have an explicit release outcome. A universal LIFE oracle that expects no active server resource
immediately after backgrounding/termination would reject valid `COMPAT_V1` behavior.

**Required corrections:**

- Run `AUTH-002/003` through the real auth/session teardown path. Assert A data is inaccessible and,
  under the current privacy policy, destroyed before B becomes observable. Model rollback within an
  authenticated principal separately from logout/account switch.
- Split simulator/app-host cases from package state-machine cases. Package tests inject actor
  cancellation/generation changes; XCUITest/device jobs perform background/foreground, process kill,
  relaunch, protected-data, memory pressure, keychain refresh, and network transitions with
  condition-based waits.
- Scope release expectations by profile: local task/tail stops promptly for all; native named claims
  are released according to the typed close result; compatibility claims may have the exact declared
  lease-expiry deadline and must be proven gone after it. “No task/claim/resource remains” must state
  the observation time and mechanism.
- Add account/principal/generation to every cache, checkpoint, fence, ownership, task, and diagnostic
  key. Assert no stale completion mutates B after A teardown.

### P1 — `COMPAT_V1` eligibility is still hypothetical and cannot use a convenient model as proof

The plans correctly say admission is fail-closed, but note 23's real-app scenarios need at least one
eligible template before `CMP-001`/`APP-OWN-001` have proven one. Current app evidence shows why this
must remain a classification result:

- `Space` cleanup also deletes `SpaceMember` rows, so ownership overlaps another collection.
- eager `SpaceMember` use has ordering and dependencies that include a progressive `User` path.
- `Calendar` appears simpler, but appearance is not eligibility evidence.

**Required corrections:**

- Make `E2E-003C` consume a generated admission manifest and fail/skip-with-approved-N/A if zero
  models qualify; do not hard-code an assumed first template.
- For each candidate, record call-site mode, predicate grammar, ordering/limit, dependency graph,
  projection presence, PK/key codec, update cleanup, delete cascades, overlapping owners, observer
  transaction requirement, and account scope.
- Prove that switching one owner cannot delete or overwrite a row still owned by another feed.
  Ownership/reference-count metadata must commit with row effects and generation/checkpoint state.

### P1 — codec/key tests combine incompatible key formats and promise scalar fidelity the server cannot supply

`CODEC-001` focuses on delimiters such as U+001F/backslash, which are relevant to the native engine's
composite-key codec. The compatibility app parses Electric structured keys whose distinct grammar
uses doubled `/` in values, doubled `.` in schema/table, quotes, and `_` for nil
(`ElectricRowKey.swift:3-43`). One shared corpus can pass the wrong parser.

The native rich schema promise is also ahead of an implementation task. The current engine schema
has only `Int`, `Text`, `Bool`, and `Float`; PostgreSQL `numeric` maps to `Float`, and every other
unrecognized type (UUID, timestamp, JSON, arrays, bytes, and so on) maps to `Text`
(`apps/engine/src/schema.rs:149-154`, `apps/engine/src/pg.rs:363-368`). Swift-only CODEC tests cannot
recover exact decimals or distinguish schema-directed rich kinds after the server has already
coerced them.

**Required corrections:**

- Split `CODEC-001` into an Electric-v1 key corpus and a native opaque-key corpus. Compatibility cases
  must include `/`, `.`, `_`, quotes, empty components, non-ASCII, Unicode normalization variants,
  schema/table components, malformed doubled escapes, and composite PKs. Native cases must include
  U+001F/backslash and treat keys as opaque identity outside codec conformance.
- Add a server/gateway result-schema and codec implementation task before `SWF-002` rich-codec
  acceptance, or explicitly mark decimal/bytes/JSON/arrays/etc. unsupported in the selected support
  manifest. `CODEC-001` must be profile- and type-manifest-driven.
- Preserve absent field versus explicit SQL NULL through gateway decoding, app field-presence
  metadata, update cleanup, GRDB binding, replay, and shadow normalization. Test full-row and partial
  projection separately.

### P1 — migration acceptance and production rehearsal are sequenced backwards

`E2E-004` depends on `MIG-005`, but `MIG-005` is the production-shaped rollback rehearsal and note 24
then recommends completing `E2E-004` before production shadow. This makes the acceptance test depend
on the rehearsal it is meant to qualify. Meanwhile `E2E-003C` includes `OWN`, `CUT`, and `ROLL`
scenarios without depending on the generic cutover/rollback contract, duplicating ownership of the
same release claim.

**Required corrections:**

- Make `E2E-004` depend on the implemented comparator/fence, `MIG-002`, `MIG-002B`, `MIG-003`, and
  `E2E-003C/N`, but not on `MIG-004/005`.
- Make pre-production rehearsal `MIG-004` depend on `E2E-004`, and keep `MIG-005` after `MIG-004`.
  Production shadow `MIG-006` then depends on the completed rollback rehearsal.
- Limit `E2E-003C/N` to client/provider/cache safety. Put cross-backend cutover, shadow comparison,
  rollback freshness, and warm/cold policy in `E2E-004`; alternatively add the exact migration-task
  dependencies if those scenarios intentionally stay in the client suites.

### P2 — fault injection names internal awaits as release-contract behavior

`LIFE-006` requires gates around specific internal stages such as headers, body, cache transaction,
cursor persistence, renew, release, and reset. Semantic I/O and commit boundaries are useful fault
points, but a release test must not require them to remain distinct. In particular the native sink
must not expose “sink committed, cursor not persisted,” and a transport may combine header/body
parsing without changing its public contract.

**Required corrections:**

- Keep an internal cut-point manifest for deterministic unit/state-machine coverage, versioned with
  the implementation.
- Define E2E cuts by externally observable boundaries: request possibly landed, response lost,
  source transaction incomplete/complete, local transaction uncommitted/ambiguous/committed, process
  killed, network severed, credential expired, and lease deadline crossed.
- Pass/fail only on public invariants: no loss, allowed replay/duplicates, checkpoint safety, single
  owner, stale-generation suppression, final fenced cache, and eventual typed release. Internal gate
  counts remain diagnostics.

## Corrected dependency spine

The minimum Swift/app E2E order should be:

1. Freeze the actual app/vendored package (`CMP-000`) and generate eligibility/ownership inventory
   (`CMP-001`, `APP-OWN-001`).
2. Define and implement the per-template app-application fence (`MIG-000` plus gateway/app receipt
   support) and fix/characterize compatibility response checkpointing across the 200-message chunk.
3. Land the single `packages/acceptance` launcher/control protocol and base scenario DSL.
4. Land generic compatibility package tests, then app `ServicesTests`, then app-host/device lifecycle
   tests (`E2E-003C`).
5. In parallel, land `NATIVE_CORE` protocol/core/checkpoint tests without APP-OWN, GRDB, or MIG
   dependencies; then the two clean consumer apps (`E2E-003N`). Add selected native optional modules
   only through generated conditional edges.
6. Run fenced cross-backend ownership/cutover/rollback acceptance (`E2E-004`).
7. Only then run pre-production shadow/rollback rehearsal (`MIG-004`, `MIG-005`) and production shadow
   (`MIG-006`).

## Explicit no-finding areas

No material issue was found in these parts of the three documents, subject to the corrections above:

- Note 24 and `E2E-001` correctly scope durable incomplete-transaction checkpoint safety to every
  profile and one observer batch/no intermediate materialization to `NATIVE_TXN_ATOMIC` only.
- The no-sleep rule, deterministic announced gates, condition-based waits, per-test isolation, and
  diagnostic-only deadlines are correct.
- `COMPAT_V1` is correctly fail-closed, restricted to inventory-proven eligible templates, and routed
  through a gateway-owned allowlist rather than exposing raw `/v1` client authority.
- The plans correctly recognize model-specific cleanup and absent-field versus explicit-null
  semantics. The correction is to observe public effects rather than demand one exact internal trace.
- Rollback freshness is correctly strict: stale incumbent state must never become visible, and
  candidate offsets/handles/tags must never be transferred to the incumbent.
- `NATIVE_SUBSET` is correctly gated on an explicit visibility fence, and aggregate/subset support is
  correctly optional rather than a `NATIVE_CORE` prerequisite at the profile-definition level.
- The two clean native consumer apps and the production package's no-GRDB runtime dependency are the
  right independence checks, once their DAG and test substrate stop importing migration/sink work.
- The principal/generation replacement model and actor-owned task intent are sound; the missing work
  is applying those identities consistently to the real auth/cache/fence/resource oracles.

