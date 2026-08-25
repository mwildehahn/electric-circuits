# Rust / Cargo project structure — state of the art (2026-08-23)

This is a primary-source-only review of Rust/Cargo structure, tailored to
`electric-circuits`. It is a structural recommendation, not a reason to weaken an engine
invariant or do a crate split for aesthetics.

## Bottom line

Keep `apps/engine` as **one deployable Cargo package now**, but turn it into a deliberately
layered package with a small public facade. First reduce accidental public API and internal module
coupling. Extract a crate only when a stable, dependency-light seam has survived real use. Do not
create a crate per subsystem—or, especially, per shape, user, table, pipeline, or query template:
those are runtime concepts and Cargo structure must not scale with subscriptions.

The code should maintain this dependency direction (arrows mean “depends on”):

```
contracts + domain types
        ↑
query semantics / lifecycle / sequencer kernel
        ↑                   ↑
Postgres + durable-streams adapters   dbsp circuit adapter
        ↑                   ↑
HTTP/Electric adapter and binary composition root
```

This matches the as-built architecture: Postgres is the record of truth, the circuit has
rebuildable derived state, and shapes fan out outside the fixed circuit structure
([architecture §§0–1](../../docs/ARCHITECTURE.md#0-system-in-one-diagram),
[circuit tier](../../docs/ARCHITECTURE.md#6b-the-circuit-tier-counts-pipelines--the-membership-circuit)).

## Repository evidence

The current facts matter more than generic “one crate versus many crates” advice:

- The root is a virtual workspace with one Cargo member, `apps/engine`. It uses edition 2024 but
  `resolver = "2"`; it inherits edition/version/license, but has no `workspace.dependencies`,
  workspace lint table, or declared `rust-version`.
- `apps/engine` is a single package with the `electric_circuits_engine` library and the
  `electric-circuits-engine` binary. This is a sound deployable shape: the binary can remain the
  composition root and integration tests can exercise the library.
- `lib.rs` declares 29 public modules and only two private top-level modules. That makes much of
  the implementation an external API in practice. Existing integration tests reach `engine`, `ds`,
  `schema`, `table_ref`, `http`, and other modules through the public library.
- Large/high-coupling sources include `subquery.rs` (3,920 lines), `engine/lifecycle.rs` (2,817),
  `engine/catalog.rs` (1,886), `engine/mod.rs` (1,838), `engine/sequencer.rs` (1,763), and
  `pg.rs` (1,501). `pg`, `schema`, `changelog`, `predicate`, `ds`, and `engine` have the most
  cross-module references. Direct two-way references exist between `ds`/`pg` and `schema`/`value`.
  Those are pressure signals, not automatic extraction mandates.
- `txn_buffer.rs` directly uses Unix filesystem extensions and `libc::{getuid,kill,EPERM}`.
  `shutdown.rs` and `statsd.rs` already use conditional compilation. This is the clearest current
  platform seam.
- There is no project-local `build.rs` or proc-macro crate. `dbsp` deliberately brings a heavy
  Feldera tree. `cargo tree -d` also finds duplicate major-version families including
  `ordered-float` 3/4, `http` 0.2/1, `h2` 0.3/0.4, and `rand` 0.8/0.9/0.10; many will be justified
  incompatible transitive requirements, not debt that can be deleted by force.
- `rust-toolchain.toml` pins 1.96.0 because later stable compilers ICE on dbsp 0.318. `Cargo.lock`
  is committed, but current CI and Docker Cargo calls do not use `--locked`.

`AGENTS.md` is decisive context: engine changes must retain the snapshot fence, delivery,
transaction, catalog, retirement, epoch and schema-drift invariants and must run the prescribed
Rust, TS, conformance, Electric-oracle, and relevant browser checks. Structure work is engine work
when it changes dependencies or code paths, so it carries the same verification burden.

## Official Rust/Cargo guidance and applied policy

### 1. Crate boundaries and workspace ownership

Cargo workspaces share dependency resolution, a `Cargo.lock`, target directory, and root-only
profiles/patches. They can inherit package metadata, dependencies, and lint configuration
([Cargo workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html)). A virtual workspace
must state the resolver explicitly; edition 2024 implies resolver 3 for a package, but not for a
virtual workspace ([Edition Guide](https://doc.rust-lang.org/edition-guide/rust-2024/cargo-resolver.html)).

A workspace is therefore a good **governance boundary** for jointly shipped packages, not proof
that every directory should be a crate. Crates create a real public/dependency boundary: integration
tests in `tests/` are distinct crates and can use only a library’s public API
([Cargo glossary](https://doc.rust-lang.org/cargo/appendix/glossary.html#test-target)). Cyclic crate
dependencies are impossible. Do not create a crate until the desired dependency direction is
already acyclic as modules.

Repository policy:

1. Retain the one-member workspace while public API is contracted.
2. When a second Rust package is justified, inherit common metadata, `rust-version`, selected
   lints, and only dependencies whose versions/features genuinely need one workspace policy.
   Do not centralize every dependency just because `[workspace.dependencies]` exists; package
   manifests should still declare what each package uses. Workspace dependency features are
   additive ([Cargo workspace dependencies](https://doc.rust-lang.org/cargo/reference/workspaces.html#the-dependencies-table)).
3. Extract only if all gates hold: a named invariant owner; no needless dependency on `Engine`,
   axum, reqwest, tokio-postgres, durable-streams, or dbsp; an acyclic Cargo graph; public contract
   tests; and measured dependency/build benefit. Start as a private module facade, migrate callers,
   then move unchanged code in a follow-up. Never use traits to conceal a cycle.

### 2. Module visibility and public API layering

Items are private by default. `pub`, `pub(crate)`, `pub(super)`, and `pub(in path)` express
progressively narrower contracts ([Rust Book privacy](https://doc.rust-lang.org/book/ch07-02-defining-modules-to-control-scope-and-privacy.html),
[Rust Reference visibility](https://doc.rust-lang.org/reference/visibility-and-privacy.html)). Files
do not themselves make an API boundary; `mod` creates the module tree
([Rust Book](https://doc.rust-lang.org/book/ch07-05-separating-modules-into-different-files.html)).

| Ring | Examples | Rule |
| --- | --- | --- |
| Stable library facade | `Engine`, selected DTOs, `Row`, `Value`, `TableRef`, config entry points | `pub`; document invariants, errors, cancellation, and concurrency semantics. Re-export from a compact facade. |
| Adapter API | HTTP/Electric router, Postgres and streams ports | Public only if downstream embedding needs it; otherwise `pub(crate)`. A wire contract does not require every helper to be public. |
| Engine kernel | lifecycle, sequencer, membership, retirement, output, planning | Private children and narrow `pub(crate)` commands/events; do not expose locks, maps, indexes, or task handles. |
| Implementation | codecs, map representations, test helpers | private or `pub(super)`; add a local facade before refactoring. |

Make `lib.rs` a list of supported names rather than a mirror of source files. First redirect
integration tests to purpose-built test helpers/facades, then reduce one module family at a time to
`pub(crate)`. `cargo doc --no-deps` is the acceptance view because rustdoc documents only publicly
reachable items by default ([rustdoc reference](https://doc.rust-lang.org/rustdoc/command-line-arguments.html#--document-private-items-show-items-that-are-not-public)).

Do not publish a trait just to break an internal dependency cycle. Prefer a concrete domain
command/event or a small port owned by the consuming boundary. For a truly public extension point,
preserve evolution room with private fields/newtypes and, where appropriate, a sealed trait; these
are Rust API future-proofing guidelines ([checklist](https://rust-lang.github.io/api-guidelines/checklist.html)).

### 3. Domain and error boundaries

The public modules currently mix domain and infrastructure types, and many public functions return
`anyhow::Result`. `anyhow` is good for internal context and the binary/composition boundary, but an
opaque public error prevents callers from reliably distinguishing invalid predicate, create race,
durable-stream unavailability, snapshot/backfill failure, epoch break, or schema drift.

At a public operation boundary, use a domain error enum (or small family) implementing
`std::error::Error`, `Display`, `Send`, and `Sync`, preserving sources where meaningful. The API
Guidelines explicitly recommend meaningful public errors
([C-GOOD-ERR](https://rust-lang.github.io/api-guidelines/interoperability.html#c-good-err)). Existing
`Degraded`, `CreateRaced`, `SubscriptionConflict`, and epoch error types show the desired direction.
Map infrastructure errors to a domain category at the adapter/operation boundary, and map domain
categories to HTTP statuses in one HTTP layer; retain `anyhow` for internal task/startup failures.

Use newtypes at input boundaries (`ShapeId`, `SubscriptionId`, validated `StreamPath`, normalized
predicate/table key) rather than repeating ambiguous `String`s. This makes catalog/wire conversion
an explicit boundary and moves validation to the edge with little runtime cost
([C-VALIDATE](https://rust-lang.github.io/api-guidelines/dependability.html#c-validate)).

### 4. Features, build scripts, macros, and platforms

Cargo feature unification takes the union of features, so features must be **additive**: enabling
one must not disable behavior or make a normal combination invalid. Mutually exclusive features
are exceptional and should fail clearly if unavoidable
([Cargo features](https://doc.rust-lang.org/cargo/reference/features.html)). No project feature table
is appropriate today.

Feature policy:

- Never gate correctness modes, serving tiers, or Postgres/library semantics: those are runtime
  choices and must be tested in their real combinations.
- Add a feature only for an independently optional, additive capability with an owner,
  compile-cost justification, default policy, documented `cfg` boundary, and CI matrix row. An
  exporter or experimental adapter could qualify; `no-postgres` normally does not.
- Treat `default` as a compatibility promise. Test the minimum intended feature set and all
  features. Use `dep:name` to avoid accidentally exposing dependency names as features.

Keep the current no-local-`build.rs`/no-local-proc-macro posture. Build scripts run before package
builds, may generate modules/link native code, and rerun on every package-file change unless they
provide narrow directives ([Cargo build scripts](https://doc.rust-lang.org/cargo/reference/build-scripts.html)).
If unavoidable, a script must be deterministic, offline, hermetic, write only `OUT_DIR`, declare
every `cargo::rerun-if-changed`/`cargo::rerun-if-env-changed`, have output checked in CI, and never
fetch schemas or query a DB. Prefer checked-in generated Rust or an explicit generator command.
Proc macros run as host tools and add compiler dependencies; prefer maintained upstream derives
unless a manual implementation demonstrably harms safety or maintainability.

For portability, isolate OS behavior behind a small `platform` port. Move Unix temporary-file
permissions/owner-liveness behavior from `txn_buffer.rs` behind `cfg(unix)` implementations and
provide conservative non-Unix behavior or an explicit boot-time unsupported-platform error. Put
`libc` in `[target.'cfg(unix)'.dependencies]`, not universal dependencies. Cargo supports target
dependency tables ([Cargo dependencies](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#platform-specific-dependencies)); use `#[cfg]`, not `cfg!`, when an alternate branch
would not compile.

### 5. MSRV, toolchain, lockfile, reproducibility

The 1.96.0 toolchain pin is correct immediate risk management for the documented compiler ICE. Make
the promise explicit:

1. Add `rust-version = "1.96"` in `[workspace.package]` and inherit it in the engine package.
   `rust-version` is Cargo’s machine-readable supported-compiler declaration and applies to all
   package targets ([Cargo rust-version](https://doc.rust-lang.org/cargo/reference/rust-version.html)).
   Document “exact CI/development compiler 1.96.0; minimum supported Rust 1.96 while dbsp 0.318 is
   in use,” then revisit it when the ICE is fixed.
2. In a separate PR, test `resolver = "3"`, regenerate and inspect `Cargo.lock`, and run the full
   matrix. Resolver 3 is the edition-2024 workspace choice and makes Rust-version-aware fallback
   relevant. Do not combine this with dbsp/dependency upgrades.
3. Keep `Cargo.lock` committed; make CI and Docker use `cargo build/test --locked`. Use `--frozen`
   only where network access is intentionally forbidden. Cargo stores the selected graph in the
   lockfile and `--locked` fails an unintended update
   ([Cargo resolver](https://doc.rust-lang.org/cargo/reference/resolver.html#dependency-updates)).
   A scheduled dependency-refresh job should intentionally run `cargo update`, execute the full
   matrix, and submit the lockfile diff.
4. Each toolchain update must run the AGENTS-required Rust, TS, conformance, Electric oracle, and
   live demo/browser checks. A pin prevents drift; it does not prove semantic correctness.

If the engine is never intended for crates.io, add `publish = false`. If publishing is plausible,
first establish SemVer/API policy, metadata, and a curated exported facade; Cargo documents this
manifest control ([manifest reference](https://doc.rust-lang.org/cargo/reference/manifest.html)).

### 6. Compile time and dependency graph trade-offs

A huge crate recompiles broadly after edits to central types; microcrates add interfaces,
feature-unification work, and incremental-build overhead. Cargo incremental compilation is for
workspace/path packages and stores artifacts under `target`; more codegen units can improve compile
parallelism at a possible runtime-cost trade-off
([incremental](https://doc.rust-lang.org/cargo/reference/profiles.html#incremental),
[codegen units](https://doc.rust-lang.org/cargo/reference/profiles.html#codegen-units)).

Recommended sequence:

1. Baseline cold and incremental `cargo build --timings` plus
   `cargo test -p electric-circuits-engine --no-run`. Preserve the timing report for structural PRs.
   Use `cargo tree -e features -i <crate>` to trace activation; `cargo tree` represents the
   feature-unified graph ([cargo tree](https://doc.rust-lang.org/cargo/commands/cargo-tree.html)).
2. Keep fast incremental dev/test defaults unless measurement says otherwise. Evaluate a named
   release benchmark profile with `lto = "thin"` only against real ingest/shape workloads. LTO can
   improve optimized output at longer link time ([Cargo LTO](https://doc.rust-lang.org/cargo/reference/profiles.html#lto)).
3. Treat direct dbsp/rkyv/size-of pins as a compatibility island. Before editing them, inspect
   duplicate/feature trees, use a precise update, and run the full suite. Do not globally patch
   transitive dependencies merely to remove major-version duplicates.
4. Extract a low-churn, dependency-light contract crate only when measurement shows that it avoids
   rebuilding costly dbsp/HTTP/Postgres adapters for common domain edits. Do not make an
   `engine-core` that depends on every adapter; it only relocates the monolith.

### 7. Tests, examples, benchmarks, and documentation

- Keep unit tests beside pure parsing, schema/value/predicate algebra, retention plans, catalog
  folds, segmented-log transitions, and OS adapters; private unit tests preserve refactoring room.
- Treat `apps/engine/tests/` as black-box tests of the narrow public facade/router construction.
  They are the detector for accidental public API.
- Preserve the real authority of conformance/fuzz/Electric-oracle/Linearlite browser tests. No
  extracted crate substitutes for snapshot fence, transaction-end hold, catalog durability, epoch,
  or retirement testing.
- Add tiny `examples/` only for supported embedding/configuration paths. Cargo builds examples
  during `cargo test` but does not run their tests by default
  ([Cargo targets](https://doc.rust-lang.org/cargo/reference/cargo-targets.html#examples)); they
  must not import private implementation modules.
- Keep macrobenchmarks in the existing packages/bench/loadgen workflow. Add Rust `benches/` only
  for a stable focused workload and baseline. Built-in `#[bench]` is nightly-only; stable projects
  commonly use a harness such as Criterion
  ([Cargo benchmarks](https://doc.rust-lang.org/cargo/reference/cargo-targets.html#benchmarks)).
- Give public operations `# Errors`, cancellation/concurrency and invariant notes plus realistic
  doctests/examples. Rust API guidance calls for crate docs, examples, and failure documentation
  ([documentation guidelines](https://rust-lang.github.io/api-guidelines/documentation.html)).

## Evolving without a big-bang rewrite

### Phase 0: guardrails

Capture `cargo metadata`, duplicate/feature trees, timings, public rustdoc, and binary/image size
in a repeatable script or CI artifacts. Add/inherit `rust-version`, use `--locked`, decide
`publish = false` if appropriate, and separately evaluate resolver 3. Record a new-direct-dependency
policy: why existing/std code is insufficient, runtime/build/feature impact, and supported platform.

### Phase 1: a well-formed module graph inside one crate

First write the supported facade—Engine operations, domain DTOs/errors, and explicitly embeddable
adapters—then reduce unrelated `pub mod`s in small batches. Introduce ownership facades without
moving all files: e.g. `engine::{commands,state,delivery,catalog}`, `query::{predicate,schema,value}`
and `adapters::{pg,streams,http,electric}`. Break two-way imports by moving shared types downward:
`schema`/`value` need a low-level row/type contract; `pg` implements conversion/backfill against it;
`ds` owns stream protocol; engine-level events/ports connect adapters. Isolate Unix code behind
`platform` and compile the supported target matrix.

### Phase 2: only proven extraction

The possible first extraction is `crates/engine-domain`: canonical table refs, schema/value/predicate
representation and validation, independent of Tokio, axum, reqwest, tokio-postgres and dbsp. It may
properly remain private if its semantics and compile cost do not justify a crate.

Do not initially extract `sequencer`, `lifecycle`, `catalog`, `subquery`, or `arrangements`; their
shared atomicity and durable lifecycle invariants justify one kernel today. If adapter crates later
pay for themselves, make them one-directional (`engine-adapter-pg`/`engine-adapter-streams` depend
on domain/kernel ports) and retain the binary as a thin real-implementation composition root.

### Phase 3: enforce it continuously

Every structural PR should show before/after public rustdoc, package/dependency/timing delta,
invariant coverage, and why it does not turn runtime shape/pipeline data into compilation structure.
Add compile-only `Send + Sync` or non-exposure checks where they matter. Move one dependency edge at
a time; a new serving tier remains a deployable circuit/layout change with reseed/fallback behavior,
not a repository rewrite.

## Proposed repository-local `rust-project-structure` SKILL.md routing

Suggested location: `skills/rust-project-structure/SKILL.md` (or the repository’s established local
skill directory).

1. **Preflight:** read root `AGENTS.md`, relevant manifests/toolchain, architecture docs, and git
   state; determine whether engine invariants/live paths are in scope.
2. **Inventory:** run `cargo metadata --no-deps`, inspect public `lib.rs`, target layout, module
   sizes/import edges, duplicate/feature trees and platform cfgs. State current dependency direction
   before editing.
3. **Classify:** public API/visibility; module ownership/cycle; workspace/toolchain policy;
   feature/platform/build-script/macro; compile-time health; or justified crate extraction. Route
   each to a short checklist with the official Cargo/Rust source above.
4. **Decide:** default to a private module facade. Require the five extraction gates, additive
   features, explicit platform behavior, domain errors at public boundaries, and MSRV/lockfile
   policy before changing manifests.
5. **Implement and verify:** use small `apply_patch` changes, preserve dirty work, run `cargo fmt`
   plus the AGENTS-required engine/TS/conformance/oracle checks and live demo/browser flow when
   applicable.
6. **Handoff:** report public API, package graph, feature/platform/MSRV impact, dependency/timing
   evidence, tests run/not run, and deferred extraction candidates. Link primary sources rather
   than presenting convention as law.
