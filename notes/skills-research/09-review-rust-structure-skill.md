# Forward review — `electric-circuits-rust-structure` skill (2026-08-23)

## Verdict: REVISE (no P0; two P1 additions)

The skill chooses the right default for this repository: establish an intentional private module
facade in the existing `electric-circuits-engine` package, then extract only a proved,
dependency-light contract.  It explicitly prevents a crate per runtime shape, table, user, or
pipeline, and it requires an acyclic direction, public-contract tests, SemVer/publishing intent,
and measured benefit before extraction.  That passes the requested `pgoutput`/query/domain split
forward test.

It also correctly treats platform code and Cargo features as distinct concerns; features are
additive and cannot select correctness or serving tiers, while a platform branch has a port,
explicit supported-platform outcome, target dependencies, and branch compile coverage.  However,
the skill does not turn either the feature/platform matrix or an MSRV declaration into executable
acceptance evidence.  Those omissions are P1 because an otherwise careful implementation can
leave an uncompiled target/feature branch or falsely call an exact toolchain pin an MSRV.

## Scope and evidence read

Read in full:

- `AGENTS.md`;
- `.agents/skills/electric-circuits-rust-structure/SKILL.md` and each of its references:
  `references/repository-map.md`, `references/decision-checklists.md`, and
  `references/official-sources.md` plus every linked primary Rust/Cargo reference;
- root `Cargo.toml`, `apps/engine/Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, and
  `apps/engine/src/lib.rs`;
- `docs/ARCHITECTURE.md` §§0–4, §9 and §14, and
  `docs/ivm-engine-internals.md` §7; and
- the current structural baseline in `notes/skills-research/02-rust-project-structure-sota.md`.

Read-only evidence captured:

```text
cargo metadata --no-deps --format-version 1
cargo tree -e features -p electric-circuits-engine
cargo tree -d -p electric-circuits-engine
```

The root is a virtual workspace with one member; the member has a library and binary target,
no `[features]`, no target-dependency table, no `rust-version`, and an intentionally committed
lockfile.  `rust-toolchain.toml` pins exactly Rust 1.96.0 because dbsp 0.318 ICEs on newer stable
compilers.  This is a compiler-selection workaround, not evidence of an MSRV promise.  The
existing public facade is too broad for an extraction decision: `lib.rs` exports 27 modules and
integration tests directly import many internals.  The direct dbsp/rkyv/size-of compatibility
island and its duplicate transitive families make an unmeasured dependency change especially
unsafe.

The architecture evidence matters to every proposed seam: the pgoutput ingest path must preserve
text-value identity with `pg.rs` backfills, the xid `SnapshotGate` fence, schema-drift retirement,
bounded transaction spill/chunking, append-before-acknowledgement, and the sequencer's
transaction-end hold/high-water behavior.  A package rearrangement cannot reassign or weaken any
of those owners.

## Forward simulation 1 — “split pgoutput/query/domain into crates”

### What the skill leads an author to do

1. Classify this as package topology, module ownership, dependency direction, and public facade
   work; inventory consumers and the actual graph first.
2. Reject the requested bulk crate split for now.  The skill's “smallest durable boundary” rule
   defaults to a private module facade, and the extraction gates are not met.
3. Make the package's `lib.rs` a curated supported API, redirect integration tests to that API,
   then narrow visibility and move one dependency edge at a time.  A plausible *private* first
   layout is `domain` (`table_ref`, lower-level scalar/type definitions), `query`
   (`predicate`, schema compilation, SQL rendering), and `adapters::pgoutput`; it is a namespace
   and ownership change, not three new packages.
4. Only reconsider a crate after a stable, independently testable API and a measured build/reuse
   win exist; retain the binary as the composition root and keep adapter dependencies pointed
   inward.

### Why a crate split is presently unsafe/unjustified

- `pgoutput.rs` is a small protocol decoder and is dependency-light in implementation, but its
  observed `Relation` metadata is deliberately paired with `schema::SchemaFingerprint` by the
  replication adapter.  It has no demonstrated external consumer, package ownership, or build
  benefit.  Keeping `pgoutput` private under an adapter facade preserves that one-way seam without
  publishing wire implementation details.
- “Query/domain” is not one already-acyclic, dependency-light unit.  `schema` and `value` refer
  to each other; `predicate` depends on schema, values, table refs, and heap accounting; and
  `sql` currently imports `pg::quote_ident`.  Extracting the named files wholesale would either
  manufacture a cycle or pull Postgres/dbsp-derived representation into a purported domain crate.
  The first safe move is therefore a module-level contract that moves one shared low-level type or
  SQL escaping edge downward, without adding a trait just to hide the cycle.
- `Value`/`Row` deliberately carry dbsp archive/`SizeOf` derives for arrangements.  Calling that
  whole representation an independent `engine-domain` package today would violate the skill's
  no-heavy-dependency gate.  A future extraction needs an evidence-backed split between wire/domain
  values and dbsp storage types, not a cosmetic move.
- The engine kernel, subquery registry, catalog, sequencer and lifecycle remain one atomicity and
  durability owner.  The skill and repository map correctly prevent moving any of them merely to
  reduce source-file size.

### Assessment

**Pass.**  “Default to a private module facade,” the all-gates extraction rule, “move one edge at a
time,” and the repository map's dependency arrow all lead to the smallest safe boundary.  The
skill neither demands nor rewards needless crate splits, and its required graph/public/measurement
checks protect dependency direction and public API intent.

## Forward simulation 2 — “add a platform feature and establish MSRV”

### What the skill gets right

The current repository has a real platform seam: `txn_buffer.rs` uses Unix filesystem extensions
and `libc::{getuid, kill, EPERM}`, while `shutdown.rs` and `statsd.rs` already use `#[cfg(unix)]`/
`#[cfg(target_os = "linux")]`.  The skill points an author to a small platform port, target
dependency tables, `#[cfg]` rather than `cfg!` where the other branch cannot compile, and a
conservative fallback or explicit unsupported-platform result.  It also forbids using a feature
to choose a correctness or serving tier, correctly preserving the engine's invariant surface.

For MSRV and reproducibility it correctly says to preserve the 1.96.0 pin, establish MSRV only as
a dedicated task, declare/inherit it consistently, keep `Cargo.lock`, use `--locked` in
reproducible CI/container paths, and isolate resolver/toolchain/dependency refresh work.  That is
particularly important here: CI and the two engine Docker builds currently invoke Cargo without
`--locked`, and the virtual workspace explicitly stays on resolver 2 despite edition 2024.

### Assessment

**Pass with P1 revisions below.**  The semantic and topology decisions are right.  The missing
piece is a concrete acceptance matrix: “compile coverage for each branch” does not guarantee that
the minimum feature set, all features, the non-host platform, and the declared minimum compiler are
actually run.

## Findings and minimal fixes

### P0 — none

No skill instruction directs a structural change that would break an engine invariant, force a
crate split, select a serving/correctness mode through features, or silently float the known-good
compiler.

### P1 — make platform/feature coverage executable

**Exact references:** `SKILL.md` §3, “Features are additive…” and “A platform-specific behavior…”;
`references/decision-checklists.md` §“Feature, platform, build-script, or macro change,” first two
bullets; `references/official-sources.md`, Cargo features and platform-specific dependencies.

**Gap:** Neither section names the acceptance matrix.  A host-only default `cargo test` can leave a
new optional feature disabled and an alternate target branch uncompilable.  It also does not state
the Cargo limitation that feature predicates cannot select a target dependency table.

**Minimal fix:** Append this paragraph to the §3 platform bullet (and mirror it in the checklist):

> Record the supported feature/target matrix before editing.  For a new additive feature, run
> `cargo check -p electric-circuits-engine --all-targets --no-default-features --locked`, the
> intended named-feature combinations (including `--all-features` where valid), and test its public
> outcome on a supported host.  For each non-host platform branch, install/pin the target and run
> `cargo check -p electric-circuits-engine --target <triple> --locked`; run behavior tests on a
> real supported host or record that this is unavailable.  Inspect feature activation with
> `cargo tree -e features --target <triple>`.  Target dependency tables select targets, not
> `cfg(feature = …)`; use optional dependencies plus `[features]` for an optional capability.

This is proportionate: it needs no feature-powerset tool and does not require cross-platform
runtime emulation, but it makes every compile branch observable.

### P1 — distinguish and prove MSRV, rather than merely preserving a pin

**Exact references:** `SKILL.md` §3, “Preserve the pinned toolchain…”; §4, “toolchain effects”; and
`references/decision-checklists.md` §“Toolchain, lockfile, or dependency change,” first and second
bullets.  Relevant primary source: Cargo `rust-version` applies to all package targets; the
workspace source permits `workspace.package.rust-version` inheritance.

**Gap:** “Declare and inherit [MSRV] consistently” does not explicitly require the author to say
whether the exact pinned toolchain *is* the MSRV, set `rust-version` in this virtual workspace,
or compile the declared minimum.  The present 1.96.0 ICE workaround could otherwise be labelled
MSRV without proof.  It also leaves a lockfile-only check in a new MSRV task discretionary even
though the workspace has a committed lockfile.

**Minimal fix:** Replace the final two sentences of the §3 toolchain paragraph with the following
(or add them immediately after it):

> An exact toolchain pin and MSRV are separate declarations: record their relationship explicitly;
> never infer an MSRV from a pin.  An MSRV-establishment task sets
> `[workspace.package] rust-version`, has every member inherit `rust-version.workspace = true`,
> and identifies any intentionally unsupported target.  It proves the exact declared compiler with
> `cargo +<msrv> check --workspace --all-targets --locked` (and the supported feature/target matrix)
> before calling the policy established; a missing compiler or target is `blocked`, not evidence.
> Keep `Cargo.lock` committed; use `--locked` for the normal CI and image commands, and review any
> resolver or lockfile refresh separately from this policy task.

This asks for a one-time policy proof, not routine duplicate toolchain work on every refactor.

### P2 — make the requested dependency direction unambiguous in one sentence

**Exact reference:** `SKILL.md` §2, “Keep dependencies flowing from contracts/domain types toward
engine kernel, then adapters, then HTTP/Electric composition.”

**Gap:** The intended direction is clear when read with `repository-map.md`, whose arrows mean
“depends on,” but the prose can be read backwards during an extraction proposal.

**Minimal fix:** Replace that sentence with: “Dependencies point inward: HTTP/Electric and
Postgres/streams/dbsp adapters depend on the engine kernel; the kernel depends on query/domain
contracts; contracts/domain types import no adapter merely to name a type.”  Keep the existing
port-ownership sentence.

## Requirement coverage after the fixes

| Requested property | Result |
| --- | --- |
| Smallest safe crate/module boundary | Pass: private facade first; extraction gates prevent an aesthetic split. |
| Dependency direction | Pass, with P2 wording clarification; the two simulations preserve adapters → kernel → domain. |
| Public API and SemVer | Pass: deliberate `lib.rs`, supported facade tests, explicit package/publication intent, rustdoc evidence. |
| Feature semantics and platforms | P1: semantics are guarded; add exact feature/target compile evidence. |
| Lockfile/toolchain/MSRV | P1: lock/pin guidance is correct; require a declared, inherited, compiler-proven MSRV and `--locked` rollout. |
| Evidence and invariant coverage | Pass for structural changes; P1 adds the missing matrix proof. |
| Avoid needless crate splits | Pass. |

## Review-only validation

No source or manifest was changed, so build/test gates were not run.  The note is based on the
read-only inventory and primary-source review above.  The working tree was already dirty
(`AGENTS.md`, `.agents/`, and `notes/`); this review changes only this note.

## Re-review (2026-08-23) — PASS

Re-read the current `AGENTS.md`, the hardened
`.agents/skills/electric-circuits-rust-structure/SKILL.md`, and its current decision checklist and
repository map.  Every finding from the initial review is resolved:

- **Former P1: feature/platform evidence.**  The skill now requires a deliberately supported
  feature/target matrix before editing; host `--no-default-features` and supported named feature
  rows, `--all-features` only when supported, per-target cross-compilation, real-host public-outcome
  testing, and target-specific feature-tree inspection.  It explicitly says not to demand a
  feature powerset, and correctly distinguishes target dependency selection from feature selection.
  The matching checklist carries the same requirements.
- **Former P1: MSRV/reproducibility evidence.**  The skill now distinguishes the exact compiler pin
  from an MSRV; makes MSRV establishment a dedicated task; requires workspace `rust-version`
  declaration and member inheritance, an explicit unsupported-target statement, the exact
  `cargo +<msrv> check --workspace --all-targets --locked` proof plus the selected matrix, and
  `--locked` for normal reproducible CI/container paths.  The matching checklist carries the same
  policy.  This is proportionate: it adds a policy-proof lane only to MSRV work, not to unrelated
  refactors.
- **Former P2: dependency direction.**  The skill now states the direction unambiguously:
  HTTP/Electric and Postgres/streams/dbsp adapters depend on the kernel; the kernel depends on
  query/domain contracts; contracts/domain types do not import adapters merely to name types.

Command-policy checks were read-only.  `cargo check --help` confirms `--all-targets`,
`--no-default-features`, `--all-features`, `--target`, and `--locked`; `cargo tree --help` confirms
`--target`, `--edges`, and feature-selection flags.  All local Markdown links in `AGENTS.md`, the
skill, and its three references resolve.  The commands correctly separate compile evidence from
real-host behavior evidence and preserve the existing no-needless-split/default-private-facade
policy.

**Remaining severities:** P0 none; P1 none; P2 none.  No engine code or manifest changed in this
re-review, so implementation test gates were not applicable.  **Disposition: PASS.**
