# Forward review: `electric-circuits-rust-code`

Date: 2026-08-23

## Scope and method

I read `AGENTS.md`, the complete skill and its only routed reference
(`references/production-rust.md`), the ingest/reliability/shutdown parts of
`docs/ARCHITECTURE.md`, ADR-0003 and ADR-0007, and the current `ds.rs`,
`replication.rs`, transaction-spill and gone-reconciliation tests. This is a
read-only forward test; no implementation or test was changed.

The simulated change is deliberately hostile: a Postgres commit spills, is
posted as several change-log chunks, a durable-stream request is ambiguous or
temporarily unavailable, SIGTERM arrives during it, a restore-time append
exhausts its retry budget, and a peer keeps producing a large transaction.

For that change, the non-negotiable outcome is:

1. The ingestor appends every chunk in order, with contiguous `(lsn, seq)` and
   `last` only on the final envelope. It must not acknowledge the slot,
   publish `last_lsn`, or release the drain barrier until every chunk lands.
2. The sequencer holds an unterminated run and does not expose a fraction of
   the source transaction, advance its position, or advance a dormant resume
   position past it. Its high-water stays durable with its position.
3. SIGTERM is not permission to cancel the commit currently being appended:
   that commit reaches its safe point, while a transaction still buffering
   appends nothing. The process watchdog bounds termination; shutdown itself
   never advances the slot or retires a shape stream.
4. `append_reliable` on the live shape path retries unavailable or ambiguous
   results indefinitely and may discard only after the gone reconciler has
   retired the still-registered shape. `append_retrying` is the different,
   bounded restore/activation operation; its budget or shutdown error reaches
   the lifecycle owner, which may retire only under the stated policy.
5. The spill cap bounds the *ingestor* (cap plus one append chunk), not every
   downstream transaction holder. No size-triggered reject, purge, or
   retirement is legal; spill-file cleanup and writable-private-directory
   checks remain part of the design.

These points come from `AGENTS.md` “Invariants” (transaction, append and
shutdown bullets, especially lines 395-436 and 590-603),
`docs/ARCHITECTURE.md` §§3, 5.5 and 10 (lines 141-176, 441-464 and 787-803),
and ADR-0003 “Consequences” (lines 25-82). They match the present code:
`replication.rs:460-470` gates acknowledgement after
`append_commit_chunked`, `ds.rs:381-428` and `508-550` intentionally have
different retry/cancellation policies, and the checked-in fault tests cover
part of the chunk and gone cases.

## Findings

### P1 — The skill does not name the required append-mode and shutdown split

**Source:** `SKILL.md` “Production Rust rules”, lines 43-45; “Read first”,
lines 23-25; `references/production-rust.md` “Tasks and cancellation”,
lines 30-32 and “Bounded work”, lines 36-39.

The generic wording correctly asks for cancellation and a safe point, but is
not actionable for a change which crosses both append APIs. A well-meaning
author can add a shutdown `select!` around the chunk loop or replace a live
`append_reliable` with a budgeted `append_retrying`. Either is plausibly
consistent with the checklist yet wrong here: the first can abandon a current
commit, and the second can turn temporary unavailability into lost live shape
state. Conversely, adding a token to `append_reliable` merely because every
await is called a cancellation point conflicts with the sequencer’s required
finish-the-batch safe point.

**Minimal wording fix:** add a path-specific paragraph to the reference under
“Tasks and cancellation”:

> For ingest/sequencer append changes, identify the append mode before adding
> cancellation. `append_commit_chunked` and the sequencer’s in-flight batch
> are non-interruptible safe sections during graceful shutdown: finish them,
> then checkpoint; never acknowledge a partial commit. Live shape
> `append_reliable` retries until landing or reconciled retirement. Only the
> restore/activation `append_retrying` path has its bounded budget and shutdown
> token. Do not substitute one policy for another.

The paragraph should link to `docs/ARCHITECTURE.md` §§3, 5.5 and 10 and
ADR-0003. This is P1 because the current skill’s high-level advice permits an
implementation that silently violates transaction visibility or a registered
shape’s no-loss invariant.

### P1 — The TDD wording weakens the repository’s genuine-red requirement

**Source:** `SKILL.md` “TDD and implementation handoff”, lines 52-56;
`AGENTS.md` “Contract-first TDD”, lines 80-104; `AGENTS.md` “Mandatory skill
routing”, lines 43-49.

“Observe its failure when feasible” and “before or alongside” allow a behavior
change to proceed without the required red proof. That is materially weaker
than the repository rule: a stable black-box regression must fail at the
intended semantic assertion on the exact red patch; setup errors, timeouts and
skips are not evidence. The skill also does not explicitly route a behavior
change to `electric-circuits-testing` *before* implementation, despite the
mandatory ordering in `AGENTS.md`.

**Minimal wording fix:** replace the first sentence of the section with:

> For every behavior change or bug, first use
> `electric-circuits-testing` and record the repository-required genuine-red
> public contract on the exact base/red patch; if that proof cannot be
> demonstrated, stop rather than implement. Then add the smallest focused
> fault, cancellation, or state-machine test and make the unchanged contract
> green.

Retain the existing narrow spike/generated-code exception after this sentence,
but state that it cannot waive a behavior-change red proof. This is P1 because
the simulated change is exactly a behavior-changing protocol/failure-path
change for which a mock-only or non-red test would be false assurance.

### P1 — The memory rule is overbroad at the transaction boundary

**Source:** `SKILL.md` line 45; `references/production-rust.md` lines 36-39;
ADR-0003 “Consequences”, lines 25-40; `AGENTS.md` lines 430-436.

“Do not materialize a whole ... transaction when the established
streaming/spill mechanism applies” is sound for the ingestor, but it lacks the
ADR’s crucial scope boundary. The spill cap is explicitly *not* a global
transaction cap: the sequencer’s read page, held run and `txn_pending` remain
transaction-sized so that it can preserve atomic visibility. An author could
incorrectly propagate the ingest cap downstream, discard an over-limit held
run, or introduce a “too large” failure path—all ostensibly in pursuit of the
skill’s boundedness rule.

**Minimal wording fix:** append to the reference’s bounded-work bullet:

> For ADR-0003, state which component owns the bound. The spill limit applies
> to ingestor buffering (cap plus one append chunk), not to the sequencer’s
> transaction hold. Preserve `last`, re-delivery and high-water semantics; a
> transaction-size limit may never reject, purge or retire valid work.

This is P1 because mishandling the scope of the bound directly creates partial
transaction visibility or data loss under load.

### P2 — No explicit acceptance matrix ties the listed risks together

**Source:** `references/production-rust.md` “Test and evidence selection”,
lines 54-67; `SKILL.md` lines 54-56.

The matrix correctly asks for deterministic fault/cancellation testing and a
real-boundary proof, but it leaves the critical cross-product implicit. For
this path, the minimally named cases are: mid-chunk failure followed by
redelivery; SIGTERM while appending versus while buffering; false 404/410 or
closed response reconciled to landing versus confirmed retirement; transient
restore failure followed by recovery, budget exhaustion and shutdown; spill
past the byte cap with cleanup and no oversize body; and a real boundary check
that no subscriber observes a fraction of the transaction.

**Minimal wording fix:** add a single “Chunked ingest/reliable append” row:

> Require deterministic cuts at each chunk, shutdown safe point and terminal
> response; assert no early ack/barrier/checkpoint or partial client-visible
> transaction, correct replay/dedup, retry classification, and the declared
> ingestor memory bound. Pair it with the real Postgres/durable-stream/client
> contract.

This is P2 because the parent rules and current tests provide the ingredients,
but the skill does not assemble them into an easily reviewable contract.

### P2 — Validation language is internally ambiguous on strict Clippy

**Source:** `SKILL.md` “Validate and hand off”, line 60;
`references/production-rust.md` “Proposed or risk-triggered hardening”,
lines 88-96; `AGENTS.md` “Build & test” strict-Clippy paragraph.

The reference accurately labels Clippy, Miri, sanitizers, docs and fuzzing as
non-CI hardening. The main skill then says not to present them as required CI
*but* to “use them when their risk conditions apply.” For strict Clippy this is
not currently actionable: the repository records a known failing baseline
(44 library and 71 test-target findings on 2026-08-23) and requires a
dedicated baseline/CI task before it can become a green gate. A reviewer could
read the main skill as requiring an impossible pass for any correctness-
critical Rust change.

**Minimal wording fix:** change the end of line 60 to:

> Do not present Clippy, Miri, sanitizers, docs or fuzzing as current CI gates.
> Run or plan risk-triggered hardening only when its prerequisites and a
> meaningful baseline exist; for strict Clippy before that baseline, report the
> command honestly only when the task packet asks for it.

Current-gate claims otherwise check out: `.github/workflows/ci.yml` runs
`cargo fmt --check`, engine tests, typecheck and the full test suite; the
Electric oracle remains a repository completion gate in `AGENTS.md`, not a CI
step. No claim should call either current green evidence production
qualification.

### P2 — The compiler-pin explanation is broader than the checked evidence

**Source:** `SKILL.md` “Read first”, line 24; `rust-toolchain.toml` comment
and `ci.yml` toolchain comment.

Keeping the 1.96.0 pin is correct. “Newer stable compilers currently ICE,”
however, reads as a claim about all later stable versions. The checked-in
evidence names 1.97.0, 1.97.1 and 1.98.0; it does not establish the broader
statement indefinitely.

**Minimal wording fix:** replace the second sentence with:

> Keep the Rust 1.96.0 pin. The repository has recorded dbsp ICEs on the
> tested newer releases named in `rust-toolchain.toml`; re-check that file and
> CI before changing the pin.

This is P2: it does not endanger the simulated data path, but it keeps current
and future validation claims precise.

## Positive checks

- The skill’s scope, repository-authority statement, typed-error guidance,
  task ownership, lock rule, observability prompts and no-detached-task rule
  are suitable for the simulated change.
- Its validation reference correctly separates the present repository gates
  from proposed hardening. The defect is the main skill’s ambiguous “use them”
  clause, not a claim that Clippy is current CI.
- The skill correctly requires reading the changed module, neighboring tests,
  toolchain, manifests, formatter and CI. For the simulated path, the proposed
  append-mode paragraph should additionally make the cross-module read set
  explicit: `replication.rs`, `txn_buffer.rs`, `engine/sequencer.rs`, `ds.rs`,
  `shutdown.rs`, `txn_spill_chunking.rs`, `txn_atomic_emission.rs`, and
  `append_gone_reconciliation.rs`.

## Verdict

REVISE

## Re-review — 2026-08-23

I re-read the current `AGENTS.md`, the complete hardened `SKILL.md`, and the
complete routed `references/production-rust.md`, then checked the linked
architecture and ADR sources against the current implementation-facing
invariants. The new relative links from the reference resolve to the intended
architecture and ADR files.

### Earlier findings now resolved

- **P1 append/shutdown policy:** resolved. The new tasks/cancellation rule
  distinguishes the graceful-shutdown safe sections from token-interruptible
  waits, and names the materially different `append_reliable` and
  `append_retrying` policies. It matches `AGENTS.md` lines 462-477 and 662-673:
  a commit in append completes without a forced slot advance; live-shape
  appends do not receive a bounded retry budget; restore/activation does.
- **P1 genuine-red and testing-skill routing:** resolved. `SKILL.md:8` now
  routes behavior work through `electric-circuits-testing` first, and
  `SKILL.md:54` requires a frozen, test-only red-patch tree that fails at the
  intended assertion. Its local-only focused-law versus product public-contract
  distinction now agrees with `AGENTS.md:104-110`; it no longer demands a fake
  E2E for a local parser/algebra law.
- **P1 ADR-0003 bound scope:** resolved. The reference now explicitly limits
  the spill cap to the ingestor and preserves the sequencer's transaction-sized
  hold, `last`, replay and high-water behavior. This matches
  `AGENTS.md:500-506` and ADR-0003.
- **P2 Clippy/current-gate phase:** resolved. The validation matrix and skill
  now call strict Clippy non-green until a dedicated baseline/CI task preserves
  an exact result. The current author/merge versus release-qualification
  distinction also agrees with `AGENTS.md:411-440`; current green regression
  evidence is not presented as production qualification.
- **P2 compiler pin wording:** resolved. `SKILL.md:24` now preserves 1.96.0
  while limiting the ICE assertion to versions actually named in
  `rust-toolchain.toml`.
- **P2 cross-cut acceptance matrix:** substantially resolved. The new
  chunked-ingest/reliable-append row names per-chunk cuts, shutdown, terminal
  reconciliation, replay/dedup, client visibility, and the ingestor bound.

### Remaining findings

| Severity | Finding |
|---|---|
| P0 | None. |
| P1 | None. |
| P2 | The new chunked-ingest/reliable-append test row says “no early ... checkpoint,” which is too broad. |

**P2 — Distinguish a checkpoint position from its high-water.**

**Source:** `references/production-rust.md:64`; `AGENTS.md:485-499`; and
`docs/ARCHITECTURE.md:173-179`.

The hardened test row is otherwise right, but its literal “no early ...
checkpoint” assertion conflicts with the required checkpoint behavior while a
run is held: the position remains pinned at the page where the hold began,
while the `(lsn, seq)` high-water can and must be durably written when earlier
completed transactions advance it. A test written from the row could forbid
that necessary high-water-only checkpoint and thereby regress crash replay
safety.

**Minimal wording fix:** replace “no early source acknowledgement, drain
barrier, or checkpoint” with:

> no source acknowledgement or drain barrier before the last chunk, and no
> processed position, checkpoint position, deletion floor, or dormant resume
> position past the page where a run is held (while retaining every required
> high-water-only checkpoint).

This is P2 because the authoritative invariants make the intended meaning
recoverable, but the test-selection rule should be unambiguous on the
correctness-critical replay path.

## Re-review verdict

REVISE
