# Production Rust for the engine — 2026-08-23

## Scope and local baseline

This is guidance for `apps/engine`: a durable, at-least-once logical-replication and IVM service
whose correctness relies on transaction fencing, ordering, retry/reconciliation and bounded state.
It uses Rust 2024 and is deliberately pinned to **rustc 1.96.0** because newer stable compilers ICE
on `dbsp`; do not change that pin incidentally. The current CI runs `cargo fmt --check` and
`cargo test -p electric-circuits-engine`, followed by TS typecheck/conformance. It does **not** run
Clippy, documentation, Miri/sanitizers, Rust-native fuzzing, or dependency analysis. The engine
already has `tracing`, Prometheus/OpenTelemetry, explicit bounded channels/semaphores, a custom
shutdown token, and several localized `unsafe` uses (DBSP downcasts and `libc`).

The project invariants in `AGENTS.md` are more specific than generic Rust advice; they win. In
particular, neither ownership safety nor a passing unit test proves the required atomic delivery,
SnapshotGate, epoch, retirement, or transaction-marker behaviour.

## Normative rules (MUST)

1. **Make illegal engine states unrepresentable where feasible.** Use ownership, private fields,
   newtypes/enums, `Result`, and RAII guards to encode a stream/shape/transaction's lifecycle. Do
   not expose a handle before its durable-before-ack condition and post-wait race check complete.
   Prefer a small state machine with named transitions over boolean combinations or a “caller must
   remember” protocol. This is an application of Rust's ownership model, not merely style.

2. **Classify every failure at the boundary.** `Result` is for anticipated runtime failure; a
   `panic!` is for a violated invariant/bug, not normal network, storage, malformed input,
   cancellation, capacity, or Postgres failure. Preserve a typed, actionable category at policy
   boundaries (retryable/unavailable, definite refusal/gone, conflict, malformed/configuration,
   invariant breach) and retain the causal source/context. Never turn a recoverable append/catalog
   failure into `unwrap`, a log-only error, or silent loss. Rust's own guidance calls `Result` the
   good default for a fallible API and reserves panic for a bad state/invariant break
   ([Book](https://doc.rust-lang.org/stable/book/ch09-03-to-panic-or-not-to-panic.html),
   [`core::error`](https://doc.rust-lang.org/stable/core/error/index.html)).

3. **Do not detach correctness-critical tasks.** Every spawned task needs an owner, a cancellation
   path, an error/panic observation policy, and a shutdown join/safe point. Propagate a
   `JoinHandle`/`JoinSet` result or deliberately record/escalate it; dropped handles may hide a
   failed background writer. A Tokio `JoinHandle` reports panic or runtime cancellation on await
   ([Tokio spawning](https://tokio.rs/tokio/tutorial/spawning)). The existing `ShutdownToken` party
   protocol is the local default for long-lived engine parties.

4. **Treat cancellation as an ordinary control-flow edge.** Dropping a future cancels it; every
   `select!`, timeout, request handler, receive loop, and spawned sub-operation must be correct if
   interrupted at any `.await`. Do not put a non-cancellation-safe multi-step mutation behind a
   losing `select!` branch; persist/commit or explicitly compensate first. Keep in-flight futures
   pinned/reused when the operation must survive select-loop iterations. Tokio documents both the
   drop semantics and that non-winning `select!` branches are dropped
   ([select and cancellation](https://tokio.rs/tokio/tutorial/select)).

5. **Bound every untrusted or long-lived resource.** Name and enforce limits for channel backlog,
   concurrent query-backs/HTTP/PG work, task count, buffered transaction bytes, retry rate and
   duration, request/long-poll duration, in-memory indexes, WAL/change-log retention, and shutdown
   grace. Choose overload semantics deliberately (backpressure, defer/retry, spill, reject, or
   retire only when protocol allows) and export the saturation/queue-age metrics. A semaphore or
   bounded channel is a correctness mechanism here, not a tuning detail; preserve its permit and
   cancellation lifetime across awaits.

6. **Keep blocking/CPU work off async executor workers.** Do not hold a lock across `.await`;
   keep `std::sync::Mutex` critical sections short and low-contention. `tokio::sync::Mutex` is for
   state intentionally held across awaits, not a default substitution. Move blocking I/O and
   substantial CPU work to a bounded blocking/worker boundary and make its queue part of the
   resource budget. Tokio specifically endorses a synchronous mutex only for short,
   low-contention, non-awaiting critical sections
   ([shared state](https://tokio.rs/tokio/tutorial/shared-state)).

7. **Safe Rust first; unsafe requires a local proof.** No new `unsafe` without an explicit reason
   safe Rust cannot meet, a minimal block, a `// SAFETY:` proof tied to current invariants, focused
   tests, and review by an owner familiar with the abstraction. An `unsafe fn` documents every
   caller obligation; an unsafe block documents how this call site discharges it. Enable
   `#![deny(unsafe_op_in_unsafe_fn)]` at crate level if absent; 2024 already warns on implicit
   unsafe operations. The Reference defines unsafe as a proof obligation, not a performance
   annotation ([Reference](https://doc.rust-lang.org/stable/reference/unsafe-keyword.html),
   [Edition Guide](https://doc.rust-lang.org/edition-guide/rust-2024/unsafe-op-in-unsafe-fn.html)).
   Audit the existing DBSP `downcast` and `libc` blocks against that standard before expanding them.

8. **Instrument correctness boundaries.** Emit structured `tracing` events/spans and metrics at
   replication transaction ingress/commit, dedup decisions, SnapshotGate skips, append/retry/gone
   reconciliation, catalog durability waits, state transitions, queue/permit waits, checkpoint
   advancement, segment rotation/deletion, and shutdown parties. Include stable identifiers
   (shape/stream/table/segment/LSN/xid/attempt) but never credentials or row payloads by default.
   Logs must make a lost/delayed/duplicate-looking update distinguishable from a normal retry.

9. **Make a change reviewable and prove its exact risk.** Keep behavior, refactor, dependency, and
   formatting changes separable. State the affected invariant, ordering/cancellation/rollback
   behavior, limits, observability, and failure test. For engine changes, run the repository's
   mandated full gates: `pnpm typecheck`, `pnpm engine:test`,
   `ELECTRIC_CIRCUITS_ENGINE_PREBUILT=1 pnpm test`, and the Electric oracle command when available;
   live-path work also requires the demo/browser validation from `AGENTS.md`.

## Validation and supply-chain baseline (MUST for CI-worthy changes unless inapplicable)

* Run `cargo fmt --check`; the root `rustfmt.toml` intentionally sets 120 columns and stable
  options. Formatting is mechanical—never hide semantic edits inside it.
* Add `cargo clippy -p electric-circuits-engine --all-targets -- -D warnings` to the validation
  lane (initially baseline/fix existing findings rather than blanket-allowing). Every `allow` must
  be narrow and explain why; use `#[expect]` only for a deliberately temporary, checked exception.
  Rust lint levels and the fact that `forbid` cannot be overridden are specified in the
  [Reference](https://doc.rust-lang.org/reference/attributes/diagnostics.html).
* Run `cargo doc -p electric-circuits-engine --no-deps` (and `cargo test --doc` for public API
  examples) for changes to public APIs/docs. Public lifecycle, error, concurrency, ordering and
  cancellation contracts need rustdoc; examples are executable tests
  ([rustdoc writing guide](https://doc.rust-lang.org/rustdoc/how-to-write-documentation.html),
  [doctests](https://doc.rust-lang.org/rustdoc/write-documentation/documentation-tests.html)).
* Commit and review `Cargo.lock` whenever dependency resolution changes. Cargo locks exact chosen
  revisions until an intentional `cargo update`; inspect `cargo tree -d` and `cargo tree -e
  features` when adding/updating a crate to find duplicate versions and unexpectedly enabled
  features ([Cargo dependencies](https://doc.rust-lang.org/cargo/guide/dependencies.html),
  [`cargo tree`](https://doc.rust-lang.org/cargo/commands/cargo-tree.html)). Prefer the smallest
  feature set and justify direct native/FFI/proc-macro dependencies.
* Do not assume dev overflow checks cover production: Cargo's default release profile disables both
  debug assertions and overflow checks. Decide per arithmetic domain whether checked/saturating
  arithmetic or `release` overflow checks are required; benchmark and document the decision
  ([Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html)).

## High-value, context-dependent additions

* **Miri:** Run a small, pure/unit-test subset that exercises each unsafe abstraction and concurrent
  ownership logic on nightly (not the pinned production compiler). Miri finds many UB/data-race and
  aliasing failures, but is an interpreter with unsupported OS/async operations and is not proof of
  absence ([official Miri README](https://github.com/rust-lang/miri)). `MIRIFLAGS="-Zmiri-many-seeds=0..N"`
  is useful for nondeterministic thread interleavings.
* **Sanitizers:** On Linux CI or a dedicated runner, use ASan (and TSan where meaningful) for unsafe,
  FFI, allocator, and cross-thread code. They are nightly/target dependent, impose overhead, and
  detect only classes of bug; Rust recommends combining them with instrumented std where possible
  ([sanitizer flag reference](https://doc.rust-lang.org/nightly/unstable-book/compiler-flags/sanitizer.html)).
* **Rust-native fuzzing:** Add `cargo-fuzz` targets for `pgoutput` decoding, envelope/catalog folds,
  SQL/predicate parsing, changelog/control envelopes, transaction buffering, and adversarial
  sequence state machines. Seed corpus with conformance regressions; make the oracle/invariants the
  property, retain minimized crashes, and run bounded time budgets in CI/nightly. Fuzzing supplies
  pseudo-random inputs to find stability/security bugs; it complements deterministic conformance
  ([Rust Fuzz Book](https://rust-fuzz.github.io/book/)).
* **Model/property/interleaving tests:** Especially valuable for sequencer highwater/held
  transactions, append-vs-retire races, snapshot gates, and cancellation at each await. Prefer
  deterministic virtual time and explicit fault injection to sleeps. This is a stronger investment
  than broad unit-test quantity for this engine.
* **Release hardening:** Decide deliberately on release debuginfo, symbols, panic strategy, overflow
  checks, and crash reporting; keep enough symbolization/trace context to diagnose production
  invariant failures. The configuration is workload/operability dependent, not a universal
  “maximum optimization” recipe.

## API and implementation review checklist

For any changed boundary ask: (1) who owns the resource and releases it on every `Result`, cancel,
and panic path? (2) Which states/transitions are valid, and what becomes durable before the caller
can observe success? (3) Is every fallible outcome classified and handled at the right layer? (4)
What happens if the future is dropped at each await or a child panics? (5) What concrete maximum
memory/tasks/time/retry work does this introduce? (6) Can an operator trace and measure the outcome?
(7) Which invariant/property/regression proves it? The answer should be evident from the API and
the PR description, not reconstructed from incidental control flow.

## Proposed repository-local `rust-code` SKILL.md

**Routing:** trigger for any change/review/debugging in `apps/engine/**/*.rs`, root/engine Cargo
configuration, Rust CI, unsafe/FFI, Tokio tasks/channels/locks, replication decoding, durability,
or Rust performance/resource behavior. It should defer to the more specific engine invariants in
`AGENTS.md`, and pair with existing engine/conformance guidance rather than replacing it.

**Contents:** (1) read `AGENTS.md`, `rust-toolchain.toml`, root/engine `Cargo.toml`, `rustfmt.toml`,
and CI before acting; preserve the 1.96.0 pin; (2) a pre-edit invariant/ownership/error/cancellation/
resource-budget checklist; (3) mandatory safe-Rust-first and local `SAFETY:` proof policy; (4) Tokio
rules for task ownership, select/drop cancellation, lock-await prohibition, blocking work, and
bounded concurrency; (5) tracing/metrics fields at durability and lifecycle boundaries; (6) a test
matrix—fmt, Clippy, docs/doctests when API changes, engine/unit/conformance/oracle/browser gates,
then targeted Miri/sanitizer/fuzz guidance for unsafe/parsers/state machines; and (7) PR handoff
template naming invariant, limits, failure semantics, telemetry, and commands actually run. The
skill should label hard requirements as **MUST** and operational trade-offs as **consider**, avoiding
generic lint cargo-culting.
