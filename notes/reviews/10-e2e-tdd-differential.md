# E2E/TDD differential review

Status: independent architecture review, 2026-08-23. No product code was changed.

Reviewed snapshots:

- `notes/24-postgres18-and-e2e-tdd-addendum.md` — SHA-256
  `5477c276cbbbef7d7078b97bc16405bcfcffdae7abc59477b6f9ae55dbbb5ed0`
- `notes/22-server-e2e-tdd-map.md` — SHA-256
  `4a83a0f6ac60c9f859ac2247b2a720303569dfd1f2ff304e29bea7e712258860`
- `notes/18-production-readiness-spec-reviewed.md` — SHA-256
  `0afc417e98951f946316ec6f7ea82dea13a15883fbf7c7300e058a40ef460ad3`

I also inspected the current conformance harness, comparator, CI PostgreSQL bootstrap, Docker
topology, transaction/client protocol tasks, and representative concurrency, large-transaction,
shutdown, lifecycle, and fault tests. `packages/acceptance` does not yet exist, which is consistent
with `E2E-000` being future work.

## Verdict

The proposed architecture has the right overall shape: PostgreSQL is the independent truth source;
the final proof is a real client/app materialization; readiness and metrics are barriers or
operational evidence rather than the data oracle; release qualification uses immutable artifacts and
finite operation/cut manifests; and focused conformance tests remain beneath image-level acceptance.

Two contract issues should be corrected before implementing the shared harness:

1. the notes do not yet define a causal, end-to-end implementation of “every compared path consumed
   `SourceCommitID`,” and note 22 instead describes a different, private-engine drain barrier; and
2. the all-profile transaction language conflicts with the canonical event-level and duplicate-replay
   contracts for `COMPAT_V1` and `NATIVE_CORE`.

The TDD dependency graph, refactor-stable scenario vocabulary, deterministic release-image cut model,
and note-22 topology also need rewiring. These are specification defects, not requests to implement
product code now.

## Severity summary

| Severity | ID | Finding |
| --- | --- | --- |
| P0 | E2E-DR-001 | `SourceCommitID` is not causally observed by the public materialization, and notes 22/24 specify incompatible barriers. |
| P0 | E2E-DR-002 | “No resumable position” and “client sees it once” overstate core transaction/replay guarantees and conflict with the canonical profiles. |
| P1 | E2E-DR-003 | Most E2E qualification tasks depend on completed implementations, so the DAG does not enforce the stated red-first workflow. |
| P1 | E2E-DR-004 | Several supposedly stable public scenario IDs encode current changelog/catalog/DS mechanics; exact cut control also conflicts with release hooks being disabled. |
| P1 | E2E-DR-005 | Note 22 still promotes an engine/API and `native-private` image topology that the canonical gateway-only trust boundary forbids. |
| P1 | E2E-DR-006 | The oracle is not explicitly independent of the production template compiler and its required schema/key semantics are weaker than the canonical protocol. |
| P1 | E2E-DR-007 | The public-image inventory omits several high-value cross-table, component-crash, leadership, revocation, transport-security, and overload cases. |
| P2 | E2E-DR-008 | Timer and post-release-baseline tests need an explicit two-tier clock policy and mechanically enforced no-sleep rule. |

## Findings

### P0 — E2E-DR-001: the source fence does not yet reach the actual client/app observation

Evidence:

- Note 24 requires the data changes and sentinel in the **same commit**, and says each compared path
  must prove it consumed the sentinel (`notes/24-postgres18-and-e2e-tdd-addendum.md:94-103`). Its
  `ReplicationBarrier` repeats that requirement but does not define the public evidence by which a TS
  client, Swift client, cache transaction, or app reader proves it (`:120`).
- Note 22 describes a different operation: update an untracked `__el_sync` row **after** the writes,
  then read `/replication/lsn.sync`, the sequencer position at `changes/<n>`, and `pendingFlips`
  (`notes/22-server-e2e-tdd-map.md:53-55`). Those are current engine/changelog observations, not proof
  that the gateway response was read and the real client/cache applied it.
- The current harness demonstrates the distinction. It creates `__el_sync` separately
  (`packages/conformance/src/harness.ts:300-306`), commits its increment in a new transaction
  (`:443-453`), polls engine replication/changelog/flip internals (`:454-503`), and only afterward
  polls the current client map against a fresh current SQL query (`:521-535`). It does not bind the
  client read or cache commit to the marker.
- Note 24 correctly says that current SQL after unrelated later commits is not a valid oracle for an
  earlier fence (`:100-102`), but neither proposed primitive states how an oracle snapshot/prefix is
  retained while concurrent later commits continue.

Why this matters:

An engine-internal drain is a useful operational barrier, but it is not the public correctness fact.
If the gateway/client is stalled, a pre-existing empty or coincidentally equal materialization can be
compared prematurely. Conversely, if later source commits are visible to the SQL query but not the
checked client prefix, a correct client can be reported divergent. The current `pendingFlips == 0`
mechanism is also coupled to the current deferred-work implementation, so carrying it directly into
the stable black-box language defeats refactor safety.

Actionable correction:

1. Make `pg.transaction`/`commitJournal` allocate `SourceCommitID` in PostgreSQL and update the
   reserved sentinel as the last statement of the **same** transaction as the source changes. The
   harness, not the application role, owns marker allocation. Remove the post-write marker from note
   22's common contract.
2. Split the barrier into three explicitly ordered acknowledgements:
   - `source.committed(id)` — commit and immutable journal prefix exist;
   - `server.drainedThrough(id)` — an adapter-specific operational barrier proves all direct and
     deferred server work through that source prefix completed; and
   - `client.appliedTailAfter(id)` — after the server barrier, the real public feed is read to a
     documented caught-up response initiated after that barrier, and the actual client/cache/app
     reports completion of its fold/cache transaction through that returned tail.
   The last step is essential; map polling alone is not a fence.
3. The SQL side must either hold a repeatable-read oracle snapshot for that prefix or reconstruct the
   expected state by folding the checked-in source journal through `SourceCommitID`. A later-current
   `SELECT` must be rejected by the harness when concurrent writes are enabled.
4. Keep adapter-specific mechanics such as `/replication/lsn`, changelog tails, and deferred-work
   gauges behind `server.drainedThrough`; report them diagnostically, never expose them in scenario
   assertions or use them as the row/value oracle.
5. Add barrier mutation controls at each stage: marker committed but not ingested, ingested but not
   sequenced, direct emissions landed while a query-back is held, gateway tail obtained before the
   barrier, client bytes fetched but cache transaction not committed, and a later SQL commit included
   accidentally. Each must fail at its named stage.

Task rewiring:

- Make this exact three-stage contract the first deliverable of `E2E-000` in both note 24 and the
  canonical task at `notes/18-production-readiness-spec-reviewed.md:1266-1281`.
- Make `MIG-000` reuse the same primitive rather than create a second notion of common fence.
- Change note 22's `barrier.afterCommit()` description to the adapter interface above. The existing
  `drainEngine` remains a process-adapter seed, not the common acceptance definition.

### P0 — E2E-DR-002: core transaction and replay semantics are overclaimed

Evidence:

- Note 24 says all profiles forbid a durable checkpoint **or resumable position** from advancing past
  an incomplete source transaction (`notes/24-postgres18-and-e2e-tdd-addendum.md:105-110`) and
  `SRV-E2E-001` requires “exactly-once materialized effect” (`:177`).
- Note 22 repeats this for the client-facing transaction suite and says after restart “the client sees
  the complete transaction once” (`notes/22-server-e2e-tdd-map.md:105-115`). Its boundedness case also
  says no resumable checkpoint exposes a partial transaction (`:226-231`).
- The canonical protocol deliberately defines all-native core as event/response-level framing;
  transaction framing is optional (`notes/18-production-readiness-spec-reviewed.md:272-288`).
  `PROTO-003B` covers one non-empty projected source transaction on **one stream** and explicitly has
  no cross-stream atomicity (`:281-288`).
- The canonical Swift core explicitly permits duplicate replay: `SWF-000` requires replay/duplicate
  semantics (`:1065-1074`), `SWF-005A` names duplicate replay (`:1135-1145`), and `SWF-006` folds
  duplicate replay and says a crash between delivery and checkpoint replays rather than loses data
  (`:1159-1170`).

Why this matters:

There are three distinct positions that the notes currently collapse:

1. the engine's input/source transaction high-water and slot acknowledgement;
2. an event-level public feed offset/resume token; and
3. a client durable application checkpoint.

All profiles must keep (1) behind a complete committed source transaction so replay remains safe.
`NATIVE_CORE` may expose and checkpoint (2)/(3) per applied event and may redeliver events after an
ambiguous client crash. Only `NATIVE_TXN_ATOMIC` with a negotiated eligible stream must withhold the
observer batch and its transaction-level checkpoint until the final marker. `COMPAT_V1` must follow
the pinned Electric behavior and cannot acquire a new source-transaction promise accidentally.

Actionable correction:

- Replace the all-profile statement with: “the server does not acknowledge/checkpoint its source-log
  position past an incomplete PostgreSQL transaction; every profile eventually folds to the exact
  fenced state without losing an acknowledged applied effect.”
- State separately that core public resume tokens are event/response-level and duplicate delivery is
  permitted according to the client checkpoint contract. Tests should require an idempotent final
  materialization, not “client sees once.”
- Apply the stronger rule only when `NATIVE_TXN_ATOMIC` is negotiated for that stream: no intermediate
  observer materialization, one final marker, one complete per-stream batch, and no transaction-level
  client checkpoint before the batch is applied. Preserve “no cross-stream atomicity.”
- Keep raw changelog terminal-marker/high-water assertions in focused `TST-011`/conformance tests.
  The public E2E asserts only the profile's documented framing, replay, checkpoint, and final state.
- Give every source journal operation its own immutable operation ID. A raw audit keyed merely by
  `(source transaction, row operation)` is ambiguous when one transaction updates the same row more
  than once.

Task rewiring:

- Amend `SRV-E2E-001`, the note-22 transaction suite, `BND-E2E-001`, and `E2E-001` acceptance in all
  three notes.
- Generate the core replay fixtures in `PROTO-003A`/`TST-002A`; generate final-marker and atomic-batch
  fixtures only for `PROTO-003B`/`NATIVE_TXN_ATOMIC`.

### P1 — E2E-DR-003: the DAG does not enforce the stated red-first process

Evidence:

- Note 24 says every behavior-changing packet first adds/selects the E2E, records a genuine product
  red, then implements and preserves unchanged test semantics (`notes/24-postgres18-and-e2e-tdd-addendum.md:230-247`),
  and says each implementation packet begins with its E2E case (`:250-253`).
- The PG18 tasks mostly honor this: `PG18-001` and `PG18-002` depend on `E2E-000` and explicitly begin
  red (`:284-308`).
- The broader tasks do not. `E2E-001` depends on the server/storage/operations implementations it is
  supposed to drive (`:323-332`; canonical lines `1456-1467`). `E2E-002` depends on the implemented
  gateway/security stack (`:337-344`; canonical lines `1474-1483`). The real-client E2Es likewise run
  after the complete provider/package tasks. None of the listed ENG/STO/SEC gateway implementation
  tasks depends on a recorded red public scenario.
- “These tests are intentionally red until the public gateway exists” in note 22 (`:192-195`) conflicts
  with the no-permanent-red/no-skipped merge rule unless those tests live as unmerged stacked patches.

Why this matters:

The current graph makes E2E-001/002 final qualification suites, not TDD drivers. An implementation can
close with unit/focused tests, and the high-level contract can be authored afterward to match it. The
prose asks reviewers to trust workflow discipline that `PLAN-001` cannot validate.

Actionable correction and task rewiring:

1. Split each family into a contract/red packet and a final qualification packet:
   - `E2E-001R` — author and record red public scenarios for server lifecycle/recovery;
   - `E2E-001Q` — run them green against release images after the implementation dependencies;
   - `E2E-002R` / `E2E-002Q` — the same split for gateway authorization/lifecycle;
   - equivalent `E2E-003CR`/`003CQ` and `E2E-003NR`/`003NQ` only where the reusable scenarios in note
     23 are not already independently frozen.
2. The `R` packets depend only on `E2E-000`, frozen public protocol/security decisions, and the
   smallest external adapter/proxy needed to demonstrate the failure. Relevant ENG/STO/SEC/CMP/SWF
   behavior packets depend on their scenario-red evidence. The `Q` packets retain the current broad
   implementation dependencies.
3. Store red patch/test semantic hash, scenario manifest hash, command, exact bad observation, and
   first divergence in the execution note. The green packet must use the same hashes; an oracle or
   assertion change requires a new reviewed red artifact.
4. Keep not-yet-green tests in a stacked patch or dedicated contract artifact, not merged as skip,
   inversion, expected failure, or a release-branch permanent red.
5. Seeded harness mutants must be external response/journal/materializer adapters or stacked product
   patches. Do not enable `ELECTRIC_CIRCUITS_FAULT` or another test-only behavior inside a release
   image and then call that image black-box production evidence.

### P1 — E2E-DR-004: stable scenario IDs encode implementation internals, and exact release-image cuts are not all gateable

Evidence:

- Note 24 promises that every internal component may be replaced while the stable scenario ID,
  journal, and oracle remain (`notes/24-postgres18-and-e2e-tdd-addendum.md:153-157`).
- `SRV-E2E-001` nevertheless requires change-log append splitting and a terminal marker/checkpoint;
  `SRV-E2E-005` names DS create/join/dropped/close/delete/retired phases; `SRV-E2E-008` names segmented
  log rotations, control append, checkpoint, and segment deletion; and `SRV-E2E-010` names catalog/DS
  combinations (`:177-186`). These are valid current invariants, but not stable public actions.
- Note 22 similarly exposes `/replication/lsn`, `changes/<n>`, `pendingFlips`, segment status, and
  `GET /replication/lsn` broken-epoch diagnostics in acceptance language (`:53-60`, `:149-160`,
  `:179-190`).
- `TST-007` supplies the truly exact in-process durable-step hooks, but says those hooks are disabled
  in release artifacts (`notes/18-production-readiness-spec-reviewed.md:1283-1293`). In contrast,
  `E2E-001` requires built images and 100 executions per enumerated cut (`:1456-1472`). External
  HTTP/TCP proxies can gate many DS/network responses, but they cannot announce every internal
  snapshot/checkpoint/control/delete transition after an arbitrary refactor.

Why this matters:

Either the stable test will become coupled to the current architecture, or the “100 exact cuts on
release images” claim will be implemented with timing guesses. A change from segmented logs to another
durable input representation should not require redefining a public recovery scenario.

Actionable correction:

- Split the cut manifest into two required tiers:
  1. **public release-image cuts** — PG commit, process signal/exit, TCP connection, external storage
     request accepted/withheld/lost, volume replacement/corruption, client request/response, gateway
     registry transaction, and public readiness/terminal events; and
  2. **implementation invariant cuts** — fsync, internal journal/checkpoint/high-water, catalog fold,
     rotation/control record, deferred query-back scheduler, and other named hooks under
     `TST-010`–`012` using an instrumented build from the same source SHA.
- Promotion requires both tiers and records source/artifact provenance, but must not claim that the
  instrumented binary is the immutable release image. Add one disabled-hook equivalence smoke proving
  the release artifact follows the same ordinary public trace.
- Rewrite stable IDs in external terms. For example, `SRV-E2E-001` should commit an operation-ID'd
  oversized transaction, interrupt the server after an externally held storage response, and assert
  the profile's public continuation/reset and fenced final state. Changelog marker/high-water details
  belong in the adjacent `TST-011` case, not the stable ID.
- Rewrite `SRV-E2E-008` as sustained writes plus a dormant consumer across declared retention/storage
  bounds and process/volume cuts. Segment-count/deletion remains operational evidence for the current
  release, not the public result.
- Every cut entry should declare `tier`, `gate mechanism`, `arrival event`, `release/kill action`,
  `public expected outcome`, `focused invariant`, and whether it is legal on the release artifact.

### P1 — E2E-DR-005: note 22's production topology conflicts with the canonical gateway-only boundary

Evidence:

- The canonical first release exposes only one authenticated TLS gateway; engine/API/DS/Postgres are
  private, public reads are proxied, and the API is absent unless an internal consumer is named
  (`notes/18-production-readiness-spec-reviewed.md:43-63`).
- The current note 24/canonical `E2E-000` text has already improved sequencing: an early direct-engine
  adapter is isolated test infrastructure, and `E2E-001` later uses the public gateway
  (`notes/24-postgres18-and-e2e-tdd-addendum.md:267-282`, `:323-332`;
  canonical `:1266-1281`, `:1456-1467`). That is a sound distinction.
- Note 22 still describes production acceptance as PG18 + engine and **API** images, accepts a
  `native-private` profile, starts the first image smoke through engine/API, and runs acceptance with
  “native-private + gateway” (`notes/22-server-e2e-tdd-map.md:31-43`, `:275-278`, `:299-310`).
- The current Docker Compose publishes PG, DS, engine, and API host ports and defaults DS to memory
  mode (`docker/compose.yaml:20-50`, `:57-95`). It is correctly a development stack, not a safe base
  whose successful test can be relabelled production topology.

Why this matters:

The detailed server map can lead implementers to make a private raw-engine/API run promotion evidence
or to test network isolation from the host/controller namespace, where all services are deliberately
reachable. It also leaves ambiguous whether “real client” means workspace source code or the exact
published package/app artifact.

Actionable correction:

- Align note 22 with the canonical two-phase rule:
  - `harness-core`/`direct-isolated` is non-promotion evidence used only to drive early PG18 work red;
  - every release/public E2E uses the authenticated gateway, with no `native-private` release profile.
- Remove API from the production acceptance topology unless a profile names a private consumer. Do
  not expose raw engine IDs, subscriptions, predicates, DS paths, or DS URLs to the reference
  materializer.
- Run `GW-E2E-006` from a dedicated client-network container/namespace. Give the controller a separate
  management network for fault injection and scraping. A host-port scan is not proof of production
  network policy.
- For promotion, accept only pinned image digests (`--pull=never`, no Compose `build:`), record runtime
  image IDs/config, exact PG18 minor/digest, DS store UUID/durability attestation, and gateway digest.
  Local source-built images may be labelled PR smoke but not immutable release evidence.
- Execute published/packed TS packages and immutable Swift artifacts in isolated consumer runners;
  importing workspace sources is acceptable only for the fast conformance adapter.

### P1 — E2E-DR-006: oracle independence and schema/key fidelity are underspecified

Evidence:

- Note 24 calls for a SQL/journal oracle and mutation tests, but does not say that the expected SQL
  must be independently authored rather than produced by the gateway's production template compiler
  (`notes/24-postgres18-and-e2e-tdd-addendum.md:94-103`, `:267-282`). Reusing the compiler on both sides
  would make a missing tenant predicate or wrong parameter binding a common-mode pass.
- The base mutation list covers missing, duplicate, stale, premature-fence, and generation, but not
  wrong predicate/tenant injection, absent-versus-NULL, wrong scalar type/precision, composite-key
  collision, wrong tombstone/delete, or wrong schema fingerprint (`:275-282`; canonical
  `notes/18-production-readiness-spec-reviewed.md:1278-1281`).
- The current comparator shows why reuse is unsafe. It sends the same `ShapeDef` to the oracle and the
  materialization (`packages/conformance/src/harness.ts:507-518`); coerces missing and NULL to the same
  value (`packages/conformance/src/compare.ts:21-24`, `:46-53`); and derives identity with
  `String(row[pk])` for one PK (`:32-40`).
- The canonical public schema correctly requires field-presence policy, ordered composite PKs and an
  opaque key codec (`notes/18-production-readiness-spec-reviewed.md:244-253`), and Swift codec tasks
  explicitly distinguish JSON null from missing and preserve numeric/key fidelity (`:1088-1099`).

Why this matters:

A beautifully fenced test still false-passes if expected and actual use the same faulty compiler or
lossy canonicalizer. Final map equality also cannot prove duplicate event behavior by itself, while a
strict raw “no duplicates” assertion is wrong for profiles that permit replay.

Actionable correction:

- For every admitted template/version, check in independent parameterized SQL and projection/key
  expectations. The oracle module must not import the production gateway compiler. Add a mutation
  that broadens only the production compiler and prove the independent SQL catches it.
- Make the comparator schema-directed: preserve missing versus NULL, scalar kind/precision,
  timestamps/bytes/Unicode, ordered fields, opaque composite key bytes, generation, and tombstone
  semantics. Reject duplicate canonical keys rather than silently overwrite a `Map` entry.
- Separate three oracles: fenced materialized state, public effect trace under that profile's replay
  rules, and ownership/generation state. Do not use message count as the state oracle.
- Extend `E2E-000` mutation proof with wrong predicate/tenant, absent↔NULL, key collision, wrong delete,
  wrong type, wrong source prefix, and stale generation. `TST-002A` remains the large cross-language
  corpus; E2E-003C/N and E2E-005 must consume the same canonical comparator artifact.

### P1 — E2E-DR-007: missing stable public-image scenarios

The broader canonical spec contains focused tasks for many of these risks, but they are absent or
implicit in the stable public-image inventory. Add explicit scenarios so a release cannot satisfy the
focused internal suite while missing the real gateway/client outcome.

1. **Cross-table/source-transaction causality.** Current transaction scenarios use multiple rows on
   one predicate/feed. Add `SRV-E2E-011`: one PostgreSQL transaction changes two tracked tables,
   including an inner membership relation plus outer direct rows, and affects direct, routed/circuit,
   and deferred query-back templates. Core profiles require exact final maps and safe source
   checkpointing; `NATIVE_TXN_ATOMIC` applies only per negotiated stream and explicitly does not claim
   cross-stream atomicity.
2. **DS process crash, not only outage/response proxy.** `SRV-E2E-005` gates DS requests and
   `SRV-E2E-010` describes restore matrices, but neither plainly requires SIGKILL/restart of the exact
   file-backed DS image around accepted-but-unanswered append/fsync and a simultaneous client tail.
   Add `SRV-E2E-012` with exact resume or typed whole-generation reset and SQL equality.
3. **Stale former engine/slot-busy public handoff.** The lower fault matrix and `TST-012` cover slot
   busy/stale process, but stable E2E covers only killing the old engine before starting its successor.
   Add `SRV-E2E-013`: start a successor while the former process still owns the slot/volume; the
   successor never becomes gateway-routable or mutates DS, then takes over only after confirmed
   termination.
4. **Revocation beyond an idle long poll.** `GW-E2E-005` gates a held long poll only. Add
   `GW-E2E-007` for policy/session generation change during create/renew and during a multi-page or
   streaming snapshot body. The byte recorder must sit outside the gateway; no post-barrier body,
   cache publication, or stale renewal is allowed.
5. **Public and internal TLS identity/rotation.** PG18 E2E covers PG TLS, while `E2E-002` does not
   depend on `SEC-006A/B` and has no gateway/DS TLS scenario (`notes/24-postgres18-and-e2e-tdd-addendum.md:337-348`).
   Add `GW-E2E-008` for wrong CA/name/client identity, plaintext/stripping, forced reconnect, and a
   dual-version rotation barrier across gateway↔engine↔DS plus public HTTPS. Rewire `E2E-002` to
   `SEC-006A` and `SEC-006B`, or rename it narrowly to gateway authz/lifecycle and require this case in
   a separate transport-security E2E before `E2E-005`.
6. **Overload cleanup with slow/cancelled consumers.** `CAP-003` names slow readers/reconnect/snapshot
   storms and the boundedness inventory samples resources, but the stable BND cases do not state the
   public crossing outcome. Add `BND-E2E-005`: at `limit-1/limit/limit+1`, stall readers, cancel
   creates/snapshots, churn reconnects, and hold a downstream permanently unavailable. Assert the
   precise admission/backpressure/reset result, no checkpoint past unapplied data, and cleanup at an
   explicit lifecycle/virtual-clock barrier. Absolute caps and crossing action, not only a sampled
   slope, are the acceptance result.

### P2 — E2E-DR-008: timer semantics need a two-tier policy and enforceable no-sleep scope

Evidence:

- The notes correctly ban sleeps for ordering and require announced gates (`notes/24-postgres18-and-e2e-tdd-addendum.md:127-129`,
  `:277-282`; note 22 `:62-66`). The canonical completion rules also require virtual-clock checks at
  `t-1/t/t+1` and finite operation manifests (`notes/18-production-readiness-spec-reviewed.md:121-139`).
- Some required release-image behaviors are genuinely controlled by external real clocks: PG18
  `idle_timeout` (`notes/24...:168`, `:306`), Electric live-poll `204`, retention/lease expiry, and
  shutdown grace. `TST-007`'s virtual-clock hooks are disabled in release artifacts (`notes/18...:1287-1290`).
- The current focused suite contains timing sleeps, including the concurrency race
  (`packages/conformance/src/conformance-concurrency.test.ts:28`, `:57-59`) and parked-poll setup
  (`packages/conformance/src/conformance-shutdown.test.ts:143-145`). The new architecture intends to
  replace, not silently inherit, those mechanics.

Why this matters:

“No test uses a sleep” is either unenforceable/overbroad (third-party protocol clients and real PG
timeouts necessarily use clocks) or will be weakened ad hoc. Conversely, a short configured timeout
plus `sleep(timeout + epsilon)` recreates the flakes the new harness is intended to remove. “Return to
baseline” can also become a calendar wait for GC/retention unless the completion event is named.

Actionable correction:

- Define two required timer tiers:
  1. deterministic process/model tests use injected clocks and assert `t-1/t/t+1`; and
  2. exact release-image tests allow the real external clock only when the timeout itself is the
     protocol/PG behavior under test. They poll an explicit state/result with a diagnostic deadline;
     they never sleep to create ordering.
- Add a lint for acceptance-owned sources that rejects direct `sleep`, unapproved `setTimeout`, and
  ad hoc `Date.now` polling outside one wait/deadline module. Maintain a small allowlist for the
  centralized deadline implementation and third-party client behavior.
- Every retention/lease/retry/rotation cleanup assertion must name the trigger and terminal event
  (claim release acknowledged, sweep triggered/observed, stream terminal, spill inventory empty,
  queue at declared baseline). A deadline is only a failure bound, not the event.
- Label the real PG18 `idle_timeout` case as one finite protocol fixture. It does not become a
  calendar-duration soak or statistical reliability claim.

## Areas reviewed with no finding

- **No calendar-monitoring release gate.** Note 24 explicitly makes capacity and long-run evidence
  finite and replayable and rejects calendar duration as promotion criteria (`:217-228`). The
  canonical spec uses fixed counts, seeds, event floors, and cut IDs (`notes/18...:121-139`). A bounded
  RTO/deadline or the one real `idle_timeout` protocol fixture is not calendar monitoring.
- **Truth oracle versus operational telemetry.** The notes correctly make PostgreSQL/journal state the
  truth, final real-client/app map equality the principal result, and readiness/metrics/resource
  observations barriers or boundedness evidence rather than row truth (`notes/22...:14-24`,
  `notes/24...:75-103`). Finding E2E-DR-001 concerns the missing causal connection, not this decision.
- **Profile-scoped observer atomicity as a product direction.** The documents correctly reserve one
  complete observer batch for `NATIVE_TXN_ATOMIC` and reject that promise in core/compatibility. The
  finding is limited to the conflicting all-profile “resumable position/once” wording.
- **PG18 scenario breadth.** Stored/virtual/unpublished generated columns, all non-null slot
  invalidation reasons including real `idle_timeout`, SQL and replication TLS, publication/plugin/
  identity drift, and unsupported promotion are all explicitly represented. PG18 is not declared
  supported merely because generic tests pass.
- **Focused tests remain first-class.** Both notes explicitly preserve conformance/unit/property tests
  beneath deployment-image E2E. That is the correct split: broad image tests should not replace
  algorithmic and exact durable-cut proofs.
- **Boundedness is operation-scoped and generally comprehensive.** The transaction spill, wide
  backfill, subscription/stream/resource plateau, fixed 10M operation corpus, absolute resource
  budgets, and safe-crossing tasks cover the major server resource classes. E2E-DR-007 adds missing
  public outcomes for slow/cancelled clients; it does not reject the overall boundedness design.
- **Early gateway sequencing is now sound in notes 24/18.** The current snapshots explicitly allow an
  isolated direct-engine adapter only for early PG18 red tests and require the public gateway for
  release E2E. The remaining conflict is the stale topology in note 22.
- **No acceptance based on circuit graph/catalog layout as the data oracle.** The canonical boundary
  rejects private Rust/Swift/circuit/catalog assertions. Current-specific metrics, stream inventory,
  and volume facts remain legitimate operational or focused-invariant evidence when kept out of the
  stable public result as recommended above.

## Recommended correction order

1. Resolve E2E-DR-001 and E2E-DR-002 in all three notes before implementing `E2E-000`; otherwise the
   shared harness will fossilize the wrong barrier and transaction promise.
2. Split red-contract authoring from green image qualification in the task DAG and add immutable test
   semantic hashes.
3. Separate public release-image cuts from instrumented invariant cuts; rewrite stable scenario
   actions in external terms.
4. Align note 22 to the canonical gateway-only topology and exact-artifact rules.
5. Strengthen the independent schema-directed oracle/mutation suite, then add the missing public-image
   scenarios and clock/lint policy.

After those corrections, the proposed E2E/TDD architecture is a credible refactor-safety layer over
the existing focused suite rather than a second implementation-coupled conformance harness.
