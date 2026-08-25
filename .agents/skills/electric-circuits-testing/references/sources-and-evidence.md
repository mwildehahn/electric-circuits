# Sources and evidence

## Authority order

1. `AGENTS.md` is the repository's executable safety and completion guidance. Read its relevant
   invariants before altering engine, lifecycle, storage, replication, or a live client path.
2. `notes/18-production-readiness-spec-reviewed.md` is the task/profile authority. Sections 1–4
   define the target topology, profiles, gates and completion rules; section 11 defines the testing
   and qualification work; sections 13–14 define review/merge control and the current no-go state.
3. `notes/23-swift-app-e2e-tdd-map.md` supplies Swift/app scenario rationale and test seams.
   `notes/24-postgres18-and-e2e-tdd-addendum.md` supplies PG18 and server scenario rationale.
   They do not independently authorize task ownership or profile closure.
4. The existing test, fixture and CI files establish what runs now. The audit in
   `notes/skills-research/04-repo-test-harness-audit.md` is the current harness map; do not
   describe proposed acceptance infrastructure as present.

The testing doctrine and supporting design rationale are in
`notes/skills-research/03-testing-philosophy-sota.md`; packet, ownership, red-patch and independent
review rules are in `notes/skills-research/05-parallel-agent-execution-protocol.md`; skill design
provenance is in `notes/skills-research/06-public-skill-patterns.md`; current-state and foundation
risks are in `notes/skills-research/07-foundation-risk-audit.md`. These local instructions are
original repository prose, not copied from the public-skill research sources.

## Evidence record

For an ordinary change, record the selected test tier, command/result, changed behavior, oracle,
and any unrun gate with its reason. For a behavior-changing packet or qualification, also record:

- classification (characterization, implementation, or qualification), task/scenario IDs and
  semantic contract hash;
- exact source/tree/base SHA, selected profile and feature hash, candidate/image/config/toolchain
  identities, fixture namespace, and seed/journal/corpus hashes;
- canonical clean-runner identity, fresh detached-worktree or verified-tree-export identity, pre/post
  source-attestation hashes, immutable external-input and resolver/mount-topology hashes, unique
  initially empty run-root identity/attestation, and effective-environment/config hash;
- red patch SHA and raw red evidence hash; then green evidence from the unchanged contract;
- source/server/target receipt tuple, independent oracle version, cut IDs, fixed counts/floors,
  deadlines, resource limits, raw artifacts and first divergence;
- cleanup outcome, remaining risks, and `pass`, `fail`, `blocked`, or validator-generated
  `not_applicable_by_profile` status; and
- an independent review result. A reviewer verifies evidence but does not modify the behavior it
  is evaluating. Integration/qualification work needs the designated cross-boundary review.

Only a selected profile's generated scheduler/manifest can mark a task not applicable. A blocked
external prerequisite is useful evidence, never completion. Changed source, contract, profile,
digest, configuration, toolchain, workload, or allowlist invalidates the affected qualification.
