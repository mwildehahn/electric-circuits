# Swift app E2E/TDD contract map

Status: reviewed scenario input, integrated into notes 18/24. Canonical task authority remains note
18; this file owns Given/When/Then rationale and proposed test placement.

**Purpose.** This is the high-level, black-box test plan for two independently shippable
Swift lanes:

- **`COMPAT_V1`** — an app-owned `CircuitsV1HTTPProvider` plus an authenticated,
  allowlisted gateway that serves the existing `electric-sync-swift` collection API.
- **`NATIVE_CORE`** — the proposed `ElectricCircuitsSwift` client and its optional transactional
  local-cache sink.

The test subject is the complete path, not an actor or a message mapper in isolation:

```text
Swift package view or app-owned real local cache
        <-> authenticated gateway <-> Circuits engine + durable-streams
                                         <-> PostgreSQL 18
```

These tests are deliberately insensitive to an internal refactor of the gateway, provider,
Swift actor, or engine. They assert externally observable requests, server responses, durable
lifecycle effects, cache/observer effects, and the authoritative PostgreSQL result after a
deterministic barrier.

## Scope and release profiles

Use an explicit profile per test rather than allowing a green shape suite to imply support for
every feature.

| Profile | Client under test | Required in first release | Excluded/rejected behavior |
| --- | --- | --- | --- |
| `COMPAT_V1` | pinned vendored app build + `CircuitsV1HTTPProvider` through its gateway | Yes, but only admitted templates | DNF/tags, `changes_only`, source-transaction observer requirement, SSE-only path, `order`/`limit`, subset/progressive load, unsupported key/scalar codec |
| `NATIVE_CORE` | `ElectricCircuitsSwift` shape subscription with in-memory view/independent checkpoint store | Only after a named product ADR/use case | Electric tags/DNF, GRDB sink, anonymous release, raw durable-stream admin API |
| `NATIVE_REPLICA_SINK` | native shape plus the app's atomic transactional sink | Only when native data is materialized locally | generic merge/deletion policy; the app manifest owns this |
| `NATIVE_AGGREGATE` | native aggregate subscription | Only if inventory names a consumer | implicitly treating an aggregate as a row shape |
| `NATIVE_SUBSET` | native subset page + base-feed merger | Only after the visibility-fence contract exists | arbitrary multi-order/offset-live shape emulation |
| `MIGRATION` | old app provider and candidate in isolated generations | Required before an app template is cut over | cursor/handle/tag state transfer between lanes |

The current app evidence is important: it has model-specific OpenAPI providers, GRDB model
writers and explicit dependent cleanup (for example `User+ElectricCollection.swift`), not a
generic URL configurable provider. The compatibility provider is therefore an adapter at the
same `HTTPClientProvider` / `HTTPStreamClientProvider` seam, and the gateway owns template
selection, auth, SQL/AST construction and Circuits credentials. It is not valid to point the
app at raw `/v1/shape` and call that a compatibility test.

The app checkout must first be frozen. The inspected candidate uses a materially customized vendored
local package at `ios/Index/LocalPackages/ElectricSync`; it is not qualified by the sibling
`../electric-sync-swift` version or tests. Every `COMPAT_V1` result records app commit, vendored
subtree content hash, any proven upstream-base/patch provenance, Circuits and gateway digests, schema
migration/semantic epoch, and PostgreSQL 18 digest. A sibling-package result is separate conformance
unless the exact patch/rebase relationship is proven.

## Test topology and deterministic barriers

### PostgreSQL 18 is the E2E baseline

Run this suite against the same PostgreSQL 18 major version as production. The repository's
current Docker compose files name `postgres:16`, so the E2E harness must make the image/tag an
explicit required input and have a `postgres18` profile; it must not silently fall back to the
existing PG16 container. PG16 may remain a compatibility matrix job, but cannot be the release
evidence for this work.

Each test gets a fresh database/schema, logical slot/publication, durable-streams namespace,
gateway tenant, and cache directory/database. Tests can then run in parallel. A test that must
restart only one component keeps those fixtures but uses names unique to the test UUID.

### Required harness seams

Propose one reusable cross-repository harness rather than duplicating stack orchestration in every
Swift test target.

| Seam | Responsibility | Observable barrier, not a sleep |
| --- | --- | --- |
| `Postgres18Fixture` | starts a clean PG18 schema; executes named transactions and oracle queries | `commit(_ journalEntry) -> SourceCommitID`; SQL `COMMIT` success is the source barrier |
| `CircuitsStackFixture` | starts DS, engine and a test-mode authenticated gateway; can restart one component | `/ready` plus gateway test control reports expected epoch/config; restart waits for ready, not elapsed time |
| `GatewayProbe` | sends only real app/native requests and records redacted request/response facts | `awaitRequest(template:, phase:)`; records request ID, template ID, status, headers allowed by contract |
| `CausalFence` | produces source commit, server-drained and per-target app-application receipts | `awaitApplicationReceipt(commitID, principal:, template:, generation:, backend:)`; cache receipt commits after all prior target effects |
| `MaterializationProbe` | reads native core's minimal view or, for app profiles, the actual GRDB/shadow DB through its normal reader and observer | `awaitVisible(generation:, expected:)`; observer effects only where the selected profile promises them |
| `LifecycleProbe` | controls foreground/suspend/background-expire/offline/online/credential responses without faking application state | `awaitState(.tailOpened/.tailStopped/.closeFinished)` and background completion callback |
| `ServerLifecycleProbe` | authenticated test-only read of claims/stream existence/reset state, never a production client API | `awaitClaimCount`, `awaitReleased`, `awaitReplacement`; use only to prove lifecycle effects that no UI read can expose |

`SourceCommitID` is allocated by the harness and its sentinel is the last statement in the same
PostgreSQL transaction as the mutation. Then obtain `server.drainedThrough(id)` and a target receipt
after a read initiated following that server receipt and the actual cache/fold transaction commits.
A separately tailed sentinel, `up-to-date`, an Electric offset, a Circuits position, and row-map
equality are not target application fences. If `COMPAT_V1` cannot carry an in-lane receipt, quiesce
writes and require an explicit per-template caught-up/cache-commit receipt.

Every barrier has a generous, diagnostic deadline, but no test uses `Task.sleep` to create order.
Timeouts report the last source commit, redacted request IDs, component readiness, cache generation,
last external semantic boundary, and a diagnostic trace. Delay/reorder tests use gates controlled by `GatewayProbe` or the fixture
transport; they release a particular request/page only after its predecessor is observed.

### Oracles

At each named source barrier, assert all applicable oracles:

1. **Source oracle:** an independently authored SQL/projection/key definition held at the journal
   prefix or folded through `SourceCommitID`; it never imports the production template compiler.
2. **Visible-cache oracle:** the app's normal read/query sees exactly one remote generation plus
   the documented optimistic overlay. It must never read candidate rows before promotion or old
   remote rows after a completed promotion.
3. **Effect oracle:** public observer-visible states obey the selected profile's allowed prefixes,
   replay, generation and reset rules. Exact operation/callback order is diagnostic or a focused
   adapter-model assertion; one observer batch is required only by `NATIVE_TXN_ATOMIC`.
4. **Lifecycle oracle:** exactly the promised claim/tailer/cursor owner exists; a named retry has
   one semantic server-side effect, and close/release eventually leaves none.
5. **Security oracle:** a cross-account request has no cache effect, no accepted template access
   and no server claim. Recorded diagnostics contain no bearer token, signed URL, raw predicate or
   row value.

Do not compare raw offsets, handles, UUIDs, timestamps, or rows after a catch-all normalization.
The comparator has a checked-in, per-template normalizer allowlist. It may normalize documented
transport representation only; it may not erase insert/update/delete, field-presence, ownership,
generation, reset, or observer events. Test fixtures retain synthetic values and causal facts;
production-shaped evidence retains keyed hashes/request IDs only.

## Proposed files and suites

Names below are proposed locations. They keep fast mocked unit/state-machine tests in their
existing packages while making real-stack tests obvious and independently runnable.

```text
electric-circuits/
  packages/acceptance/src/swift/
    src/postgres18-fixture.ts                 # starts PG18 + stack, migrations, sentinel barrier
    src/gateway-probe.ts                      # test-only, authenticated request/lifecycle probes
    src/contract-fixtures.ts                  # seed rows, canonicalization, journals
    src/stack.ts                              # ports/namespaces/component restarts
  test-fixtures/swift-contract/
    eligible-template-manifest.json
    ownership-manifest.json
    scalar-key-corpus.json
    lifecycle-journals.json
    native-protocol-fixtures.json
    subset-aggregate-fixtures.json

ElectricCircuitsSwift/                         # proposed independent repository/package
  Tests/ElectricCircuitsE2ETests/
    Support/NativeAppHarness.swift
    Support/TransactionalSinkProbe.swift
    NativeShapeLifecycleE2ETests.swift
    NativeSinkRecoveryE2ETests.swift
    NativeAuthAccountE2ETests.swift
    NativeAggregateE2ETests.swift              # profile-gated
    NativeSubsetE2ETests.swift                 # profile-gated
    NativeCodecContractE2ETests.swift

indexed-mighty-prod-ecs-proof/                 # candidate real-app integration target
  ios/Index/LocalPackages/ElectricSync/Tests/ElectricSyncTests/CircuitsContract/
    CompatV1ResponseCheckpointTests.swift       # 199/200/201/400/max chunk-crash matrix
  ios/Index/LocalPackages/Services/Tests/ServicesTests/Electric/Circuits/
    CircuitsV1ProviderAppE2ETests.swift
    CircuitsCacheOwnershipAppE2ETests.swift
    CircuitsCutoverRollbackAppE2ETests.swift
  ios/Index/.../CircuitsLifecycleUITests/       # app-hosted scene/kill/keychain/protected-data cases
```

Native-core package suites use a small in-memory view and independent checkpoint store—no GRDB import
or runtime assertion. Optional sink and candidate-app suites use the actual app DB/reader and exercise
every admitted production template/ownership pattern. Provider/GRDB/auth cases live in existing
ServicesTests; OS suspension/kill/scene/keychain/protected-data cases require an app-hosted UI test
plan. Sibling `electric-sync-swift` tests are separate conformance unless provenance is proven.

Use Swift Testing (`@Suite`, `@Test`, `#expect`, `#require`) with tags such as `.e2e`,
`.postgres18`, `.compatV1`, `.native`, `.migration`, `.aggregate`, and `.subset`. Tests get
isolated namespaces and can therefore run in parallel; use `.serialized` only for a physical
device lifecycle suite that cannot receive an isolated simulator/app container. Await an event
barrier/`AsyncSequence` confirmation rather than inspect a callback after the test returns.

## Reusable contract scenarios

The scenario IDs are the stable contract inventory. A suite can parameterize them by the
eligible-template manifest, but a failure must preserve the case ID, template ID and fixture seed.

### A. Bootstrap, live data, reset and restart

| ID | Given | When | Then | Profiles / proposed suite |
| --- | --- | --- | --- | --- |
| `SYNC-001` initial snapshot is a complete starting point | PG18 contains matching and nonmatching rows; cache generation is empty | start subscription and hold a source transaction across snapshot setup; commit insert/update/delete/move-in/move-out before the shared source fence | caught-up cache equals SQL oracle, contains each matching key once, contains no nonmatch, and reports readiness only after its initial apply commits | `COMPAT_V1`, `NATIVE_CORE`; `CompatV1BootstrapLiveE2ETests`, `NativeShapeLifecycleE2ETests` |
| `SYNC-002` live absolute changes converge | a caught-up subscription with three rows | commit one transaction containing value update, predicate exit, predicate entry and delete; obtain its target application receipt | final cache equals SQL; exit/delete are absent; update keeps one key; any duplicate replay is safely idempotent. One observer batch is asserted only by `NATIVE_TXN_ATOMIC` | `COMPAT_V1`, `NATIVE_CORE`, sink |
| `SYNC-003` restart during live tail | a caught-up stream and a committed mutation held behind a gateway read gate | restart engine, DS, or gateway one at a time, release the gate, reconnect | no committed row is lost; either normal continuation or exactly one documented reset/rebootstrap occurs; final cache equals SQL after a fresh barrier | `COMPAT_V1`, `NATIVE_CORE`, `NATIVE_REPLICA_SINK` |
| `SYNC-004` server reset/retirement is a rehydrate, not stale continuation | active subscription; schema retirement/known replacement trigger or test fixture response | deliver reset/replacement | old cursor/handle/feed is never reused as if valid; cache reaches a complete new generation before it is visible; claim/tailer for old generation ends | `COMPAT_V1`, `NATIVE_CORE`, `NATIVE_REPLICA_SINK` |
| `SYNC-005` v1 handle expiry and server restart | `COMPAT_V1` snapshot and handle are established | expire the handle or restart gateway/engine; next positioned request receives 409/must-refetch | provider atomically discards the candidate v1 generation and bootstraps with `offset=-1`; it never sends an old handle as a new snapshot identity | `COMPAT_V1` only |
| `COMPAT-001` response-chunk checkpoint safety | vendored compatibility response contains 199/200/201/400/max admitted messages with updates/deletes/move-outs/missing-vs-NULL | terminate/cancel after every committed local chunk and redeliver the response | persisted offset/handle/cursor never names uncommitted cache effects; restart replay is safe and final fenced cache equals SQL | `COMPAT_V1` hard admission condition |

For `SYNC-002`, hold the PostgreSQL source commit and assert no pre-commit publication; after commit,
core/compatibility may expose only states allowed by their event/response application policy and must
reach fenced SQL. Require no intermediate materialization only in `NATIVE_TXN_ATOMIC`. Do not assert a
particular HTTP page or callback count.

### B. Idempotent lifecycle, reconnect, cancellation and background

| ID | Given | When | Then | Profiles / proposed suite |
| --- | --- | --- | --- | --- |
| `LIFE-001` duplicate create/renew has one claim | a named native subscription, or a v1 handle being long-polled | lose the successful response and retry the same create/renew; race a normal renewal | native: same subscription returns the authoritative handle or documented replacement and only one claim remains. v1: one per-handle poll is observed; no parallel provider poll corrupts the cursor | `COMPAT_V1`, `NATIVE_CORE` |
| `LIFE-002` close is one-shot under races | tail and renewal are open | concurrently close twice, cancel, and release a late renewal | local tasks stop and late response cannot reopen/persist; native named release has its typed outcome; compatibility server claim may persist only through its declared lease deadline and is then observed gone | `NATIVE_CORE` and `COMPAT_V1`, profile-specific |
| `LIFE-003` idle long poll is benign | live subscription at current tail | complete real gateway/v1 long poll with 204 (or native idle boundary) then commit a row | 204 yields no row mutation and exactly the permitted repoll/reconnect; next source commit arrives once; no tight loop | `COMPAT_V1`, `NATIVE_CORE` |
| `LIFE-004` offline and reconnect | caught-up cache and an in-flight read | force connection loss before headers, in body, after server commit/before response, and after first applied event; restore network | client follows documented retry/backoff/`Retry-After`, never preflights reachability, converges after barrier; request transcript proves non-idempotent work was not duplicated | `COMPAT_V1`, `NATIVE_CORE`, `NATIVE_REPLICA_SINK` |
| `LIFE-005` suspension/background expiration | tail active and a finite checkpoint requested | app-host/device suspends; controlled background task expires; resume or terminate/relaunch | no assumption timers/socket ran; local work cancels; persisted record recovers; cache converges; server claim outcome follows the profile's named-release or lease contract | `COMPAT_V1` and selected native app integration; app-host/device tag |
| `LIFE-006` cancellation at every external await | gates exist at request send, headers, body page, cache transaction, cursor persist, renew, release and reset | cancel at each gate then relaunch/retry | exactly the documented atomic result is visible; no half generation, orphan tail, duplicate effect or cursor advanced beyond committed cache | `COMPAT_V1`, `NATIVE_REPLICA_SINK`; core external-I/O cuts are also `NATIVE_CORE` |

`LIFE-006` stable E2E cuts use external semantic boundaries: request possibly landed, response lost,
source incomplete/complete, local transaction uncommitted/ambiguous/committed, process killed,
network severed, credential expired and lease boundary. Implementation-specific header/parser/
statement awaits remain a versioned focused state-machine manifest. Both tiers announce arrival; no
test uses timing to manufacture the race.

### C. Authentication and account/cache isolation

| ID | Given | When | Then | Profiles / proposed suite |
| --- | --- | --- | --- | --- |
| `AUTH-001` gateway allowlisting and credentials | two accounts/tenants, a valid template and an invalid template/parameter | request allowed and disallowed combinations, 401, 403, expired credential then refresh | only server-owned template IDs/typed parameters reach Circuits; denied request creates no claim/cache effect; refresh resumes only the authenticated account; logs/transcript are redacted | `COMPAT_V1`, `NATIVE_CORE`; `CompatV1AuthCodecAdmissionE2ETests`, `NativeAuthAccountE2ETests` |
| `AUTH-002` account switch is a hard generation boundary | account A has active tail/cache and a delayed A response; account B has different rows including same PK | run the real app auth teardown/switch to B, then release delayed A; relaunch | only B is observable; delayed A cannot write; A data follows the frozen app privacy policy (currently destroyed/inaccessible); no credential/URL/cursor crosses account | `COMPAT_V1` and selected native app integration |
| `AUTH-003` logout clears authority and applies the app privacy policy | active generation; rollback-within-principal is modelled separately | logout during bootstrap, tail, reset and close through the real auth path | no authenticated tail continues; account cache/metadata/keychain state is inaccessible/deleted as specified; logout never preserves a warm rollback cache by assumption | `MIGRATION`, `COMPAT_V1`, and selected native app integration |

### D. Keys, scalars, field presence and deletion semantics

| ID | Given | When | Then | Profiles / proposed suite |
| --- | --- | --- | --- | --- |
| `CODEC-001` manifest-selected scalar round-trip | admitted template and only support-manifest scalar kinds, including missing-vs-NULL and numeric/time/bytes/JSON forms when actually selected | PG18 snapshot/live round trip | generated model/value equals declared schema; unsupported type is rejected before feed, never silently Float/Text-coerced | `COMPAT_V1` and native with tagged fixtures |
| `CODEC-002` tagged key grammars delete exactly one row | Electric grammar fixtures (`/`, `.`, `_`, quotes/escapes/schema/table) and separate native opaque composite fixtures (U+001F/backslash), including normalization/collision mutants | snapshot/update/delete generated keys | zero collisions/wrong-row cleanup; no lane invokes the other key grammar | `COMPAT_V1` and `NATIVE_CORE`, using separate corpus tags |
| `CODEC-003` projection merge is explicit | seed a full row then a projection omitting one non-PK field | apply snapshot/live partial update and delete | app's model provider obeys its field-presence rule; omitted never means null; delete invokes documented dependent cleanup; unsupported partial projection is rejected before a network request | `COMPAT_V1`, `NATIVE_REPLICA_SINK` |
| `CODEC-004` capability admission fails closed | each unsupported collection mode/template (DNF/tags, changes-only, subset/progressive, ordering/limit, transaction-observer requirement) | attempt to select `COMPAT_V1` 10 times per mode | no gateway/Circuits request is emitted, no cache state changes, and a typed reason is recorded | `COMPAT_V1` only |

The selected scalar cases are shared semantic fixtures across Rust, TypeScript, the gateway and
Swift, but each fixture is tagged with the profiles and manifest type that actually support it.
Key syntax is deliberately not one shared grammar: Electric structured keys and native opaque/
composite keys have separate tagged corpora and collision rules. It is not enough to decode JSON
fixtures in Swift: each selected case must traverse PG18 -> engine -> gateway -> typed decoder and,
for app materialization profiles, the real cache. An unsupported composite-key case stays rejected;
it is never normalized into a fake single-key success.

### E. Cache ownership, shadowing, cutover and rollback

| ID | Given | When | Then | Profiles / proposed suite |
| --- | --- | --- | --- | --- |
| `OWN-001` shadow lane is invisible | legacy and Circuits feed the same eligible source trace into isolated local generations; candidate starts first/last and is deliberately delayed | complete common fence before promotion | user-visible reader selects legacy only; candidate state may differ only while barrier is blocked; after barrier comparator reports canonical rows *and effect/ownership trace*, not just final map | migration; `CircuitsV1ProviderAppE2ETests` |
| `OWN-002` no cross-owner delete | two selected feeds/generations share a destination model/key and one moves out/closes/resets | apply delete/move-out/close in all orders | a row remains visible while any documented owner still owns it; only the owner manifest's final release removes it; optimistic overlay follows its explicit publication evidence | `MIGRATION`, `NATIVE_REPLICA_SINK`, `COMPAT_V1` where applicable |
| `OWN-003` atomic promotion under crash | complete candidate bootstrap but hold the local promotion transaction at every DB write | crash/cancel before and after row copy/swap, ownership metadata, cursor state and reader-generation marker; relaunch | normal read exposes either complete old or complete new generation, never a mix; no Electric cursor/tag/handle becomes a Circuits/native resume token | migration; `CircuitsCacheOwnershipAppE2ETests` |
| `CUT-001` per-template cutover | shadow has passed common fence for one template while other templates stay legacy | enable its kill-switch/promotion and commit writes around drain/close/promotion | exactly one fresh authoritative remote generation is visible; feature flags affect no other template; old and new subscribers cannot both delete the visible row | `MIGRATION` with `COMPAT_V1` or `NATIVE_REPLICA_SINK` |
| `ROLL-001` warm rollback freshness | manifest says legacy is warm and continuously caught up | cut over, make writes, trigger rollback at deterministic drain/close/promotion/crash/restart points | old generation is promoted only after its common fence; it is fresh and within the declared doubled-load budget; no candidate ownership is left visible | migration |
| `ROLL-002` cold rollback freshness | manifest says legacy is cold | cut over, make writes, trigger rollback at the same points | retained old cache stays invisible; a fresh old-service `offset=-1` bootstrap plus common fence completes into a new generation before it becomes authoritative; RTO clock starts at this rehydrate | migration |

`OWN-001` through `ROLL-002` are blocked on a checked-in ownership manifest for every registered
model, destination-table writer/reader, optimistic overlay, projection merge, delete/dependent
cleanup, generation strategy and rollback mode. A generic library test cannot make these app
product decisions. Add a CI check that an unclassified registered model, writer or reader fails
the manifest build.

### F. Optional native surface contracts

| ID | Given | When | Then | Profile / suite |
| --- | --- | --- | --- | --- |
| `TXN-001` per-stream observer transaction | negotiated eligible stream and oversized/chunked source transaction | cut before final marker/application and restart | one complete per-stream observer batch and transaction checkpoint after final marker, or typed reset; no cross-stream claim | `NATIVE_TXN_ATOMIC` only |
| `NATIVE-001` sink acknowledgement/crash | native shape with one atomic transactional app sink and source journal | terminate before transaction, during it, on ambiguous commit result, and after committed return; relaunch | no loss; safe replay is idempotently absorbed; row effects+ownership+generation+checkpoint are atomic; second cursor owner rejected | `NATIVE_REPLICA_SINK`; `NativeSinkRecoveryE2ETests` |
| `AGG-001` aggregate retraction/precision | empty and nonempty sets containing nulls and bigint values | count/sum/avg/min/max through insert/update/delete/predicate exit/restart/replacement | `{value,n}` has the documented null/precision semantics and equals SQL after each barrier; aggregate reset rehydrates rather than continues stale | `NATIVE_AGGREGATE`; `NativeAggregateE2ETests` |
| `SUBSET-001` page/feed seam | page request is held while matching live events include update/delete/reorder | release page and feed in both orders; load more with a tie/NULL/Unicode cursor; force lapse/crash | visible page plus feed equals SQL oracle; LSN watermark and tombstones prevent resurrection/duplication; unsupported order/predicate/page rejected before network work | `NATIVE_SUBSET`; `NativeSubsetE2ETests` |

`NATIVE-001` must remain red/blocked until `SWF-000` defines who owns acknowledgement and cursor
persistence. Yielding an `AsyncSequence` element is not acknowledgement. `SUBSET-001` must remain
blocked until the server offers the documented snapshot visibility fence; an LSN obtained by an
unfenced page is not enough.

## TDD order: make the most valuable end-to-end behavior red first

1. **Freeze and admit.** Check in the exact revision manifest, eligible-template inventory,
   ownership manifest and release profile selector. Add red `CODEC-004` tests showing that every
   not-yet-admitted app collection is refused before network work. This prevents accidental broad
   `/v1` rollout.
2. **Build the PG18 real-stack harness and common barrier.** Start with a deliberately delayed
   backend test for `OWN-001`: it must block comparison rather than report equality. Add an
   intentional divergence mutation and prove it is detected. This is the prerequisite for any
   meaningful shadow/cutover claim.
3. **Compatibility happy-path vertical slice.** For one small, statically-simple admitted app
   template, write `SYNC-001` red, then `SYNC-002` and `LIFE-003`. Implement only the provider and
   gateway behavior needed to turn the tests green; preserve the existing Electric default path.
4. **Compatibility checkpoint safety before breadth.** Make `COMPAT-001` red first and prove the
   199/200/201/400/maximum response matrix across every committed local chunk. Then add `SYNC-005`,
   `LIFE-004`, `LIFE-006`, `AUTH-001`, `AUTH-002`, `CODEC-001` and `CODEC-003` before enabling a
   second production template. Parameterize
   the resulting suite over each admitted template only after its codec and ownership facts are
   supplied.
5. **Generation/cutover proof.** Freeze `OWN-001`–`OWN-003`, `CUT-001`, and the selected warm/cold
   `ROLL-001`/`ROLL-002` as red public contracts before implementing ownership/promotion. Turn the
   unchanged contracts green, then run their immutable candidate qualification before clone
   rehearsal or production shadow. Do not retrofit this after dual-read is enabled.
6. **Native shapes independently.** In the new package, make `SYNC-001`, `SYNC-002`, `LIFE-001`,
   `LIFE-002`, `SYNC-004` and `AUTH-002` red against the versioned native gateway contract. Native
   tests must not initialize, link to, or mutate the compatibility cache/lane.
7. **Native materialization only after acknowledgement ADR.** Freeze `NATIVE-001`, ownership and
   `LIFE-006` red; implement the reusable one-transaction sink contract, then the separately owned
   real-app adapter. Turn both tiers green without a post-commit checkpoint gap. This is the gate for
   native local-cache migration.
8. **Only inventory-driven extensions.** Add `AGG-001` and `SUBSET-001` only when a product ADR
   names their consumer and server contract. Their profile-gated suites do not block a
   compatibility-only or shape-only release.
9. **Scale after semantic coverage.** Replay deterministic journals: initially 100 per admitted
   template; then the release gate's high-volume shadows/lifecycle loops. Preserve seed and first
   divergence. Large counts validate the already-defined contract; they do not substitute for the
   red examples above.

## Execution and evidence gates

- Every E2E command prints the PG18 image digest, app/package revisions, gateway/engine/DS digests,
  profile, fixture seed and namespace. It fails closed if any required component is missing or not
  ready.
- `COMPAT_V1` runs the actual gateway/provider and a real local GRDB cache. A URL-protocol test is
  useful below it, but cannot satisfy this gate.
- `native` runs only against the versioned native public gateway contract. It must not reverse
  engineer tRPC internals or use raw durable-stream administration as its public client path.
- Faults are controlled gates: withheld response/page, injected status, connection close, process
  restart, lifecycle transition, cache transaction cut point and source sentinel. No arbitrary
  sleeps and no broad catch/retry that hides an expected reset.
- The failure artifact contains a replay command, profile/template/case ID, synthetic source journal,
  canonical source/cache/effect hashes, barrier attestations and redacted request IDs. It excludes
  credentials, signed URLs, raw predicates and production row values.
- Keep suite pass/fail separate for `COMPAT-RC` and `NATIVE-RC`. A compatibility release neither
  requires nor claims native aggregate/subset coverage; a native release neither depends on nor
  mutates the compatibility provider/cache.

This map intentionally leaves actor-level scheduling, parser fuzzing and pure codec properties to
their fast unit/conformance suites. Its job is to make the externally observable promises hard to
regress while the middle layers are free to change.
