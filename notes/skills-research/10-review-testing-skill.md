# Forward review: `electric-circuits-testing` skill

Status: **REVISE** — one P1 provenance clarification is required; the tiering and safety controls
otherwise pass this forward test. This is a skill-policy review, not production evidence or a task
registry.

Reviewed: [AGENTS.md](../../AGENTS.md); the complete
[`electric-circuits-testing` skill](../../.agents/skills/electric-circuits-testing/SKILL.md) and its
three routed references; canonical [note 18](../18-production-readiness-spec-reviewed.md); Swift/app
map [note 23](../23-swift-app-e2e-tdd-map.md); and PG18/E2E addendum
[note 24](../24-postgres18-and-e2e-tdd-addendum.md).

## Forward-test result

| Check | PG18 snapshot/live generated-column case | Pure `pgoutput` decoder regression | Result |
| --- | --- | --- | --- |
| Tier selection | Select target real-stack qualification: `PG18-E2E-003` proves published stored-generated snapshot/live values; `PG18-E2E-004` proves virtual generated-column admission fails before readiness/feed creation. `PG18-001Q` may use its isolated real reference materializer; final public claim remains `PG18-003Q` through immutable images, gateway, and target materializer. | Start with a focused byte corpus plus unit/property/model and fuzz evidence; minimize and retain the failing corpus. Add one real-PG/engine integration only if the decoded behavior participates in a public promise. | Pass — no manufactured E2E for a local codec. |
| Genuine red provenance | Freeze scenario, exclusions, source journal, SQL oracle and hash; capture the existing virtual-column acceptance/live-`null` divergence at its public assertion before the admission/decode repair. | Add the unchanged decoder corpus before the fix and record its intended parse/semantic divergence, replay command and corpus hash. | **P1 wording ambiguity** below. |
| Independent same-prefix oracle | For the stored-column positive, source DML and final `SourceCommitID` share one PG18 transaction; checked-in SQL/projection/key logic holds that prefix and detects generated value, missing-vs-`NULL`, predicate and delete errors without importing the production compiler. The virtual-column negative instead asserts the stable pre-readiness rejection, so it has no fictitious client receipt. | The independent oracle is the separately authored decoded-message expectation/property, not a reimplementation copied from the decoder. It needs no source-prefix SQL comparison unless promoted to a replication integration case. | Pass. |
| Server and target receipts | Require `source.committed` → `server.drainedThrough` (including deferred work) → a post-drain public read → principal/template/generation-specific cache/fold commit and `appliedTailAfter`. A separate sentinel, offset, LSN, byte arrival, or current SQL after later writes cannot substitute. | No target receipt is required for the pure parser law. If the regression reaches the live product promise, reuse the full fence rather than treating parsed bytes as client application. | Pass. |
| Fault gates and isolation | Hold snapshot creation after repeatable-read fixation, commit the named transaction, then release; use named arrival/hold/release/terminal gates and diagnostic deadlines. Pin PG18 and candidate digests; own DB/schema, slot/publication, DS volume, gateway namespace, cache, ports, journals and cleanup. | Use deterministic corpus splits/malformed bytes and seeded fuzz/property cases. Any process/network cut belongs to the later integration tier and gets the same named-gate and owned-fixture rules. | Pass. |
| Qualification and retry discipline | The target lane rejects PG16/default Compose substitution, requires file-backed storage and immutable inputs, and treats every retry-pass as flaky failure. | Deterministic focused tests must not hide a red/green result with retries; release qualification is unnecessary unless the selected profile claims the affected public behavior. | Pass. |

The skill does not overuse E2E. Its opening exclusion, tier table, and workflow send codecs/parsers to
focused evidence and reserve black-box E2E for a real replication, durability, gateway, cache, or
release boundary. The PG18 case correctly has two contracts: a causally fenced positive for stored
generated values and an early-rejection negative for virtual generated columns.

## Findings and minimal fixes

### P0

None. The routed material requires a pinned PG18 target profile, independent same-prefix oracle,
three-stage receipt, named external gates, full fixture ownership, and zero-retry qualification.

### P1

**Red tree is called a “candidate,” which can be mistaken for the later green candidate.**

- Exact sections: [`SKILL.md` — Workflow, item 3](../../.agents/skills/electric-circuits-testing/SKILL.md)
  and [`references/contract-protocol.md` — Genuine red-to-green provenance, step 2](../../.agents/skills/electric-circuits-testing/references/contract-protocol.md).
- Risk: the canonical requirement is the exact base/red patch before implementation. “Exact
  candidate” permits a reader to record a failure from a later source tree, defeating stacked
  red/green provenance even while preserving logs and hashes.
- Minimal wording fixes:

  - In `SKILL.md`, replace “run it against the exact candidate” with “run it on the exact
    frozen base/red patch, before the implementation exists.”
  - In `contract-protocol.md`, replace “Run the exact focused command against the candidate” with
    “Run the exact focused command on that frozen base/red tree.”

### P2

**Make the local-risk interpretation of “highest stable black-box contract” explicit.**

- Exact section: [`SKILL.md` — Workflow, item 2](../../.agents/skills/electric-circuits-testing/SKILL.md).
- Risk: surrounding text and the tier table already prevent E2E overuse, but this single sentence
  can be read as requiring an E2E-shaped test for every behavior change.
- Minimal wording fix: append “For a local-only risk, this is the focused unit/property/model
  boundary; do not add E2E merely to satisfy this step.”

## Conclusion

**REVISE.** Apply the P1 wording correction before relying on the skill for split-owner TDD or
qualification evidence. The P2 clarification is low-risk but makes the already-correct proportional
tier policy unambiguous.

## Re-review — 2026-08-23

Status: **PASS**. Re-reviewed the current hardened skill, `AGENTS.md`, note 18 `E2E-000A`, and
note 24's causal-fence specification.

- **Original P1 — resolved.** The skill now requires the exact frozen red-patch tree descended from
  the pinned base before implementation; the contract protocol uses the same term, preserves the
  red tree SHA, and requires the green candidate to descend from that patch.
- **Original P2 — resolved.** Workflow item 2 explicitly makes a local-only risk a focused
  unit/property/model boundary and prohibits adding E2E merely to satisfy the step. The tier table
  independently sends parsers/codecs to focused corpus/property/model/fuzz coverage and reserves
  real-stack E2E for a boundary promise.
- **Marker-publication/deferred-work P0 — resolved.** `E2E-000A`, note 24, `AGENTS.md`, and the
  contract protocol agree that the harness-only marker relation is in the immutable explicit test
  publication, excluded from public templates/client results, and not a separate sentinel feed.
  It is observed only after the terminal envelope; `server.drainedThrough` waits for every causally
  preceding direct and deferred action. The acceptance/mutation cases reject an unpublished or
  early marker and a receipt that skips deferred work.
- **No current-as-target claim.** The skill and `AGENTS.md` continue to label current direct
  surfaces, PG16 Compose, host-selected CI, and current conformance as characterization/regression
  evidence. PG18, file-backed storage, authenticated gateway, immutable candidate images, and the
  causal-fence harness remain target work requiring named evidence.

Remaining severities: **P0 none; P1 none; P2 none.**

**PASS.**
