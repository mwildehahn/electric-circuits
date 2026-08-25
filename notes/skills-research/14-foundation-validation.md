# Skill and parallel-agent foundation validation

Status: **ready for an authorized foundation integration commit; product implementation dispatch has
not started**. Date: 2026-08-23.

## Scope

This pass created and hardened three repository-local skills:

- `electric-circuits-rust-code` for Rust implementation, concurrency, failure, durability, bounds,
  unsafe code, and observability;
- `electric-circuits-rust-structure` for modules/crates, dependency direction, facade/API, Cargo
  features and targets, build mechanisms, toolchain/MSRV, and reproducibility; and
- `electric-circuits-testing` for contract-first genuine-red TDD, proportionate focused tests,
  causally fenced high-level E2E, faults, isolation, and qualification evidence.

It also hardened `AGENTS.md`, the canonical production-readiness specification, the PG18/E2E
addendum, and the parallel execution protocol. The work changes guidance and specifications only; it
does not claim that the target PG18, gateway, causal-receipt, clean-runner, or scheduler infrastructure
already exists.

## Research basis

The recommendations were derived from official primary material, then reconciled with this
repository's as-built invariants and test harness:

- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/), the
  [Rust Book](https://doc.rust-lang.org/book/), and the
  [Rust Reference](https://doc.rust-lang.org/reference/);
- [Cargo workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html),
  [features](https://doc.rust-lang.org/cargo/reference/features.html),
  [build scripts](https://doc.rust-lang.org/cargo/reference/build-scripts.html), and
  [MSRV-aware resolution](https://doc.rust-lang.org/cargo/reference/resolver.html#rust-version);
- [Tokio task spawning](https://tokio.rs/tokio/tutorial/spawning),
  [`select!` and cancellation](https://tokio.rs/tokio/tutorial/select), and
  [graceful shutdown](https://tokio.rs/tokio/topics/shutdown);
- [Miri](https://github.com/rust-lang/miri),
  [Rust Fuzz Book](https://rust-fuzz.github.io/book/),
  [Loom](https://github.com/tokio-rs/loom), and
  [Proptest](https://github.com/proptest-rs/proptest); and
- PostgreSQL 18's official logical-replication documentation, including
  [generated columns](https://www.postgresql.org/docs/18/logical-replication-gencols.html),
  [publications](https://www.postgresql.org/docs/18/logical-replication-publication.html), and
  [logical replication slots](https://www.postgresql.org/docs/18/logicaldecoding-explanation.html#LOGICALDECODING-REPLICATION-SLOTS).

Public skills were inspected only for packaging, routing, and progressive-disclosure patterns; the
repository skills use original prose and repository-specific contracts. Detailed research and local
evidence are in notes 01–07 in this directory.

## Foundation decisions

- Behavior work is public contract → genuine red → minimal green → refactor. Product behavior uses
  the highest stable black-box boundary; pure/local laws stay at unit, property, model, fuzz, Loom, or
  Miri level rather than manufacturing E2E coverage.
- Replicated E2E comparison requires a source transaction marker, a server receipt that includes
  deferred work, a target materializer/cache receipt, and an independent SQL/reference oracle at the
  same source prefix. This is target work owned by `E2E-000A`, not a current capability claim.
- PostgreSQL 18 is the production acceptance major. Evidence pins the OCI index, platform tuple, and
  resolved platform-manifest digest; current PostgreSQL 16/host-selected tests remain regression
  evidence only.
- Parallel behavior work uses a separately reviewed, consumer-bound `red_artifact` packet followed by
  an implementation packet. Only the green stack merges.
- Task packets, leases, patches, gate phases, evidence, handoffs, and controller resolutions are
  immutable and injectively keyed. Integration is serialized and stale semantic/input overlap is a
  hard invalidation.
- Evidence never runs from the editing/control checkout. It uses an exact fresh detached commit tree
  or verified prepared-tree export; pre/post source state, immutable external dependency/resolver mount
  topology, a unique initially empty run root, and effective configuration are hash-bound. Reviewers
  recreate these independently.
- There is no calendar-duration acceptance criterion. Workloads use fixed operation/event floors,
  named cuts, explicit resource bounds, diagnostic deadlines, and deterministic terminal conditions.

## Independent review

The Rust-code, Rust-structure, and testing skills each passed a separate forward review after
hardening. Skill packaging/trigger quality passed. The canonical specification and execution protocol
then passed independent parallel-protocol and red-team reviews after closing these material findings:

- circular `PLAN-001` bootstrap identities and self-cancelling exact-base parallelism;
- gate-phase deadlocks around unavailable external qualification lanes;
- missing consumer-bound red-artifact review topology;
- mutable prepared patches and terminal handoff mutation;
- incomplete lease heartbeat/generation semantics;
- evidence from dirty/overlaid source trees; and
- stale or mutable external dependency/build/cache state outside an otherwise clean tree.

Final reviewer result: **PASS; no remaining P0/P1/P2 finding reported**.

## Validation executed

| Check | Result |
| --- | --- |
| Skill-creator `quick_validate.py` through `uv run --with pyyaml` for all three skills | Pass |
| Parse every `agents/openai.yaml`; require string display name and short description | Pass |
| Local Markdown targets across `AGENTS.md`, foundation notes, skills, and references | Pass |
| Balanced code fences, no trailing whitespace, no merge-conflict markers | Pass |
| Stale/contradictory terminology scan | Pass |
| `git diff --check -- AGENTS.md` | Pass |
| `cargo fmt --check` | Pass |

`cargo clippy -p electric-circuits-engine --all-targets -- -D warnings` was also executed and exited
nonzero on the current tree. Strict Clippy is therefore recorded as a known non-green baseline and is
not represented as a current gate; `TST-000` must retain the exact source/toolchain/raw result before
a dedicated repair/CI task can make it mandatory.

The full engine, TypeScript, Vitest/conformance, external Electric, and browser suites were not rerun
for this pass because no product code, runtime fixture, protocol implementation, or browser behavior
changed. Their required phase-classified execution remains in every future engine/live-path packet.

## Dispatch decision

The current control checkout contains modified/untracked foundation files, and repository policy says
not to commit without explicit authority. The parallel protocol also forbids launching an author from
a dirty control checkout or from prose-only task identities. Therefore no product implementation agent
was launched from this state.

The next legal sequence is concrete:

1. Obtain explicit authority for a local foundation commit (push/integration authority remains
   separate).
2. Integrate these exact reviewed foundation bytes into a clean pinned control history.
3. Create the sole bootstrap packet for `PLAN-001` at that commit and dispatch one
   `gpt-5.6-terra` high-reasoning agent in its own linked worktree.
4. Review and integrate `PLAN-001`'s six generated artifacts and validator.
5. Dispatch only the validator-emitted ready set; initially that is expected to expose `GOV-001` and
   `TST-000`, after which safe parallelism expands from generated dependencies and ownership leases.

Launching implementation agents before step 4 would bypass the very provenance and dependency
controls this foundation was created to establish.
