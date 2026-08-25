# Differential hardening review: readiness spec / parallel execution protocol

Reviewed read-only on 2026-08-23 against:

- `notes/18-production-readiness-spec-reviewed.md` (canonical reviewed spec)
- `notes/skills-research/05-parallel-agent-execution-protocol.md` (protocol v2)
- `docs/production/readiness-{tasks,schema,gates}.json`
- `scripts/readiness-plan.ts`, `scripts/readiness-plan.test.ts`, `scripts/readiness-evidence.ts`, `scripts/readiness-evidence.test.ts`
- current controller/resolution artifacts under `notes/execution/**`

## Executive result

The spec has the right safety posture (no production traffic, immutable candidates, profile closure,
gateway-only boundary, causal E2E fence, and explicit blocked outcomes), but the checked-in control
plane is not executable yet. It can validate a narrow, pre-registry shared planning packet only when
the checkout exactly equals a hard-coded historical integration commit. It cannot issue a selected
profile packet, red-artifact packet, or implementation packet, and it has no controller operation for
reserve/renew/revoke/integrate. Evidence helpers can also manufacture a clean row without running a
command or comparing a post-mutation external-input snapshot.

This is a no-go for claiming PLAN-001 completion or any production qualification. The failures below
are protocol/control-plane blockers, not permission to loosen the spec or treat current baselines as
qualification.

## Findings and exact corrections

### P0 — authority/base drift makes the scheduler unusable

Evidence: `scripts/readiness-plan.ts:21-36` pins integration commit
`520751ef250abd4936720e1e7e0c620a158833a0` / tree `8374caa…`; the checkout is currently
`0f94a029dc82a29c6f0f36ff82d262f49572c232` / tree `3367b528…`. `buildPlanningPacket` rejects any
state whose head/tree differs from the checkout (`:404-417`). Running the unit tests therefore fails
four packet tests with `stale_controller_state` (the tests themselves contain the old expected head).
The authority blobs happen to match, but the source/tree identity does not.

Correction: move the integration head/tree to controller state (signed, append-only, generation-bound)
and keep authority *content* pins separate. Packet issuance must read the current controller head and
verify it is an ancestor/current clean tree; updating the head after an accepted merge must not require
editing the validator source. A stale packet must fail with a deterministic `stale_dispatch_base`,
while a fresh packet on the new generation must be issuable.

Acceptance:

1. A clean detached source at the current controller head issues `GOV-001` and validates its packet.
2. Advancing the controller head revokes the old lease and issuing against the old packet fails.
3. A fresh packet at the new head validates without changing `scripts/readiness-plan.ts`.
4. Tests no longer embed a historical head; they derive a fixture commit/tree and test both stale and fresh cases.

### P0 — documented bootstrap gate commands are not runnable

Evidence: protocol/spec bootstrap acceptance lists `readiness-plan.ts ready --scope ... --completed ...`
without `--state`; `main` unconditionally interprets `args[-1 + 1]` as a path and reads `--scope` as a
file (`scripts/readiness-plan.ts:545`). The two documented commands fail with `ENOENT .../--scope`.
The `--completed` argument is ignored; only resolution rows in a supplied controller state are used.
`validate --manifest`, `--schema`, and `--gates` are also ignored and fixed paths are always read.

Correction: implement strict argument parsing. Every documented command must either receive a typed
controller-state path or use a documented read-only state source; reject unknown/omitted flags with a
named error. Implement `--profile` and `--completed` semantics, and honor (or reject) alternate
manifest/schema/gate paths. Update bootstrap packets/gates to the exact argv actually executed.

Acceptance: run each of the seven bootstrap direct-gate argv arrays in a fresh detached worktree and
obtain the expected exit/status/output; malformed or missing state, profile, and paths fail before
filesystem reads with named errors. No command is a daemon or unbounded wait.

### P0 — no executable controller / lease state machine

The protocol requires atomic reserve CAS, authenticated heartbeat/renewal, generation revocation,
event-log append/hash, merge admission, and separate resolution writing (protocol §§7–10), but the
repository contains only pure validators/builders. There is no reserve, renew, revoke, acknowledge,
integrate, or resolution command/service. `buildPlanningPacket` consumes externally fabricated state
and lease JSON; `validateLease` is a predicate, not a reservation implementation.

Correction: add a controller component (or explicitly mark PLAN-001 blocked until one exists) with
durable state and append-only event records. Implement CAS on
`(task,scope,attempt,generation,head,lease,available)`, authenticated nonce/ack exchange, expiry,
revocation, generation increment, and resolution creation. Include packet hash, handoff hash, event-log
hash, candidate/tree, gate report hash, and superseding packet in resolution records.

Acceptance: deterministic fixtures prove two concurrent reserves admit exactly one; stale generation,
wrong lease, replayed nonce, missed heartbeat, control-plane loss, and silent renewal all stop the
attempt; accepted integration increments generation and revokes/refreshes exactly the declared leases;
the resulting `.resolution.json` is content-bound and independently replay-verifiable.

### P0 — profile packets are structurally impossible; full packets are under-validated

The protocol packet requires `profile` and `release_profile_hash` for `per_profile` work, but the
generated `ordinary_packet` schema fixes `profile: null` and has no release-profile field
(`scripts/readiness-plan.ts:530`). `buildPlanningPacket` explicitly rejects every non-shared task
(`:415`) and has no profile argument. Thus no selected release profile can launch.

Conversely, the “full packet” branch (`:441-462`) is entered merely by adding empty
`candidate_identity`, `execution`, `ownership`, and `deliverables` objects; it then checks only a few
discriminators and returns. The existing test deliberately proves this (`readiness-plan.test.ts:123-129`).
Required allowed paths, read-set capture, candidate ancestry, gate matrix, evidence workspace, profile
manifest/blob, toolchain/image tuple, authority, reviewers, and Git authorization are not validated.

Correction: use a real discriminated schema: `bootstrap_plan`, `red_artifact`, `implementation_shared`,
and `implementation_per_profile`; require all fields from protocol §2 for each discriminator and run
the same strict schema/semantic validation on every packet (no “presence of four objects” bypass).
Implement profile packet generation and verify the selected release-profile canonical SHA + Git blob,
complete evaluated predecessors, applicability, and profile-specific scope.

Acceptance: all 17 legal profiles (compatibility plus 16 native feature subsets) can produce a packet
for an applicable task; an inapplicable task is rejected/N/A only by the controller; deleting any
required packet field, adding an empty placeholder object, changing profile/hash, or changing the
evaluated predecessor set fails with a named reason.

### P0 — genuine-red causality is not enforced

`scheduledReady` accepts a string set of `provider:consumer` keys as proof (`:318-324`); it does not
verify an artifact record, registry identity, current base/tree, red commit ancestry, test-source hash,
oracle/exclusions hash, or independent review. Genuine-red provider tasks themselves can become ready
without a red packet. `validateRedArtifactAdmission` only checks labels and `base_sha === checkout HEAD`
(`:388-402`), not that `red_patch_sha` is a real commit descended from the declared base, that its
tree contains only contract tests, or that evidence is a semantic assertion failure. It also accepts
legacy field aliases and a caller-supplied consumed set.

Correction: make `red_artifact` a first-class immutable ledger object. Verify commit/tree existence,
base ancestry, changed-file ownership, scenario registry identity/semantic hash/profile, exact failing
assertion and raw evidence digest, author/reviewer distinction, and one-consumer nonce. `scheduledReady`
must consume validated ledger identities, never arbitrary strings. Every `genuine_red` task (including
the provider/test-owner task) must have a registered red packet before implementation readiness.

Acceptance: a real red patch fails only at the intended semantic assertion; compile/setup/timeout/skip,
mutated oracle, changed exclusions, wrong profile/base, forged key, reused artifact, and a red patch
with production changes all fail. The implementation candidate must descend from the exact red commit;
changing the contract starts a new red identity.

### P0 — evidence builder can produce false pass evidence

`buildEvidenceRow` snapshots source before and immediately after setup, not after the command; it sets
`external_inputs_unchanged: true` unconditionally (`scripts/readiness-evidence.ts:112-136`).
`validateExternalInputs` is called with the same object for expected and current, so that comparison is
vacuous. `runCommand` has no timeout/deadline and does not bind a fresh source/run-root or invoke the
post-command attestation. `runBaseline` executes in the caller's checkout and writes the default result
under `docs/production/validation-baseline.json`; its `browser-demo` command is a long-running daemon.

Correction: make one runner own the full lifecycle: create fresh detached/exported source, attest before,
resolve read-only dependency/mount manifest, create unique empty external output root, execute each
command with diagnostic/global deadlines and process-group cleanup, snapshot source/dependencies/mounts
after, and emit a row only if all identities and raw result hashes match. Make baseline output external
to the source and reject daemon commands unless a finite terminal condition is declared.

Acceptance: a test mutating source, dependency manifest, mount target, config, or run root during a
command is rejected after the command; a command exceeding its deadline is terminated and `fail`/`blocked`
with raw evidence; no baseline run changes the source or reuses an output root; a finite browser/demo
smoke has explicit readiness and shutdown barriers.

### P1 — conditional DAG validation is incomplete

`validateManifest` cycle-checks only unconditional integrated edges (`:304-306`). Conditional edges
are syntax-checked and compared to a hard-coded array, but no selected-profile graph is topologically
sorted. Existing randomized tests pre-populate every red key and exercise only the current fixed graph;
they cannot catch a future profile-specific cycle or a conditional edge that creates a cycle only when a
feature is enabled.

Correction: compile every legal profile, add evaluated conditional edges, reject required inapplicable
dependencies and cycles per profile, and store the profile closure hash in the release-profile manifest.

Acceptance: all 17 legal profiles topologically sort; mutation fixtures add a cycle under exactly one
feature combination and must fail that profile while leaving unrelated profiles diagnosable.

### P1 — task identity/ownership is mostly synthetic and under-bound

The generator creates generic resources (`artifact/<id>/<boundary>`, `write/<id>/<boundary>`, and a
hash of the title) from prose (`scripts/readiness-plan.ts:47-48, 279`), and validation only checks that
strings are non-empty (`:284-302`). It does not bind actual changed paths, modes, read-set capture,
semantic contract IDs, or runtime resource leases. Authority inputs contain blob hashes but no per-file
tree/mode identity. This cannot prevent two tasks from owning the same real endpoint/schema/fixture when
their synthetic IDs differ.

Correction: make PLAN-001 output explicit path/resource records with canonical path/mode/content or
declared-absent status, semantic registry IDs, observed read-set report, and runtime reservation class.
Require every changed path to be owned, every declared path to be inside the pinned tree/expected export,
and every semantic/runtime collision to be machine-detected. Add tree IDs for both authority files and
the complete generated-output bundle.

Acceptance: an unowned path, overlapping endpoint/error/schema/fixture, changed symlink/mode, undeclared
read, or duplicate runtime namespace fails independently of textual diff disjointness.

### P1 — resolution and gate evidence schemas do not match protocol semantics

The generated `resolution` definition (`scripts/readiness-plan.ts:534`) requires only task/scope/outcome/
state/generation/base; it omits packet/handoff/event-log/gate/candidate hashes required by protocol §7.
The gate generator emits three generic command rows per task (`:539`) and validates only that the task ID
appears in a command comment (`:470`); most security, gateway, PG18, capacity, and qualification rows
therefore run `pnpm typecheck`, `pnpm test`, or `pnpm engine:test` rather than their exact real-stack
contract. A baseline exception is accepted for every TST-000 phase, including final qualification.

Correction: gate rows must reference a checked-in finite command/fixture/config/toolchain identity and
exact acceptance oracle, with applicability and evidence phase. Require capability-specific real PG18,
gateway, TLS/admission, capacity, backup/restore, external Electric, Swift/app, and browser commands;
reject comment-only commands. Restrict inherited baseline exceptions to the declared author/merge
characterization phase and never final qualification.

Acceptance: replacing a gate command with `echo ... TASK-ID`, generic typecheck, skipped/filtered/zero
tests, or a missing external lane fails validation; each selected profile's G0–G9 and G10 stage has at
least one finite command that reaches the named external contract and records raw result/count/deadline.

### P1 — lease/hash semantics conflict on renewal

The protocol treats packet identity as immutable while leases renew, but `packetCoreSha256` includes
lease `expires_at` (and `buildPlanningPacket` binds that digest as `packet_sha256`; `:421`). A legitimate
renewal changes expiry and therefore changes the packet hash, while bootstrap hashing excludes a related
expiry field. This permits accidental invalidation or ad-hoc rebinding of reviewed packet identity.

Correction: define immutable packet hash over task/base/profile/contract/inputs and an independent,
monotonic lease-event hash over nonce/generation/expiry. Renewal must preserve packet hash and append a
controller-signed acknowledgement; reviewers reject any packet/hash mismatch.

Acceptance: renewing a live lease changes only the lease-event record and expiry, not packet/core hash;
replayed, out-of-order, or silently renewed acknowledgements fail; generation changes revoke the packet.

### P2 — command/source safety details need hardening

`sourceAttestation` is a Git status/ignored-list check, not the complete tracked/index/mode/content and
mount inventory required by protocol §2. `assertRunRoot` relies on path-prefix and `readdir` checks and
does not bind an expected packet root or defend the create/check race. `createDetachedWorktree` and
`runCommand` do not enforce no writable overlays, process-group timeout, or post-command checks on their
own. These are acceptable focused helpers only if a single higher-level runner makes them mandatory;
currently nothing does.

Correction: use a privileged runner that opens/creates roots with no-follow semantics, records full tree
manifests and mount topology, binds expected root identity, and always performs post-command checks.
Keep helpers incapable of emitting a promotable evidence row in isolation.

Acceptance: symlink/race/root escape, writable overlay, ignored artifact, process child leak, and
post-command source mutation fixtures fail closed with a named reason.

## Security, gateway, admission, and operations differential

The canonical target contract is appropriately narrow: one authenticated TLS gateway, opaque
principal-bound feeds, private engine/DS/API/PG, explicit publication, non-superuser runtime, PG18
verified TLS, fail-closed epoch/reset, and bounded quotas/capacity. The gap is executable proof:

- No current gate command starts a topology and proves only the gateway is reachable while engine/DS/
  tRPC/PG listeners are private.
- No current runner exercises gateway auth/policy/revocation/quota atomicity or admission before engine
  mutation; generic unit commands cannot prove cross-process tenant isolation.
- No current command resolves OCI index to platform manifest, validates PG18 image/provider bytes, or
  records TLS `pg_stat_ssl`/channel-binding proof for bootstrap, pool/query-back, and walsender paths.
- No current capacity runner enforces open-loop offered/admitted/committed/applied floors, p999 sample
  minimums, resource reserve crossings, or deterministic cut-point terminal states.

Add named tasks/commands (or keep the profile blocked) for each. Acceptance must include negative
cross-tenant/forged-claim/unauthorized-route/unsupported-type tests, fixed PG18 platform digest and
TLS proof, disk/queue/FD/connection admission rejection with no downstream residue, and a finite
open-loop workload with per-template floors and exact stop conditions. A `blocked` external lane remains
non-promotable; it cannot be converted to a monitor-only wait or a passing inherited baseline.

## Current evidence and disposition

- `pnpm exec tsx scripts/readiness-plan.ts validate` passes its static manifest/gate checks, but this is
  not evidence that the controller or packet protocol is executable.
- `node --import tsx --test scripts/readiness-plan.test.ts` currently reports 6 pass / 4 fail; the packet
  failures are the stale hard-coded controller head noted above, plus red-fixture identity drift.
- The inherited baseline records Vitest/conformance retention failure, fuzz and external Electric lanes
  blocked, and browser lane unavailable. Per the spec these are characterization/blocker evidence, not
  qualification.
- Existing resolution notes for PLAN-001 attempts a1–a9 correctly classify invalidated/blocked work;
  none supplies a reviewed green PLAN-001 candidate or controller implementation.

## Fixed acceptance checklist for reopening PLAN-001

1. Refresh authority content/tree pins and controller state without editing pins per merge.
2. Implement strict CLI/controller operations and durable lease/event/resolution records.
3. Implement all packet discriminators, profile generation, full-field schema validation, and red ledger ancestry/evidence checks.
4. Replace evidence helpers with one finite fresh-source runner and mutation tests for source/dependency/config/root/process changes.
5. Compile and cycle-check every legal profile; verify ownership/read-set/semantic/runtime path identities against real artifacts.
6. Replace generic gate comments with exact finite commands and disallow baseline exceptions in final qualification.
7. Rerun the unchanged red/green PLAN-001 contract from a fresh evidence source, independently review it, and only then emit the next ready set.

