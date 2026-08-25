# Sol-high differential hardening synthesis

Date: 2026-08-23  
Repository tree reviewed: `0f94a029dc82a29c6f0f36ff82d262f49572c232` (canonical report context)  
Authorities: `notes/18-production-readiness-spec-reviewed.md`, `notes/24-postgres18-and-e2e-tdd-addendum.md`  
Inputs: `/tmp/electric-circuits-sol-server-differential.md`, `/tmp/electric-circuits-sol-swift-differential.md`, `/tmp/electric-circuits-sol-pg18-differential.md`, `/tmp/electric-circuits-sol-protocol-differential.md`, and `notes/reviews/01`–`16` (later `-sol-*` anchors are shorthand for these full paths).

## Verdict

**No-go for production traffic, PLAN-001 completion, or any release qualification today.** The
canonical spec is a materially improved *target* and correctly keeps the first profile narrow
(PG18, one primary/slot/engine, explicit publication, authenticated gateway, reset on promotion).
It does not yet prove that the control plane can issue/lease/review packets, that the exact PG18
connectors and startup state machine exist, or that the public causal fence reaches a real Swift/app
cache. The current tree also contains known red behavior (purge and fail-open restore/sequencing)
and PG16/cleartext launchers; these are not inherited passes.

Conditional go is possible only for read-only planning and evidence preparation after the
amendments below are represented in generated task/profile identities. A selected profile may be
called “conditionally ready for implementation” only when: (1) PLAN-001 is an executable controller,
(2) E2E-000S/A registry/fence is independently reviewed, (3) the selected closure topologically
sorts with exact gates, and (4) the profile's PG18 image/config/platform and app/package revisions
are content-bound. No conditional go authorizes traffic or waives a blocked lane.

## Evidence posture and historical cross-check

The canonical note itself states “no-go for production traffic today” (`notes/18-production-readiness-spec-reviewed.md:2452-2473`). Note 24 explicitly labels the PG18.6 generated-column observation unverified (`notes/24-postgres18-and-e2e-tdd-addendum.md:43-61`) and makes the marker/three-receipt causal fence a target (`:73-112`). The four Sol-high reviews agree that most earlier omissions are now specified, but identify remaining executable gaps:

| Source | Current evidence (not qualification) | Target claim that still needs proof |
|---|---|---|
| Server review `-sol-server:202-221` | Retention test 6/7, known purge mismatch; catalog restore logs and skips errors | `ENG-014/015` fail-closed purge/catalog, bounded deferred lanes, safe sequencer checkpoint |
| Swift review `-sol-swift:26-99,216-228` | ECS code has 28 providers, subsets/SSE/progressive modes; real GRDB clear is a no-op on 409 | `COMPAT_V1` admitted-template manifest, authoritative reset, control-state and app-host proofs |
| PG18 review `-sol-pg18:8-43` | Local OCI index/platform metadata and PG18.6 observations only | Exact image fixture, TLS on every connector, canonical publication/schema, slot incarnation/frontier |
| Protocol review `-sol-protocol:252-262` | Static validator passes; 6/4 test result and old PLAN attempts invalidated/blocked | Executable controller, fresh-source evidence runner, red ledger and profile packet generation |
| Prior reviews `notes/reviews/01:66-224`, `03:26-178`, `07:28-187`, `10:52-172`, `12:84-237` | Repeated cycles, DS restore, purge, source-fence, profile and gate laundering findings | Canonical task graph and E2E design must be implemented and independently rerun |

Earlier direct DAG-cycle findings are not repeated as a canonical-graph defect: PG18 review found the
checked-in graph structurally acyclic (`-sol-pg18:24-29`). They remain a regression obligation in
PLAN-001 profile mutation fixtures, not permission to skip controller work.

## Deduplicated omissions and exact task proposals

Severity is launch impact: P0 can falsely authorize a release or lose acknowledged state; P1 leaves
an essential proof/owner/edge missing; P2 is a bounded mismatch or documentation/support issue.

### P0 — Control plane cannot safely authorize work

1. **PLAN-CTRL-001 (new, owner `PLAN-001`) — executable controller and lease ledger.** Current
   `scripts/readiness-plan.ts` pins a historical head/tree, has no reserve/renew/revoke/integrate,
   and validates caller-fabricated lease state (`-sol-protocol:27-84`). Implement durable CAS on
   `(task,scope,attempt,generation,head,lease)`, signed nonce/ack heartbeats, expiry, revocation,
   append-only event hash, and controller-only resolution. Acceptance: two concurrent reserves admit
   exactly one; stale generation, replayed nonce, missed heartbeat, control-plane loss, and silent
   renewal invalidate; integration increments generation and revokes superseded leases.
2. **PLAN-PKT-001 (new, `PLAN-001`) — strict packet discriminators.** `ordinary_packet` fixes
   `profile:null`; selected profiles and genuine-red packets cannot be generated, while empty
   objects bypass full validation (`-sol-protocol:85-108`). Add `bootstrap_plan`, `red_artifact`,
   `implementation_shared`, `implementation_per_profile` schemas with profile/release hash,
   evaluated predecessors, ownership/read-set/resources, candidate ancestry, reviewers, toolchain/
   image/config and authorization. Validate all fields; reject placeholders and required N/A edges.
3. **PLAN-RED-001 (new, `PLAN-001` + E2E-000S)** — immutable red ledger. Bind provider/consumer/
   scenario semantic hash, base ancestry, test-only tree, exact semantic assertion and raw digest,
   independent review and one-consumer nonce. `scheduledReady` must consume ledger IDs, not strings;
   implementation must descend from exact red SHA (`-sol-protocol:109-129`; canonical `:313-329`).
4. **PLAN-EVID-001 (new, `PLAN-001`) — canonical finite evidence runner.** Existing evidence marks
   external inputs unchanged without a post-command snapshot and runs in caller checkout (`-sol-protocol:130-149`). Create fresh detached/exported source, read-only dependency/mount manifests, unique empty external roots, process-group deadlines, pre/post tree/config attestation and raw result hashes. Mutation, stale/writable overlay, source mutation, root reuse, daemon or timeout must fail/blocked with evidence.

These four are prerequisites of every later packet. They close the protocol findings without changing
the no-placeholder policy in canonical `:321-361`.

### P0 — Server/PG18 can acknowledge an effect or serve before continuity

5. **ENG-006B (new, depends `ENG-006`, `DSR-002`, `ENG-015`, `PG18-002C`) — startup preflight/ownership
   state machine.** Current boot mutates publication/replica identity before catalog/epoch checks and
   treats `Busy` as restorable (`-sol-server:27-72`; code anchors in review). Require read-only DS,
   catalog, landed frontier, slot incarnation, publication and exclusive-owner checks before restore,
   sequencer, mutation, or gateway routing. Runtime role cannot DDL publication/identity/slot. Foreign
   PG, DS-behind-slot, and former-engine-held-slot cuts assert zero PG/DS mutation and unready status.
6. **ENG-007B (new, depends `ENG-007A`) — fail-closed sequencer error classification.** Current
   `process_envelope`/replay logs errors, advances highwater and skips malformed envelopes (`-sol-server:74-109`). Permanent schema/table/op errors retire/reset; transient/deferred errors halt before
   processed/checkpoint; replay restore errors are typed failures. Add genuine-red malformed envelope,
   deferred query-back failure and checkpoint inspection to `TST-011/012`, `SRV-E2E-001/011/012`.
7. **ENG-007C (new, depends `ENG-007`) — owned deferred-emission lane.** Replace unbounded detached
   subquery writers and infinite retries with task handles, byte/item caps, shutdown barriers and a
   durable child intent or generation reset before source checkpoint (`-sol-server:111-142`). Add
   append-response loss, SIGTERM/SIGKILL and forced-grace cuts; `drainedThrough` includes the lane.
8. **PG18-001C (new, depends `PG18-001A/B`) — canonical schema enforcement mutation gate.** One
   effective publishable-column set must drive introspection, fingerprint, backfill, tuple decode,
   identity, drift and restore. Reject virtual or unpublished stored generated columns before feed
   creation; retain stored-positive and virtual/missing-field-to-NULL red fixtures (`-sol-pg18:60-76`,
   note 24 `:43-61`).
9. **PG18-002D (new, depends `PG18-002A/B/C`) — slot incarnation/frontier/timeline ledger.** Persist
   fully-landed source frontier and slot properties (`invalidation_reason`, restart/confirmed LSN,
   system/timeline, type/database/plugin/failover/two_phase). Any non-null/unknown invalidation and
   every promotion/timeline change break the epoch; same-name recreation ahead of frontier resets;
   replacement only via authorized reset (`-sol-pg18:77-97`).
10. **PG18-003B (new, depends `PG18-000`, `SEC-006A`, `ENG-006`) — connector TLS/conninfo parity.**
    Parse one canonical policy for bootstrap/admin, pool/backfill/query-back and walsender; enforce
    SCRAM `verify-full`, CA/SAN/channel binding, rotation/reconnect and `pg_stat_ssl`, reject unknown/
    weaker options. Current `NoTls`/reduced replication URL and cleartext Compose are P0 evidence
    (`-sol-pg18:46-59,185-197`).
11. **PG18-000B (new, depends `PG18-000`) — exact PG18 launcher/image gate.** Pin minor, OCI index,
    OS/arch tuple and resolved platform manifest in CI/Compose/demo/tutorial/bench; assert
    `server_version_num` before setup and reject PG16/17/unknown host binaries. Existing launchers are
    PG16/host-default and cannot qualify PG18 (`-sol-pg18:123-137`).
12. **PG18-GATE-001 (new, PLAN-001 gate generator)** — replace generic `pnpm typecheck` PG18 rows
    with finite fixture commands that emit version/image/config/fixture/source/engine/DS digests and
    raw artifacts. `PG18-001Q`, `002A/B/C`, `003A/Q`, `004`, `PGR-001` must execute real PG18 stacks,
    not compile-only checks (`-sol-pg18:138-154`).

### P0 — Public causal fence and Swift app boundary are not proven

13. **E2E-000C (new, depends `E2E-000A/B/I`) — target materializer receipt adapter.** The current
    `drainEngine` is diagnostic (standalone counter/private LSN/pending flips), not a marker in the
    same transaction plus public `server.drainedThrough` and target `client.appliedTailAfter`
    (`-sol-pg18:155-170`; prior `notes/reviews/10:52-114`). Build bad-adapter mutation fixtures:
    unpublished/early marker, deferred-work skip, wrong principal/template/generation and later-SQL
    query all fail. This is required by `E2E-000A` (`notes/24:73-112`) before any qualification.
14. **CMP-001A (new, depends `CMP-000/001`) — executable app capability census.** ECS has 28 model
    providers and progressive/on-demand/order/limit/subset/SSE call sites, beyond compatibility
    exclusions (`-sol-swift:26-55`). Emit per-callsite admitted/rejected capability rows and require
    `APP-REDESIGN-001` or `APP-NATIVE-SUBSET-001`/aggregate adapter for rejected consumers. G7a must
    fail on a toy template or make zero eligible templates an explicit N/A/block.
15. **CMP-004B (new, split from `CMP-004A`) — authoritative 409/reset cache transition.** Existing
    GRDB clear is a no-op, leaving stale rows after metadata reset (`-sol-swift:56-80`). Atomically hide/
    delete old synced generation, preserve declared local/optimistic overlays and dependent cleanup,
    and expose replacement only after application receipt. Add `RESET-001` to `E2E-003CR/CQ` and
    `E2E-004R/Q`; make `CMP-005` depend on B.
16. **CMP-002C (new, depends `CMP-002B`) — control-state semantics.** `snapshot-end`/`subset-end` are
    not `up-to-date`; compatibility must reject unsupported subset before network (`-sol-swift:81-99`).
    Add `CONTROL-001` with delayed page/live/delete/crash and readiness/checkpoint assertions.

### P1 — Missing proof owners, profile edges, migration semantics

17. **SWF-014 (new, depends `SWF-000/002/004`, `APP-OWN-001`) — versioned metadata/token migration.**
    Tag legacy offset/handle/cursor versus native stream/page LSN/feed/cache epoch; schema/account
    mismatch causes cold rebootstrap or typed reset. Add `TOKEN-001` to `E2E-003NR`, `E2E-004R` and
    migration closure (`-sol-swift:100-117`).
18. **CMP-003A (new, depends `SEC-002A`, `SEC-006B`, `CMP-003`) — app-owned credential/provider seam.**
    Real keychain/API provider, refresh/logout/redirect policy; preview/no-op providers cannot pass.
    Add `APP-AUTH-001` to compatibility/native E2E (`-sol-swift:142-160`).
19. **APP-TXN-ADR-001 (new, under `NATIVE-ADR-001/SWF-000`) — observer acknowledgement decision.**
    Explicitly select event-level receipt or `NATIVE_TXN_ATOMIC`; atomic requires `PROTO-003B`,
    `ENG-002`, `SWF-005B`, app sink and `TXN-APP-001`; event-level tests allow only fenced-prefix
    semantics (`-sol-swift:177-194`).
20. **APP-MIG-001 (new, depends `APP-OWN-001`, `CMP-000`, `GOV-004`) — app DB/schema/overlay rollback.**
    Bind schema epoch, local-only fields, optimistic replay, dependent cleanup and reader owner into
    migration artifact; `MIG-002/B`, `MIG-004`, `OPS-009` consume hash. Add `ROLL-APP-001`
    (`-sol-swift:196-215`).
21. Amend `TST-006C`, `CMP-006`, `SWF-010` for repeated background/foreground, delayed GC, duplicate
    close, account teardown, `@unchecked Sendable` and detached-task cancellation (`-sol-swift:119-141`);
    add `APP-LIFE-001` to `E2E-003CQ/NQ`.
22. Conditionalize `TST-005`, `ENG-003`, `TSC-001` and raw Electric portions of `E2E-001Q` on
    `lane == COMPAT_V1`; native receives validator N/A or isolated characterization only
    (`-sol-server:172-181`). Amend TST-004 for one gateway writer; two-replica quota race is future
    HA (`-sol-server:183-192`). Amend TST-010 to permit typed admission/refusal before mutation,
    while post-ack effects remain landing/retirement/reset only (`-sol-server:193-200`).
23. Add missing continuity edges: `STO-001 + ENG-015 -> PG18-002C`, `OPS-008 -> PG18-003A ->
    PG18-003Q`, and `ENG-015 + DSR-003 -> ENG-011` (`-sol-server:144-170`). Add catalog/store-generation
    hash and atomic frontier artifact to G3/TST-012.
24. Add PG restore/PITR producer/consumer binding: `PG18-002D + PGR-001 -> OPS-004/PG18-003Q`; exact
    provider artifact, system/timeline, slot/publication/schema and landed-frontier manifest decides
    resume versus whole-generation reset (`-sol-pg18:171-184`). Add conninfo parity to `PG18-E2E-013`.
25. Add protocol/profile identity edges: all 17 legal lane/feature profiles are compiled and cycle
    checked; packet profile closure hash is required. Add runtime path/endpoint/schema/fixture ownership
    and read-set manifest, and immutable packet core hash separate from lease-event hash
    (`-sol-protocol:150-213`).
26. Replace generic gate comments with exact finite commands and acceptance oracles. Every selected G0–G9
    and G10 stage must carry command/config/fixture/toolchain/image/platform/profile/scenario hashes,
    raw result/counts/deadline, and independent reviewer identity; generic typecheck/echo/skip/zero
    tests fail (`-sol-protocol:182-199`).

### P2 — Bounded mismatches and support posture

27. `TST-004`: test limit+1 admission against the selected single gateway writer; move two-replica test
    to future HA profile (`-sol-server:183-192`).
28. `TST-010`: distinguish pre-mutation typed refusal from post-ack landing/reset (`-sol-server:193-200`).
29. Add `DOC-PG18-001` (or include in `PG18-000B`) to mark PG16/cleartext, DS-memory, direct engine/
    visualizer routes as development/compatibility only and state exact PG18 TLS/publication/reset policy
    (`-sol-pg18:199-211`).
30. Harden evidence helpers (`sourceAttestation`, run-root no-follow/race checks, process child cleanup)
    behind `PLAN-EVID-001`; they must not emit promotable rows independently (`-sol-protocol:215-229`).
31. Keep future `DIRECT_DS_CAPABILITY`, `EDGE_CACHE`, HA gateway and seamless PG failover explicitly
    unsupported; no CDN or failover evidence can promote a common profile (prior `notes/reviews/08:475-506`).

The first canonical baseline repair remains `ENG-014` purge completion. Its known retention failure
(`-sol-server:202-221`) is not permission to alter the assertion or classify it inherited-green.

## Corrected dependency DAG (machine edges)

The following is the minimum amendment; PLAN-001 must generate hashes and applicability rather than
accepting prose. `->` means required predecessor.

```text
PLAN-001
  -> PLAN-CTRL-001 -> PLAN-PKT-001 -> PLAN-RED-001 -> PLAN-EVID-001
  -> GOV-001 -> GOV-002 -> PG18-000 -> PG18-000B
  -> PG18-001A -> PG18-001C -> PG18-001B -> PG18-001Q
  -> PG18-002A + PG18-002C -> PG18-002D -> PG18-002B
  -> PG18-003B + OPS-003A/B -> PG18-003A -> PG18-003Q
  -> ENG-006B -> ENG-007B + ENG-007C -> TST-011/TST-012 -> E2E-001R -> E2E-001Q
  -> E2E-000S -> E2E-000A/B -> E2E-000C -> E2E-000I

CMP-000 -> CMP-001 -> CMP-001A -> CMP-002 -> CMP-002B -> CMP-002C
  CMP-003 -> CMP-003A -> CMP-004 + (CMP-004A, CMP-004B) -> CMP-005 -> CMP-006 -> E2E-003CR/CQ
  APP-OWN-001 -> APP-MIG-001 -> MIG-002/MIG-002B -> MIG-004 -> OPS-009

SWF-000 -> SWF-002 -> SWF-014 -> SWF-004 -> SWF-006 -> APP-NATIVE-* -> E2E-003NR/NQ
NATIVE-ADR-001 -> APP-TXN-ADR-001 -> [PROTO-003B, ENG-002, SWF-005B] (only T)
NATIVE_SUBSET -> APP-NATIVE-SUBSET-001 -> SWF-009 -> E2E-003U
NATIVE_AGGREGATE -> APP-NATIVE-AGGREGATE-001 -> SWF-008 -> E2E-003A

STO-001 + ENG-015 -> PG18-002C; OPS-008 -> PG18-003A -> PG18-003Q;
ENG-015 + DSR-003 -> ENG-011; PG18-002D + PGR-001 -> OPS-004/PG18-003Q
```

Remove any inverse/late edge that creates a cycle; e.g. qualification consumes implementation and
red evidence, never supplies it. Migration `E2E-004Q` must precede `MIG-005` rehearsal, not follow it
(historical `notes/reviews/12:142-165`). Optional native runners and app adapters are conditional
edges and a selected profile with a missing applicable producer is invalid, never silently N/A.

## Concrete high-level E2E/TDD additions

All cases use the E2E-000A marker as the final statement of the same PG18 transaction, an independent
SQL/journal oracle folded only through `SourceCommitID`, real process/network/storage cuts, named event
barriers and finite deadlines. Internal LSNs/actors/logs are diagnostics only.

### PG18/server cases

- **PG18-TLS-001 (`PG18-E2E-007/013`)**: run setup, pool/backfill/query-back and walsender through
  hostssl/SCRAM/verify-full; wrong CA/SAN, rotation, reconnect, downgrade and channel-binding cuts.
  Require `pg_stat_ssl` per connector, exact image/platform/config digests, readiness fence and no
  plaintext fallback.
- **PG18-SCHEMA-001 (`PG18-E2E-003/004/005/012`)**: stored generated positive; virtual and unpublished
  generated negatives; toggle publication generated setting/RLS/partition policy while live/down.
  Snapshot and live values must equal SQL; any omitted wire field becoming null is a genuine-red failure.
- **PG18-SLOT-001 (`PG18-E2E-006/009/010/011`)**: idle_timeout/wal_removed invalidation; missing and
  synchronized same-name slot promotion; drop/recreate ahead/behind/equal frontier. Non-null reason,
  timeline change or ahead incarnation fail closed/reset before public reads.
- **PG18-BOOT-001 (`SRV-E2E-013`)**: start successor while former engine owns slot/volume or foreign
  PG endpoint is configured. Assert no publication/identity/slot/DS/catalog mutation, no sequencer and
  no gateway route until exclusive ownership and frontier checks pass.
- **SRV-SEQ-001 (`SRV-E2E-001/011/012`)**: malformed envelope, unknown table/op, transient deferred
  query-back, append response loss, SIGKILL/SIGTERM at lane barriers. Outcome is landed effect or one
  typed retirement/reset; checkpoint never passes the bad marker.

### Swift/app cases

- **RESET-001**: force 409/must-refetch with stale synced row, local-only row, optimistic mutation and
  dependent child; cut each GRDB transaction boundary. Old synced generation is never readable; local
  policy survives; replacement appears only after `client.appliedTailAfter`.
- **CONTROL-001**: hold `snapshot-end`/`subset-end` before `up-to-date`, delay page/live/delete, crash/
  reconnect. Readiness/checkpoint cannot advance or resurrect deletes; unsupported subset is rejected
  before gateway/engine work.
- **TOKEN-001**: persist legacy offset/handle/cursor, native feed/page LSN, subscription and cache epoch
  at delivery-before-apply/apply-before-checkpoint/schema/account/rollback cuts. Cross-lane deserialization
  fails closed; restart replays or typed-resets.
- **APP-AUTH-001/LIFE-001**: real injected API/keychain refresh, logout/account generation, duplicate
  close, delayed GC and repeated foreground/background. No preview/no-op provider, late old generation,
  stale claim or credential after the revocation barrier.
- **SUBSET-APP-001**: translate raw subset compiler output only to admitted AST/order/keyset/composite-key
  forms; raw SQL, multi-order, offset-live and unsupported keys reject before network.
- **TXN-APP-001**: if atomic selected, observer sees complete source transaction only after final marker
  and sink/checkpoint; otherwise assert event-level allowed prefixes and no false source-transaction claim.
- **ROLL-APP-001**: cutover/rollback with schema epoch, local-only field, optimistic replay, dependent
  cleanup, crash before owner switch. Exactly one complete visible generation; stale incumbent held or
  cold-rebootstrapped.

Each behavior case is a genuine-red/green pair: red patch test-only and independently reviewed; green
candidate descends from exact red SHA with unchanged scenario/oracle/exclusions. Focused unit/property/
model tests cover codecs, key identity, queue bounds and actor laws; they supplement but never replace
the black-box contract.

## Executable gate and evidence identity definitions

Every command runs in a new detached/exported source and unique empty external root. A gate row is
promotable only when all fields below match the generated packet; no comments, generic typecheck,
filtered/zero tests, retries or daemon waits count.

```yaml
evidence_identity:
  packet_sha256: immutable task/base/profile/contract/input hash (excludes lease expiry)
  lease_event_sha256: controller-signed nonce/generation/expiry chain
  source: commit_sha + tree_sha + complete tracked/index/mode manifest
  scenario: scenario_id + semantic_contract_sha256 + oracle_sha256 + exclusions_sha256
  profile: release_profile_sha256 + evaluated_closure_sha256 + gate_matrix_sha256
  inputs: pg_oci_index_sha256 + platform_tuple + platform_manifest_sha256 + image/config/provider
          digests + engine/ds/gateway/swift/app commits/subtree hashes
  execution: clean-source pre/post hashes + dependency/mount resolver hash + unique empty run-root
             identity + command argv hash + toolchain hash + raw artifact/result/count/deadline hashes
  review: author_packet_hash + independent_red_review_hash + reviewer_identity + resolution_hash
```

Minimum finite commands/oracles:

1. **PLAN gates:** controller CAS/lease fixtures; all seven documented bootstrap argv arrays; all 17
   profile compilations and cycle mutations; red-ledger semantic-failure mutations; source/dependency/
   root/process mutation runner. Assert named exit/status and content-bound resolution JSON.
2. **PG18 gates:** image resolution/version assertion; real hostssl fixture per connector; publication/
   schema/slot/frontier/promote/restore tests above; raw `pg_stat_ssl`, `pg_replication_slots`, SQL journal,
   readiness and public receipt artifacts. `pnpm typecheck` is adjacent only.
3. **Server gates:** `SRV-SEQ-001`, `PG18-BOOT-001`, deferred-lane and purge/catalog tests on file-backed
   DS with independent SQL oracle and durable checkpoint/frontier artifact. Acceptance inspects public
   outcome plus stored frontier, not logs.
4. **Swift gates:** frozen vendored app commit/subtree, real keychain/API provider, GRDB/cache reader,
   candidate gateway/engine/DS/PG18 digests; package-only sibling tests are `inherited_control` only.
   E2E-003CQ/NQ must report normalized app-reader map at `appliedTailAfter` and lifecycle/auth traces.
5. **Capacity/qualification:** open-loop fixed operation counts, per-template floors, 100 cuts and
   10,000,000 committed-operation minimum from the canonical manifest; resource samples keyed by
   operation/barrier and first crossing. Any blocked lane is non-promotable.

Gate outcomes are `pass | fail | blocked | not_applicable_by_profile`; only validator-generated N/A
is admissible. A `blocked` environment is handoff evidence, never a pass. Renewal changes only the
lease-event hash; changing packet/core, profile, source, config, image, toolchain or scenario hash
invalidates the attempt and requires a new packet.

## Prioritized first-wave dispatch

This is a dependency order, not a manual bypass of PLAN-001. No implementation packet is launchable
until its predecessor is integrated and the controller emits a ready packet.

1. **PLAN-001 recovery:** refresh controller head/tree authority and land `PLAN-CTRL-001`,
   `PLAN-PKT-001`, `PLAN-RED-001`, `PLAN-EVID-001`; independently rerun bootstrap commands and mutation
   fixtures. This is the highest leverage/no-traffic prerequisite.
2. **Governance + baseline:** `GOV-001`, `GOV-002`, `TST-000`, then `CMP-000` and `PG18-000/000B` in
   parallel where generated leases permit. Preserve the known purge red and all blocked lanes.
3. **Contract registry/fence:** `E2E-000S`, `E2E-000A/B/C/I`; publish the marker fixture, independent
   oracle and target materializer receipt before server implementation red patches.
4. **PG18 admission/connectivity:** `PG18-001A/C`, `PG18-002A/C/D`, `PG18-003B`, `OPS-003A/B`; freeze
   exact image/platform/config and create genuine-red schema/TLS/slot artifacts.
5. **Server safety:** `ENG-006B`, `ENG-007B/C`, `ENG-015`, `ENG-014`, then `TST-011/012` and
   `E2E-001R` red review. Startup, sequencing, deferred lanes, purge and catalog are common blockers.
6. **Gateway/profile closure:** `GOV-005`, `SEC-000`/gateway ownership and conditional external Electric
   edges; regenerate the manifest and gate matrix after each integrated task.
7. **Swift inventory and contracts:** `CMP-001A`, `CMP-002B/C`, `CMP-003A`, `CMP-004B`, `SWF-014`,
   `APP-TXN-ADR-001`, `APP-MIG-001`; then app capability red patches (`RESET/CONTROL/TOKEN/AUTH/LIFE`).
8. **Profile-specific implementation and qualification:** only after the selected lane's app adapter
   and optional modules close; run `E2E-003CR/CQ` or `E2E-003NR/NQ`, migration `E2E-004R/Q`, then
   `E2E-001Q`, `E2E-002Q`, `E2E-005`, `TST-003`, and G10 authorization stages on the immutable candidate.

## Closing claim

The canonical spec is suitable as the authority to compile a corrected task graph, not as evidence
that any target capability exists. The next safe action is to make PLAN-001 executable and to preserve
every current failure/blocked observation as immutable characterization. Production claims remain
prohibited until the exact selected closure, PG18 topology, causal receipts, Swift/app reader, and
all generated gates pass on one content-addressed qualification candidate.

