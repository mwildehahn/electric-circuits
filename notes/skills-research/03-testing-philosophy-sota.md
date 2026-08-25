# State-of-the-art testing philosophy for Electric Circuits

Status: research recommendation, 2026-08-23. This translates the high-level black-box E2E/TDD
decisions in [18-production-readiness-spec-reviewed.md](../18-production-readiness-spec-reviewed.md),
[23-swift-app-e2e-tdd-map.md](../23-swift-app-e2e-tdd-map.md), and
[24-postgres18-and-e2e-tdd-addendum.md](../24-postgres18-and-e2e-tdd-addendum.md) into a testing
doctrine. Those notes remain authoritative for profiles and scenario inventory.

## Decision

Treat each supported profile as stable high-level, black-box contracts. Drive a feature from a
genuinely failing example of one contract: begin with a named PostgreSQL source transaction and
finish only at a named client/cache materialization fence. The authoritative value oracle is an
independent canonical SQL query at that same source fence. Tests may control real processes,
networks, volumes, requests and responses, but never assert circuit layout, Rust task topology,
private offsets, or incidental retry counts.

Support that small product-contract core with focused unit, model/property, concurrency and fuzz
tests. Thus a gateway, circuit/routing/fallback tier, durable-stream implementation, client actor,
or cache can be refactored without rewriting product tests, while small invariants still get fast,
minimal counterexamples.

## Why a replication engine needs this

```text
committed PostgreSQL --logical replication--> engine/change log
  --durable streams--> HTTP client --materialization--> app observer/cache
```

Every boundary can delay, duplicate, lose a response after committing, restart, or expose a
partial operation. PostgreSQL logical replication starts with a snapshot and then continually
sends changes; it preserves transactional order within a subscription, but DDL/schema changes are
not replicated. [PostgreSQL logical replication](https://www.postgresql.org/docs/18/logical-replication.html)
[and restrictions](https://www.postgresql.org/docs/18/logical-replication-restrictions.html)
This is why a mock replica cannot qualify the real source path. Logical-decoding slot state can lag
through a crash and replay changes, so downstream effect must be idempotent.
[PostgreSQL logical decoding](https://www.postgresql.org/docs/16/logicaldecoding.html)

The repository already has the correct bones:

- `packages/conformance/src/harness.ts` owns a real Postgres database and globally unique slot per
  test, runs the Rust engine as a child process, uses durable-streams, and compares a client
  materialization to Postgres SQL.
- `packages/conformance` covers real-stack backfill, restart, epoch, catalog, retention, drift,
  transaction, subscription, subquery, aggregate and fuzz cases. `packages/oracle` and
  `electric-conformance/` are independent semantic checks.
- `apps/engine/tests` contains focused in-process/fake-DS proofs for mechanics such as chunked
  transaction emission and live-poll deadlines.

Keep all layers; make their relationship and release evidence disciplined.

## The hierarchy: trophy core, pyramid foundation

The testing trophy is right that integrations give high product confidence. The test pyramid is
right that small tests give fast and exhaustive feedback. For this system, use both deliberately.

| Layer | Question | Boundary / oracle | Frequency |
| --- | --- | --- | --- |
| Product acceptance | Does a supported profile keep its public promise across real PG, engine, DS and client? | public request/event/cache effect + SQL at source fence | PR smoke; full matrix nightly/release |
| Cross-runtime conformance | Do independent implementations agree on a stable corpus? | versioned fixtures + SQL/reference/Electric oracle | PR selected; full nightly |
| Component integration | Does a real component meet its contract against a controllable dependency? | PG, DS, process, HTTP boundary | PR |
| Model/property/metamorphic | Do sequences obey lifecycle and algebraic laws? | independent compact model / SQL | PR bounded; long corpus release |
| Unit/concurrency/memory | Is a local invariant true for all small states/schedules? | direct state model | every edit |

Use E2E when a real seam matters: snapshot/live handoff, xid visibility, slot/publication/schema,
durable acknowledgment/restart, public authorization/idempotency, or real Swift cache/observer
behavior. Do not make E2E the primary proof for parsing, predicate translation, envelope codec,
set/count fold, retry classification, ID generation, or every task interleaving. It is slow and
cannot exhaust schedules; use deterministic focused tests, retaining one E2E proof of integration.
Do not use an E2E elapsed-time assertion unless the deadline itself is a public wire/PG contract.

## TDD: red must be real

1. Name profile, stable scenario ID, public contract, source journal, source-to-client fence,
   oracle and terminal outcomes before implementation. Add a red reject test first for disabled
   capabilities.
2. Demonstrate the test fails against the actual candidate and reaches its assertion. A skip,
   compile failure, TODO, mock-only failure, or unexecuted assertion is not red.
3. Add the smallest focused failing test that explains the E2E failure: a model transition,
   property, codec case or bounded concurrency case. It may know internals; E2E may not.
4. Implement only enough to make both tests green. If public behavior changes, version the
   contract first; do not weaken its assertion to preserve green.
5. Refactor under the unchanged acceptance test. A minimized fuzz input or incident becomes a
   permanent checked-in regression.
6. Qualify only from an immutable pre-run manifest: exact source/digests/config/profile/contracts,
   fixtures/seeds/cut matrix and raw evidence. A changed input creates a new qualification.

The order in note 23 is sound: fail-closed admission, prove a deliberate corruption is detected,
then bootstrap/live/reconnect, checkpoint safety, ownership/cutover, and optional native modules.

## The oracle: SQL at the same source fence

Each mutating test transaction writes rows and a harness-allocated `SourceCommitID` sentinel in
the same PostgreSQL transaction. Compare client state only after a receipt chain:

```text
source SQL transaction + SourceCommitID
  -> server drained-through receipt (including deferred work)
  -> public event/response receipt
  -> client/cache transaction receipt
  -> canonical SQL materialization at SourceCommitID
```

Do not compare a historical client state with "whatever Postgres says now" after later writes. Do
not make private engine offsets/trace endpoints the public oracle: they are good diagnostics and
may implement a test-only barrier, but the assertion is SQL equality plus public behavior.

PostgreSQL snapshots matter: separately started sessions can see different content, and exported
snapshots exist to synchronize views. [Snapshot synchronization
functions](https://www.postgresql.org/docs/18/functions-admin.html) Therefore hold creation after
its repeatable-read snapshot, commit concurrently, release it, and assert snapshot+live equals SQL
(`PG18-E2E-002`). For ambiguous response loss, preserve the distinction between definite success,
definite failure, and an indeterminate operation which may or may not have taken effect—the same
history semantics used by [Jepsen](https://github.com/jepsen-io/history). Never call an
indeterminate lifecycle mutation a definite failure in the checker.

The existing `drainEngine` sentinel -> change-log tail -> pending-flip barrier is a good interim
harness. New acceptance work should progress toward note 24's reusable `CausalFence` receipts so
correctness cannot be manufactured by polling or a private implementation observation.

## Transaction and visibility contracts

Make the promise explicit per profile:

- No client-visible result claims an incomplete PG transaction. Chunked transactions hold until the
  final marker; test prefix redelivery, page boundaries and checkpoint restart.
- At-least-once transport is legal only when the final *effect* is exactly once: rows, aggregate
  weights, claim refcounts and cache cursor state all remain correct under replay.
- `NATIVE_TXN_ATOMIC` promises one complete per-stream observer batch and checkpoint after final
  marker. `COMPAT_V1` does not gain that promise accidentally; neither implies cross-stream atomicity.
- Deferred membership flips/query-backs have their own receipt. A sequencer-tail barrier alone is
  insufficient until deferred work drains.

Focused mechanics such as `txn_atomic_emission.rs` should prove highwater/hold/re-pin behavior;
real-stack scenarios prove the observable result.

## Fault, restart and network matrix

Each enabled profile needs a checked-in operation/cut manifest: operation ID, initial state, gate,
injected event, expected terminal state, recovery action, source fence and oracle.

| Boundary | Semantic cuts | Required result |
| --- | --- | --- |
| PG/snapshot | before commit; committed while snapshot held; engine absent | no uncommitted visibility; snapshot+live equals SQL |
| Slot/schema/publication | missing/lost/recreated/wrong plugin; identity regression; DDL/`TRUNCATE` | fail closed or documented retirement/reset; no half schema |
| Engine | SIGTERM drain; SIGKILL around checkpoint; writes while absent; successor overlap | no acknowledged loss; exclusive ownership; resume or typed reset |
| DS | create/append/close/delete held; response lost; DS restart; disk bound | durable intent before acknowledgement; eventual retirement |
| Gateway | request/upstream/body loss; idempotency retry; revoke during stream | exactly one claim or none; no post-revoke effect |
| Client/cache | cancel/crash around cache transaction/cursor/promotion; reconnect/account switch | no half generation, stale account or cursor past committed cache |
| Network/security | partition/delay/loss; TLS CA/SAN/credential rotation | retry or typed terminal state; no downgrade; recovery converges |

Control processes, a test proxy and a test issuer at external semantic gates. Do not mutate private
in-process state to create a public race. Maintain a second, same-SHA instrumented cut tier for
fsync/checkpoint/catalog/rotation/deferred-scheduler invariants, and run hooks-disabled equivalence
smoke on the release candidate. For broad concurrent histories, borrow Jepsen's approach—client
operations, explicit fault nemesis, persisted complete history and a declared checker—without
claiming linearizability for an asynchronous feed that does not promise it. [Jepsen fault
tutorial](https://github.com/jepsen-io/jepsen/blob/main/doc/tutorial/05-nemesis.md)

## Waits, clocks and determinism

- Create order with gates/events, not sleeps: every gate acknowledges arrived, held, released and
  terminal. Tests wait for arrival, inject, release, and await the contract.
- Every wait has one diagnostic deadline and emits phase, source ID, receipt trace, process state,
  redacted request IDs, logs and resource snapshot on expiry.
- A `sleep` is forbidden to manufacture a race or establish convergence. A centralized wait may
  back off internally, but its condition must be an explicit receipt/state transition. Existing
  harness polling is transitional infrastructure, not a pattern for new acceptance tests.
- Use injected/virtual clocks at `t-1`, `t`, `t+1` for lease/TTL/retry/retention/rotation. Use a
  real clock only when elapsed wire/PG time is itself the public contract.

## Property, model, metamorphic and tool-assisted testing

Build compact models for claim lifecycle, catalog fold/retirement, transaction assembly, client
cache generation and protocol codecs. Generate valid operation sequences, execute model and SUT,
compare at each synchronization point, shrink failures and check in the trace. Proptest provides
generation, shrinking, persisted regression cases and state-machine strategies; it explicitly
complements hand-written examples. [Proptest](https://proptest-rs.github.io/proptest/)
[failure persistence](https://proptest-rs.github.io/proptest/proptest/failure-persistence.html)
[state machines](https://proptest-rs.github.io/proptest/proptest/state-machine.html)

The TS seeded fuzz suite is valuable because it uses a real PG oracle. Evolve it to persist a
versioned operation trace (not seed alone), replay corpus first, minimize divergences, then run a
long immutable corpus. Use Rust `proptest` for pure Rust state machines and parser/fold laws.

Metamorphic relations efficiently cover representation-independent laws. The original technique
derives follow-up tests from successful executions through relations between their outputs.
[Chen, Cheung and Yiu, *Metamorphic Testing*](https://arxiv.org/abs/2002.12543) Examples:

- one multi-row transaction and an equivalent set of single-row transactions have the same final
  materialization, allowing only profile-declared event/batch differences;
- replay of an acknowledged change-log page/response is effect-idempotent;
- an impossible-to-match input does not alter a shape; predicate normalizations yield equal sets;
- `A; B; inverse(B)` returns to `A`; direct/routed/circuit tiers converge to the same SQL fence;
- restart at a recovery point is observationally equivalent to uninterrupted execution except for
  documented reset/refetch events.

Use Loom only for small synchronization cores behind `cfg(loom)`: it explores C11-memory-model
interleavings with state reduction. [Loom](https://github.com/tokio-rs/loom) Good candidates are
catalog wakeups, claim publication, retirement ownership and close-vs-append. Run Miri on reduced
nightly targets with unsafe/FFI/buffer/aliasing code; it detects many UB classes and data races but
does not prove all Rust bugs. [Miri](https://github.com/rust-lang/miri/) Consider bounded Kani
proofs for finite pure safety properties. [Kani](https://github.com/model-checking/kani) Fuzz
untrusted byte decoders—pgoutput, envelopes, headers, DS responses and URL/query codecs—with
`cargo fuzz`, real captures and a persistent minimized corpus. [Rust fuzz book](https://rust-fuzz.github.io/book/)

## Flakes, isolation and parallelism

Qualification default: zero retries; any failure fails. Retries are only a quarantined diagnostic
or stress facility; retain every attempt. A retry-pass is flaky and fails qualification, gets an
owner/scenario/expiry/reproduction plan, and never converts an ambiguous mutation into success.
`cargo-nextest` can label retry-pass as flaky and fail it explicitly. [nextest flaky
policy](https://nexte.st/docs/features/retries/)

Each real-stack test owns its PG database, global slot name, publication/role as applicable,
Compose/project namespace, ports, DS path/volume, process group, temp directory, seed/journal and
fault proxy. It proves cleanup of child processes, slots, databases, volumes and ports. Parallelize
only independent namespaces within declared resource budgets. Do not serialize the whole suite to
hide leaks, and do not share mutable default resources for speed. A process-global setting belongs
in a dedicated test process.

## Immutable qualification and provenance

Create a machine-readable manifest *before* every qualifying run. It pins source SHA and dirty
state, toolchains/lockfiles, PG/engine/DS/gateway/client image/package digests, OS/arch, release
profile, contract/scenario/cut hashes, exact redacted config, schema/publication fingerprints,
seeds/traces/corpus, event floors, deadlines and stop condition. Persist raw results, source
journals, logs, redacted requests/fences, comparator output, resource samples, first divergence and
allowlist hash. Fail closed for missing/zero/filtered/under-run tests, wrong/dirty source,
stale/missing artifacts, wrong digest/profile/config, changed comparator/allowlist, or unexpected
divergence. `blocked` is evidence, not pass.

Promotion consumes that immutable bundle. Changing a dependency image, config, seed, scenario/cut
list, comparator or allowlist invalidates the stage and requires a new run. This implements note
18's release rules and makes replay of a shipped candidate possible.

## Proposed `test-philosophy` SKILL.md outline

```markdown
# Test philosophy for Electric Circuits
## Scope and trigger
## First decisions: profile, scenario, public contract, SQL oracle, cuts
## Required genuine-red TDD loop
## CausalFence protocol and transaction visibility
## Waits, clocks and semantic external fault gates
## Choosing E2E vs conformance vs model/property vs Loom/Miri/fuzz
## Isolation, parallelism, cleanup and artifact capture
## Flake policy and immutable qualification
## Completion checklist
```

## Explicit enforceable rules

1. Every supported-profile E2E declares `scenario_id`, profile, `SourceCommitID`, public assertion,
   oracle type and diagnostic deadline; discovery fails if any are absent.
2. Materialization equality must follow a named source-to-client fence and use independent SQL or
   reference evaluation; private-offset-only/current-state comparisons fail lint.
3. Acceptance tests may not use sleeps/ad-hoc polling to establish order; only central gate/deadline
   APIs are allowed. Grandfathered occurrences are tracked and may not increase.
4. Every enabled profile has a real-PG, real-engine, real-DS, real-public-client smoke and explicit
   rejection tests for every excluded capability. Mock-only tests cannot satisfy it.
5. Every lifecycle/durability acknowledgment has response-loss, retry and restart coverage with
   `ok`/definite-fail/indeterminate history classification.
6. Every fault cut is checked in, semantic and externally controlled; internal hooks require a
   hooks-disabled release-candidate equivalence smoke.
7. Real-stack tests own unique database, slot, storage/volume, namespace/ports and process group,
   and prove cleanup.
8. Fuzz/property failures retain seed/trace plus generator hash; minimized counterexamples are
   checked in and replayed before novel generation.
9. Qualification is immutable and fails closed on bad provenance, skips, under-run, missing raw
   evidence, changed hashes or unexpected divergence.
10. Qualification has zero retry tolerance: retry-pass is flaky and fails the gate.
11. Engine/live-shape/client-path changes run the repository's required typecheck, Rust, Vitest,
   external Electric oracle and applicable real browser/app E2E gates; a missing prerequisite is
   reported blocked, never waived as done.
