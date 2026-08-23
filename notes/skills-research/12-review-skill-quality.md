# Skill-quality review — Rust code, Rust structure, and testing

**Verdict: REVISE.** The three skills have clear routing, compact executable checklists, working references, and strong current-versus-target boundaries. No P0 issue was found. One P1 trigger contradiction can make an agent skip Rust Code on a read-only Rust review even though `AGENTS.md` requires it.

## Scope and method

Reviewed on 2026-08-23 against working-tree commit `474577a088b95c746bd9ab2c8e4b6552a72f151f`; pre-existing dirty and untracked files were not altered. Scope covered `AGENTS.md`, every file in these three skills (three `SKILL.md`, seven routed references, and three `agents/openai.yaml` files), plus the cited local evidence.

- Verified every local route target named by the skills. The `AGENTS.md` serving-tier anchor and the structure map's `ARCHITECTURE.md` §14 reference both exist.
- Parsed all three `openai.yaml` files as YAML and verified string `interface.display_name` and `interface.short_description` fields.
- Verified each skill has `name` and discriminating `description` front matter plus one H1.
- Verified the current toolchain, manifests, CI workflow, engine file map, package scripts, note 18, notes 23/24, and the cited skills-research authority files.
- Requested all 30 Rust/Tokio/Cargo URLs in the code and structure references; each returned HTTP 200 on 2026-08-23.

The Agent Skills-format finding is deliberately limited to observable local conventions. Official OpenAI material available to this audit establishes that Codex skills are supported artifacts, but did not expose a public schema for this repository's `agents/openai.yaml`; this is not a stronger vendor-schema certification. Project-specific claims are traceable to explicit local authority, while Rust/Cargo/Tokio guidance is traceable to the primary sources listed in the references.

## What passes

### Trigger coverage and overlap

`AGENTS.md` has a sound three-way dispatch: Rust implementation/debug/refactor/review under `apps/engine` selects Rust Code; Cargo/module/public-boundary/toolchain decisions additionally select Rust Structure; any behavior/test/qualification work selects Testing before implementation. The intended overlap and order for behavior-changing Rust structure work—Testing → Structure → Rust Code—are explicit. Testing correctly covers Rust, TypeScript, protocol, and real-stack work, so test location does not incorrectly follow implementation language.

### Progressive disclosure and completion criteria

The entrypoints are 62, 109, and 80 lines. They retain activation, decision gates, and handoff evidence in `SKILL.md`, and place detailed material in named references. Structure's checklist explicitly routes to only the matching decision section. Code and Testing distinguish current required gates from conditional hardening and give measurable completion records: commands, candidate, oracle/receipts, limits, raw artifacts, cleanup, and unrun-gate reasons.

### Current versus target, evidence, and links

Testing unambiguously says the repository is not production-ready; it labels PG16/direct-engine/current-CI work as characterization and names digest-pinned PG18, file-backed storage, and an authenticated gateway as target. That agrees with `AGENTS.md`, note 18, and notes 23/24. Rust Code's 1.96.0 pin and current CI claims agree with `rust-toolchain.toml`, manifests, and `.github/workflows/ci.yml`; Structure's one-package/edition-2024/resolver-2 map agrees with the current workspace and source tree.

`production-rust.md` and `official-sources.md` point to primary Rust/Cargo/Tokio documentation, with first-party project sources for Miri and Rust fuzzing. Testing's source ledger defines its local authority order and distinguishes repository-original doctrine from public-skill research. No dead local path, broken anchor, unreachable cited technical source, or target-as-current claim was found.

### Agent Skills formatting and metadata

All three skills use valid slug names, focused descriptions, YAML front matter, and one top-level H1. The metadata files parse and consistently provide the two observed interface fields. Their display names and short descriptions agree with the front-matter scope; no misleading UI metadata or metadata/scope conflict was found.

## Findings and minimal fixes

### P0 — none

No missing mandatory route, invalid local link, false production-ready claim, broken required-command reference, or malformed skill/metadata file was found.

### P1 — Rust Code's exclusion conflicts with mandatory Rust-review routing

**Evidence.** `AGENTS.md` lines 37–39 require Rust Code for *any* review of Rust under `apps/engine`. Rust Code's “Do not use when” section says not to use it when one “only need[s] test strategy or a read-only invariant audit.” A read-only invariant audit can be a Rust review, so the activation instructions disagree. `AGENTS.md` wins by authority, but the skill tells the agent the opposite action.

**Minimal fix.** Replace that bullet with:

> The work is only test strategy or a read-only audit **and does not review Rust implementation**. Use the dedicated testing/audit workflow when available. If it reviews Rust under `apps/engine`, this skill remains required by `AGENTS.md`.

### P2 — Make cross-skill order visible from Rust Code

**Evidence.** The mandatory order appears only in `AGENTS.md`. An agent who enters via Rust Code can see TDD advice but not the explicit requirement that Testing precede a behavior change or that Structure also applies to a boundary decision.

**Minimal fix.** Add to Rust Code's introductory paragraph:

> For a behavior change, read `electric-circuits-testing` first; for a structural decision, also read `electric-circuits-rust-structure`, in the order required by `AGENTS.md`.

### P2 — Tighten Rust Code's first-read disclosure

**Evidence.** Rust Code requires the whole 111-line `production-rust.md` before every edit, then says to use risk-specific sections. It remains a modest reference, but a parser-only change must initially consume task, unsafe, observability, and full-gate guidance.

**Minimal fix.** Change “Read [the production checklist] before editing” to “Read its Boundary review, Test and evidence selection, and Validation matrix before editing; then load the applicable Errors/tasks/bounds/unsafe/observability section for the risk.” The reference itself need not change.

### P2 — Define the metadata-validation boundary once

**Evidence.** The metadata is valid and consistent, but neither the skills nor local research names an authoritative schema or a repository lint command for `agents/openai.yaml`. Future field requirements would be discovered only at install time.

**Minimal fix.** Add to repository skill-maintenance guidance:

> Validate `agents/openai.yaml` against the Codex-distributed schema/tool when available; until then require valid YAML plus `interface.display_name` and `interface.short_description`.

This avoids claiming that the observed two-field shape is the complete vendor schema.

## Acceptance after revision

The P1 wording change is sufficient for a PASS: re-run YAML parsing, local-link/anchor verification, and a routing read-through confirming a read-only `apps/engine` Rust review no longer receives conflicting instructions. The P2 clarifications can accompany the documentation-only update; they need no runtime test.

## Re-review — 2026-08-23

**Verdict: REVISE.** The prior content findings are resolved, and no new trigger or
progressive-disclosure contradiction was found. A new P1 applies only to the required skill
maintenance validation path: the installed validator cannot execute in the present environment.

### Re-checked scope and passing evidence

- Re-read all current `SKILL.md`, `references/`, and `agents/openai.yaml` files for the three
  skills, plus `AGENTS.md` routing.
- The prior P1 is fixed: Rust Code now excludes a read-only audit only when it does **not** review
  Rust, and expressly defers to `AGENTS.md` for an `apps/engine` Rust review.
- The prior P2 order ambiguity is fixed: Rust Code now tells a behavioral task to read Testing
  first and a structural task to also read Structure in the `AGENTS.md` order.
- The prior P2 disclosure issue is fixed: Rust Code's initial reference route is limited to
  Boundary review, Test and evidence selection, and Validation matrix, then names the
  risk-specific sections to load.
- The prior metadata-boundary issue is fixed in `AGENTS.md`: it now specifies the fallback YAML
  requirements and directs use of the Codex-distributed schema/tool when one is available.
- Every current local route target, including the new deep Architecture/ADR links, exists; the
  Architecture headings match their anchors. The new append/shutdown guidance is traceable to
  `replication.rs`, `ds.rs`, `sequencer.rs`, `ARCHITECTURE.md`, and ADR-0003.
- Each metadata file parses; all strings are quoted; each `short_description` meets the documented
  25–64 character range (35, 37, and 35 characters); and the display/short descriptions remain
  consistent with the skill's front-matter scope. Optional `default_prompt`, icons, dependencies,
  and policy are not required.
- Current-versus-target language remains correct. The new causal-fence marker details explicitly
  identify themselves as target `E2E-000A` infrastructure rather than a current implementation.
  The Rust 1.96.0, one-workspace-member, edition-2024, resolver-2, and current-CI statements still
  match the checked-in toolchain, manifests, and CI workflow.

### P1 — the mandated installed validator is not runnable

**Evidence.** `AGENTS.md` now requires maintainers to run the installed skill-creator validator.
The installed `quick_validate.py` has mode `0644`, so invoking it as documented returns
`permission denied`; invoking it with its declared Python interpreter reaches
`ModuleNotFoundError: No module named 'yaml'`. The validator therefore supplied no pass evidence,
even though independent Ruby YAML parsing and the documented local metadata checks passed.

**Minimal fix.** Make the validator runnable in the supported agent environment: ship an executable
wrapper or document the interpreter invocation, and provide its `PyYAML` dependency in that same
environment. Until then, amend the maintenance instruction to require the available fallback
(front-matter/YAML parse, field checks, and local-link verification) and record the validator as
blocked rather than implying it was run.

### P0/P2 status

There is no P0 finding and no remaining P2 finding from the initial review. No new activation,
skill-overlap, or progressive-disclosure contradiction was found. Once the P1 validator path is
made executable with its dependency, this re-review becomes **PASS**.
