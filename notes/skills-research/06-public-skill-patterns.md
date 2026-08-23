# Public skill patterns for the Rust engine — 2026-08-23

## Scope, method, and reuse caution

This is pattern research, not a proposal to import a skill pack or copy its prose. Sources below
were read on 2026-08-23 and are pinned to the listed Git commit where practical. Licenses are
reported only as visible in the linked repository/file; a permissive repository license does not
prove that every vendored example, generated artifact, or upstream-derived paragraph has the same
provenance. Write original repository-specific instructions and preserve attribution/notice if any
source text is ever reused after legal review.

The local baseline remains [01-rust-code-sota.md](01-rust-code-sota.md) plus this repository's
`AGENTS.md`. Its durable-engine invariants are more authoritative and much more specific than any
public generic Rust skill.

## Public-source observations

| Source (exact repository/file) | Visible license/provenance | Strong pattern worth learning | Limits / reuse caution |
| --- | --- | --- | --- |
| [Open-Dot-Agents/SKILL.md `README.md`](https://github.com/Open-Dot-Agents/SKILL.md/blob/69ef37e9424c0a7ea9dd2293b559e43ec8176379/README.md), [license](https://github.com/Open-Dot-Agents/SKILL.md/blob/69ef37e9424c0a7ea9dd2293b559e43ec8176379/LICENSE) | Root license is Apache-2.0; the README distinguishes code (Apache-2.0) from documentation (CC-BY-4.0). | Treat a skill as a directory with a small `SKILL.md` discovery/activation surface and optional `references/`, `scripts/`, and `assets/` loaded only when needed. This is the right portability and progressive-disclosure shape. | This is a format/specification source, not evidence that a workflow is correct for a replication engine. Do not infer a blanket license for prose from the root license; check the specific material. |
| [bcgov/agent-skills `SKILL_SPEC.md`](https://github.com/bcgov/agent-skills/blob/4eebd779e9d224f99fb574d57d5276fb3c40eb01/spec/SKILL_SPEC.md), [license](https://github.com/bcgov/agent-skills/blob/4eebd779e9d224f99fb574d57d5276fb3c40eb01/LICENSE) | Apache-2.0 repository license. | Make the description discriminating because it is the routing signal; require an H1 plus **Use When**, **Don't Use When**, **Workflow**, **Rules**, **Examples**, **Edge Cases**, and **References**. Enforce a size limit and move detail to references. It also distinguishes contributed `skills/` from repo-operational `.github/skills/`. | Its exact seven-section schema, 500-line cap, one-level resource restriction, and validator are implementation choices—not a standard this repo needs to copy. The useful idea is CI-linted, non-duplicated, skimmable manifests. |
| [full-stack-skills/rust-skills `rust-concurrency`](https://github.com/full-stack-skills/rust-skills/blob/25e44452df00055ca246ec806425d99028eaae19/skills/rust-concurrency/SKILL.md), [rust-testing](https://github.com/full-stack-skills/rust-skills/blob/25e44452df00055ca246ec806425d99028eaae19/skills/rust-testing/SKILL.md), [rust-module-layout](https://github.com/full-stack-skills/rust-skills/blob/25e44452df00055ca246ec806425d99028eaae19/skills/rust-module-layout/SKILL.md), [license](https://github.com/full-stack-skills/rust-skills/blob/25e44452df00055ca246ec806425d99028eaae19/LICENSE) | Apache-2.0 repository license (copyright notice: PartMe.AI). | Split Rust expertise by decision type rather than one monolith. Each sampled skill states capabilities, prerequisites, out-of-scope routing, concrete trigger phrases, a procedure, and gotchas. The concurrency workflow's sequence—classify workload, name budgets, decide ownership, choose queue semantics, supervise tasks, design shutdown, then measure—is a particularly good *shape*. | Some skills are long reference manuals despite the progressive-disclosure goal; descriptions are broad enough to collide. The testing file's stated capabilities/out-of-scope boundary is internally inconsistent about property testing. Use as an index of topics, not an unquestioned authority or copied text. |
| [rtk-ai/rtk `rtk-tdd/SKILL.md`](https://github.com/rtk-ai/rtk/blob/29f9bb7161775cd807565fd3041eb2b7d1be071c/.claude/skills/rtk-tdd/SKILL.md), [license](https://github.com/rtk-ai/rtk/blob/29f9bb7161775cd807565fd3041eb2b7d1be071c/LICENSE) | Apache-2.0, copyright rtk-ai and rtk-ai Labs. | A short Rust-specific Red → Green → Refactor loop gives observable exit conditions: run a focused test and see the intended failure, make the smallest change, then run repository gates (`fmt`, `clippy`, tests). Its trigger explicitly names implementation, testing, refactoring, and bug fixing. | “Tests in the same file” is too rigid: public behavior, cross-crate/protocol, real-process, and conformance tests belong elsewhere. The skill lacks distributed failure, recovery, and concurrency-interleaving evidence; do not adopt the three-law formulation as a substitute for a test strategy. |
| [arjunprabhulal/agent-skills `test-driven-development/SKILL.md`](https://github.com/arjunprabhulal/agent-skills/blob/42dd24080fce6d731d00e2a1134f398c3da4171b/skills/qa/test-driven-development/SKILL.md) | The skill frontmatter declares MIT; no root `LICENSE` was found by this research pass. **Reuse status uncertain beyond that declaration.** | The strongest testing guidance sampled: test a caller-visible boundary; prefer real collaborators; mock only slow, nondeterministic, or external collaborators; for a bug, prove the regression fails for the intended reason and, where feasible, revert the fix to prove causation. It correctly names exploration, judged outputs, and thin adapters as TDD exceptions. | The instructions are useful methodology but the provenance is incomplete. Do not copy prose until the license is resolved. “Prefer real collaborators” must still accommodate deterministic fault injection and controlled clocks/transports. |
| [addyosmani/agent-skills `test-driven-development/SKILL.md`](https://github.com/addyosmani/agent-skills/blob/5a5ea45e806f82273549fd85e60adb95d55f510d/skills/test-driven-development/SKILL.md), [license](https://github.com/addyosmani/agent-skills/blob/5a5ea45e806f82273549fd85e60adb95d55f510d/LICENSE) | MIT. | **Discover the repository's test stack before the first RED step**: use its wrappers, focused command, full suite, conventions, and CI gates rather than a guessed default. Its distinction between small/medium/large tests and outcome assertions rather than interaction assertions is useful. | Its automatic trigger (“any logic or behavior”) is too broad for a compound engine change. The 80/15/5 pyramid is a generic heuristic, not a measured target for this repository; do not turn it into a quota. |
| [outcomeeng/plugins `architect-rust/SKILL.md`](https://github.com/outcomeeng/plugins/blob/3d371e091bd11a900d931e5661eeeb91f54bc9c2/src/plugins/rust/skills/architect-rust/SKILL.md), [test-rust](https://github.com/outcomeeng/plugins/blob/3d371e091bd11a900d931e5661eeeb91f54bc9c2/src/plugins/rust/skills/test-rust/SKILL.md), [audit-rust-tests](https://github.com/outcomeeng/plugins/blob/3d371e091bd11a900d931e5661eeeb91f54bc9c2/src/plugins/rust/skills/audit-rust-tests/SKILL.md), [license](https://github.com/outcomeeng/plugins/blob/3d371e091bd11a900d931e5661eeeb91f54bc9c2/LICENSE) | MIT, copyright Simon Heimlicher. These files rely on the repository's own templating/skill-composition system. | Separate authoring from a read-only evidence audit. The audit asks whether a test has an independent oracle, reaches the governed path, is falsifiable by a named mutation, and covers each assertion clause—excellent review questions for protocol/conformance tests. The architecture skill also usefully asks explicitly about ownership, error model, concurrency, lifecycle, unsafe boundary, security, and testability. | The pack is highly coupled to its `spec-tree` router and nonstandard template syntax. “Every module needs an ADR,” “no mocking,” and its test-location rules are over-prescriptive for this repo. Adopt the questions, not the policy or framework. |
| [leonardomso/rust-skills `SKILL.md`](https://github.com/leonardomso/rust-skills/blob/fd2a861ab0406a4ac536a55274d14ea6fd1ca9c9/SKILL.md), [license](https://github.com/leonardomso/rust-skills/blob/fd2a861ab0406a4ac536a55274d14ea6fd1ca9c9/LICENSE) | MIT, copyright Leonardo Maldonado. | A lightweight index of one-line rules keyed by topical prefixes, with separate leaf files, is a strong answer to a large language-specific knowledge base. A concurrency or unsafe review can load only the relevant rules. | Its catch-all description (“writing, reviewing, or refactoring Rust”) guarantees routing overlap and risks generic advice overriding local toolchain/invariant knowledge. Many rules are valid only with repository context; no rule list proves durable protocol correctness. |

### Common organization patterns

1. **A description is a routing contract, not marketing.** The best descriptions include both the
   object of work and discriminators: `Tokio channel backlog`, `failing test`, `ADR`, `compile-fail`,
   or `test evidence audit`. They also name adjacent tasks to defer.
2. **Keep the entry file operational.** Put objective, prerequisites, ordered workflow, hard rules,
   failure/edge cases, completion evidence, and direct references in `SKILL.md`; keep tables,
   long examples, commands, and templates under `references/` or scripts.
3. **Compose deliberately, do not duplicate.** A small reference standard can be loaded by a
   change workflow and a review workflow. It must not be a user-facing catch-all trigger itself.
4. **Make completion falsifiable.** “Tests passed” is inadequate. Good instructions identify the
   exact behavioral contract, expected red state, changed mutation that would go red, relevant
   focused/final commands, and any unrun evidence.
5. **Separate mutation from adjudication.** An implementer can add tests; a read-only reviewer can
   judge target, coupling, oracle independence, and gaps without rationalizing its own work.

### Recurrent public anti-patterns

- A general “Rust expert” description alongside several narrower Rust skills; all will activate.
  Give one router/entry skill a narrow role or make it non-user-invocable.
- A single giant `SKILL.md` that repeats the Rust Book or a style guide. It consumes context,
  becomes stale, and hides the few rules relevant to the current risk.
- Prescribing `cargo test`, test placement, crates, mocking, CI tools, or Rust version without
  first reading this repository's `AGENTS.md`, toolchain, package layout, existing tests, and CI.
- Treating a conventional unit-test loop as proof of a crash/restart, replay, transaction-order,
  or multi-process protocol property.
- Requiring TDD universally. Spikes, migrations, generated code, thin wiring, and diagnosis need
  different evidence; they must still finish with an appropriate regression/property/integration
  proof when behavior changed.
- Letting a writing skill self-certify test quality. Keep evidence review read-only and preserve
  failed/blocked command output rather than silently weakening assertions or skipping tests.
- Importing an entire public pack (or copying its prose) without resolving per-file provenance,
  license, update cadence, and mismatch with the engine's Rust 1.96 pin.

## Representative sibling-repository layout (local observation)

This is an inspection of a small local sample, not an assertion that every sibling has the same
policy.

| Local source | What it shows |
| --- | --- |
| `/Users/bozilabs/labs/mighty.mh-organization/AGENTS.md` | The root instruction file is a persistent, concise-ish router: it sets the governing workflow (`mt prime`, graph/spec context, task closeout), cites its tool, and says how specs, decisions, and tasks relate. It is not a pile of language recipes. |
| `/Users/bozilabs/labs/mighty.mh-organization/skills/codex/mighty/SKILL.md` and `skills/codex/tasks/SKILL.md` | Skills are grouped first by host surface (`codex`, with a parallel `claude` tree), then domain. A top-level `mighty` skill handles session/context routing; a narrower child `tasks` skill owns a concrete workflow and points to `references/` for templates/examples. |
| `/Users/bozilabs/labs/mighty.3skm-admission/skills/agents/mighty/decomposition/SKILL.md` | Agent-oriented skills live under `skills/agents/` and use strong negative constraints, an explicit shared-surface ownership rule, dependency-gated fan-out, and a pre-dispatch checklist. The goal is prevention of coordination failure, not generic delegation encouragement. |
| `/Users/bozilabs/labs/mighty.3skm-admission/skills/agents/mighty/test-quality-review/SKILL.md` | A narrow, advisory, read-only reviewer is explicitly separate from the test author. It defines the test/guarantee relation, severity, exclusions, and a finding format instead of running broad implementation commands. |

The inferred layout to preserve is: **`AGENTS.md` owns durable repository truth and broad routing;
agent-surface folders own integration differences; narrow skills own one repeatable judgment or
workflow; references carry depth; separate agents own independent review.** This maps well to the
existing Electric instructions: `AGENTS.md` should continue to own the non-negotiable stream,
epoch, snapshot, retention, and test gates, while skills make that knowledge usable at the right
decision point rather than duplicating it.

## Recommendations for electric-circuits (original, repository-specific)

### Skill boundaries and trigger descriptions

Use a small hierarchy, not a public pack wholesale. The following are candidate boundaries; their
descriptions are deliberately discriminating and should be written anew if adopted.

| Candidate | Trigger description / boundary | What it should load or enforce |
| --- | --- | --- |
| `rust-engine-change` | “Use for implementation, debugging, or review under `apps/engine` that changes Rust lifecycle, replication, durable-streams, backfill, engine routing/circuits, cancellation, resource limits, or unsafe/FFI. Not for a docs-only edit or a TS-only client change.” | Read `AGENTS.md`, the engine architecture docs, toolchain/Cargo config, and the local change path before edits. Require an invariant/failure/cancellation/resource-budget note and correct code-first gates. It should defer generic Rust stylistic material to on-demand references. |
| `engine-protocol-test` | “Use when an engine change can affect emitted rows, transaction visibility/order, duplicate delivery, recovery, retries/gone reconciliation, shape lifecycle, schema drift, epoch/segment state, or boundedness. Use for regression/conformance/fault/interleaving test design; not merely for formatting.” | Select the cheapest evidence that can falsify the risk, then require the appropriate existing gates: focused Rust test, engine suite, TS typecheck/conformance, Electric oracle where available, and demo/browser check for live paths. It must know tests can span Rust, Postgres, durable-streams, TS client, and the UI. |
| `engine-invariant-audit` (read-only) | “Use to review a diff/test plan touching the same protocol boundaries before merge or after an incident. Do not edit.” | A separate reviewer applies an invariant-to-evidence matrix: promise, durable point, failure/retry outcome, cancellation point, recovery/replay behavior, bound, trace/metric, and independent oracle. Output concrete gaps plus exact AGENTS invariants. |
| `engine-architecture-decision` | “Use when selecting/changing a persistent state boundary, stream/log protocol, task ownership model, serving-tier structure, storage schema, or public client consistency contract; not for a local refactor.” | Produce a concise decision record with alternatives, invariants, failure/recovery behavior, migration/rollout, observability, and test plan. It should not claim every module warrants an ADR. |

Keep a generic Rust rules index optional and non-authoritative. `rust-engine-change` needs the
current toolchain pin and this service's safety model; a generic Rust skill is useful only after
that context is loaded.

### Durable-engine concerns public skills leave out

Generic Rust/TDD material needs a first-class supplement for these exact questions:

1. **Visibility and ordering:** What makes a multi-envelope Postgres transaction visible as one
   unit? Does `headers.last` remain correct across chunks/pages/holds, and is the checkpoint plus
   de-dup highwater advanced only at a safe durable boundary?
2. **At-least-once and recovery:** Which keys make repeated delivery harmless? Can a restart,
   chunk replay, a mid-append failure, or task cancellation duplicate a non-idempotent aggregate
   or skip a committed update?
3. **Read/live fence:** Does every new/read path use `SnapshotGate` xid visibility (with only its
   documented fallback), rather than an unsafe LSN shortcut?
4. **Durability promises:** Before a client sees create/join/delete success, what catalog record
   is durable, what happens if the future is dropped, and are generations/registration rechecked
   after a long wait? On an append terminal answer, does reconciliation prove stream retirement
   before any event is discarded?
5. **Lifecycle and epoch:** How do activation, dormant replay, close-then-delete retirement,
   schema drift, truncate, slot loss/reset, and segmented-log retention behave after interruption
   or concurrent create/join? Does any path silently recreate a slot/stream or re-mint an id?
6. **Boundedness and shutdown:** State numeric/semantic bounds for per-transaction buffers,
   queues, retry budgets, query-backs, task handles, and retained log segments. Prove slow
   consumers, cancellation, panic, and shutdown cannot leak work or violate correctness.
7. **Oracle independence:** For protocol changes, use the existing engine-vs-oracle/conformance
   machinery, differential state checks, deterministic fault injection, and real Postgres/stream
   boundaries where needed. A mock proving its own scripted result is not evidence of replication
   semantics.
8. **Operational proof:** Name the tracing/metrics fields and an incident/debug story for the new
   transition. Correctness paths without observability are difficult to audit after a partial
   outage.

### Recommended skill template

For each new local skill, write original content in this order:

1. YAML frontmatter: stable kebab-case name, a concrete `Use when` description, optional owner,
   compatibility/tooling, and license for the local text.
2. **Use when / do not use when:** include paths, failure modes, and clear hand-offs to another
   skill; avoid “all Rust work.”
3. **Read first:** the exact repository docs/config/tests it must consult. Place `AGENTS.md` and
   relevant architecture/ADR files before generic language advice.
4. **Workflow:** ordered decision points with stop/ask conditions—not a long prose primer.
5. **Rules and edge cases:** separate hard inherited invariants from context-dependent practices;
   explain the non-obvious failure prevented.
6. **Evidence and handoff:** command matrix, required runtime/fault/browser validation, expected
   output, and an honest `not run / why` rule. Link long command recipes and examples from
   `references/`.

Add a lightweight validation check once more than a few local skills exist: frontmatter parses,
name equals directory, descriptions are distinct, required hand-off/edge/evidence sections exist,
and references resolve. Do not impose an arbitrary line-count or folder-depth policy until the
actual loader requires it.
