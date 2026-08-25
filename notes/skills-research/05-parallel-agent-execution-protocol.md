# Parallel-agent execution protocol, v2

Status: operational protocol for the production-readiness DAG. It implements the scheduling rules in
[the canonical specification](../18-production-readiness-spec-reviewed.md), especially sections 2,
4, 5, 11, and 13. It is not a second task registry: the checked-in manifest produced by `PLAN-001`
is the only scheduler and authority for task IDs, dependencies, applicability, artifacts, owners,
and scenario hashes. Notes 23–25 are scenario rationale and review traceability only.

This protocol is directly usable for independent `gpt-5.6-terra` high agents once an authorized
integration commit contains this foundation and the canonical specification. A task may produce a
merge candidate only when its packet, base, legal scope, ownership, contract, and evidence inputs are
all pinned first. Parallelism must not trade correctness for speed.

## 1. Control plane and legal ready set

The manifest declares each task as exactly one of `shared_producer` (one profile-independent source
producer with profile-specific qualification consumers) or `per_profile` (a separately executable
task/profile pair). It rejects a task that reserves both forms concurrently. Every mutable record is
injectively named by `(task_id, execution_scope_id, attempt)`: a shared producer uses its
manifest-derived `shared-<source-scope-sha>` scope, while a per-profile task uses
`profile-<release-profile-canonical-sha256>`. `attempt` is monotonic within that pair.

The scope is mandatory in branches, notes, and leases:

```text
agents/<task-id>/<execution-scope-id>/a<attempt>
review/<task-id>/<execution-scope-id>/a<attempt>
leases/<task-id>/<execution-scope-id>/a<attempt>
notes/execution/<task-id>/<execution-scope-id>/a<attempt>.md
notes/execution/<task-id>/<execution-scope-id>/a<attempt>.resolution.json
```

The scheduler records a state for every task/execution-scope pair:

```text
declared -> ready -> reserved -> red_proved -> implemented -> reviewed -> integrated -> qualified
                         |             |              |             |
                         |             +-> characterized             +-> rejected
                         +-> blocked   +-> failed     +-> invalidated
```

`characterized` is distinct from `red_proved`: an inherited passing control is useful evidence, but
never genuine-red provenance. `invalidated` is terminal for an attempt; it is neither a test failure
nor an external blocker.

Only `PLAN-001` is initially merge-ready. Before it merges, agents may collect read-only inventory
facts, but cannot claim completion, change authoritative task metadata, or merge implementation. At
the actual post-`PLAN-001` integration SHA, the manifest—not section order—selects work. The reviewed
bootstrap sequence is:

1. `PLAN-001`.
2. Once the graph validates: `GOV-001` and `TST-000`.
3. After `GOV-001`: `CMP-000`, `GOV-002`, `GOV-003`, and `SEC-008B`, subject to the manifest.
4. `CMP-001` needs `CMP-000` and `GOV-002`; `PG18-000` needs `GOV-002`.

A branch, a green local command, review approval, or an execution note does not make a consumer
ready. An ordinary predecessor must be integrated and recorded `pass` in the profile-evaluated
manifest. The only exception is a manifest edge typed `red_artifact`: it unlocks only its named
implementation consumer after the scenario/scope-specific artifact is independently reviewed and
recorded `red_proved`; it never satisfies an ordinary dependency or release gate.
Only the validator can write `not_applicable_by_profile`; a required dependency cannot be N/A.
`blocked` is a named external dependency failure and `failed` is a test, count, hash, or
acceptance failure. Neither closes a task or unlocks consumers.

`PLAN-001` alone may reserve a discriminated bootstrap packet. It pins the canonical specification
and this protocol's blobs/tree plus the packet-schema version, uses `profile_scope: uncompiled_all`,
and records the future profile and scenario registries as typed unavailable outputs. It has
`profile: null` and `release_profile_hash: null` by rule: it must not claim a selected release-profile
hash, semantic scenario hash, red proof, or qualification result. The absent files it owns are exact
declared outputs, never inputs with fabricated blob hashes.

```yaml
packet_kind: bootstrap_plan
task_id: PLAN-001
execution_scope: { kind: bootstrap, id: bootstrap-plan-001, profile_scope: uncompiled_all }
bootstrap_inputs:
  canonical_spec: { path: notes/18-production-readiness-spec-reviewed.md, blob_sha: <sha>, tree_sha: <sha> }
  execution_protocol: { path: notes/skills-research/05-parallel-agent-execution-protocol.md, blob_sha: <sha> }
  packet_schema_version: 2
declared_outputs:
  task_manifest: docs/production/readiness-tasks.json
  task_schema: docs/production/readiness-task.schema.json
  gate_matrix: docs/production/readiness-gates.json
  validator: scripts/readiness-plan.ts
  validator_tests: scripts/readiness-plan.test.ts
  generated_report: docs/production/readiness-plan.generated.md
future_inputs:
  release_profiles: { kind: unavailable, owner: GOV-005, path: docs/production/release-profiles.yaml }
  scenario_registry: { kind: unavailable, owner: E2E-000S, path: docs/production/e2e-scenarios.json }
profile: null
release_profile_hash: null
```

The bootstrap packet also carries the ordinary pinned base/tree, exact owned output paths, direct
validator gates, scheduler generation/lease, reviewers, evidence schema and explicit Git
authorization from section 2. Their omission from this discriminator excerpt is not an exception.

Before it emits any non-bootstrap ready set, `PLAN-001` must produce and validate its actual checked-in
task manifest/schema, validator/tests and report identities: paths, blob/tree IDs, canonical SHA-256s,
schema/canonicalization versions, and validator-output hash. Early profile-independent non-behavioral
shared producers use that typed planning scope and `scenario_registry: not_applicable_pre_registry`;
they do not pretend to select a release profile. A selected-profile packet remains unready until
`GOV-005` emits its profile manifest/blob/hash, and a `genuine_red` behavior packet remains unready
until `E2E-000S` emits the populated registered scenario identity. No non-bootstrap packet may
substitute an untyped bootstrap placeholder for either artifact.

Every selected per-profile run selects exactly one canonical manifest value:

```yaml
lane: COMPAT_V1 | NATIVE_CORE
features: [] # subset of NATIVE_AGGREGATE, NATIVE_SUBSET,
             # NATIVE_TXN_ATOMIC, NATIVE_REPLICA_SINK
```

The release-profile identity is the SHA-256 of canonical JSON emitted from the selected release-profile
manifest, plus its Git blob SHA. A profile is illegal when it enables a native feature without
`NATIVE_CORE`, has an unknown feature, leaves a required edge inapplicable, or selects an app
integration different from `NATIVE-ADR-001`. The packet lists the complete evaluated predecessor
set, including generated conditional edges. Thus `E2E-003NQ` gets exactly one selected app
integration edge, and `E2E-005`/`TST-003` receive exactly the selected optional
`E2E-003T/S/A/U` runners. Compatibility evidence never proves native support, nor vice versa.

## 2. Launchable task packet

The orchestrator writes and hashes a packet before it reserves an agent. A field containing `TBD`,
a range, prose `or`, an unpinned tag such as `latest`, or a mutable input makes the packet
unlaunchable. The agent prepares its durable handoff at
`notes/execution/<task-id>/<execution-scope-id>/a<attempt>.md`; it commits that handoff only when the
packet's explicit Git authorization permits it. Packet metadata stays in the scheduler/control plane
until the task is ready to integrate.

```yaml
packet_version: 2
packet_kind: task                  # PLAN-001 alone uses bootstrap_plan in section 1
task_id: <manifest task ID>
execution_scope:
  kind: shared_producer | per_profile
  id: <shared-source-scope-sha | profile-release-profile-canonical-sha256>
  profile_scope: shared | <release-profile-canonical-sha256>
attempt: 1                         # monotonically increasing for this task/scope pair
control:
  scheduler_generation: <integer>
  integration_head: <40-hex-commit>
  reservation_lease_id: <opaque immutable lease ID>
  lease_ttl_secs: <integer, 30..300>
  heartbeat_interval_secs: <integer, at most lease_ttl_secs / 3>
  heartbeat_auth_ref: <controller-issued credential reference, never credential bytes>
  lease_expires_at: <UTC timestamp>
profile:                            # required for per_profile; null for shared_producer
  manifest_path: docs/production/release-profiles.yaml
  manifest_blob_sha: <git-blob-sha>
  canonical_sha256: <sha256>
  lane: COMPAT_V1
  features: []
base:
  integration_branch: readiness/integration
  initial_head: <40-hex-commit>     # exact clean initial base
  initial_tree_sha: <git-tree-sha>
  required_merge_base: <40-hex-commit>
  required_predecessor_commits: []
  declared_read_set: [<resolved paths/globs>]
  observed_read_set_capture: { command: <audited tracer command>, sha256: <immutable read-set report hash> }
  declared_semantic_resources: [<manifest resource IDs>]
topology:
  proof_kind: genuine_red | inherited_control | non_behavioral
  red_artifact_input: <provider-task/scenario/scope/consumer identity | not_applicable>
  red_patch_sha: <reviewed input commit-sha | not_applicable>
  red_evidence_sha256: <sha256 | not_applicable>
  candidate_must_descend_from: <red-patch-sha | required-merge-base>
  first_parent_range_start: <required-merge-base>
  immutable_test_source_sha256: <sha256 | not_applicable>
candidate_identity:
  branch: agents/<task-id>/<execution-scope-id>/a<attempt>
  worktree: /absolute/path/outside-the-shared-worktree
  prepared_patch:                    # required only for prepare_patch handoff
    format: git-diff-binary-full-index-v1
    base_commit: <base-initial-head>
    base_tree: <base-initial-tree>
    sha256: <exact patch-byte hash>
    expected_result_tree: <git-tree-sha>
    changed_files_manifest_sha256: <sha256>
    artifact_ref: <immutable artifact reference>
  images:
    engine: { oci_index_digest: <sha256 | not_applicable>, platform: { os: linux, architecture: amd64, variant: <string | null> }, platform_manifest_digest: <sha256 | not_applicable> }
    durable_streams: { oci_index_digest: <sha256 | not_applicable>, platform: { os: linux, architecture: amd64, variant: <string | null> }, platform_manifest_digest: <sha256 | not_applicable> }
    gateway: { oci_index_digest: <sha256 | not_built_yet>, platform: { os: linux, architecture: amd64, variant: <string | null> }, platform_manifest_digest: <sha256 | not_built_yet> }
    postgres:
      oci_index_digest: postgres@sha256:<index-digest>
      platform: { os: linux, architecture: amd64, variant: <string | null> }
      platform_manifest_digest: sha256:<resolved-platform-manifest-digest>
  config_sha256: <canonical-redacted-config-hash>
  toolchain_lock_sha256: <lock/toolchain-hash>
ownership:
  principal_write_boundary: <manifest-declared boundary>
  allowed_paths: [<non-overlapping paths>]
  forbidden_paths: [AGENTS.md, <other owned artifacts>]
  semantic_resources: [<public contract/schema/registry resources>]
dependencies:
  evaluated_required_tasks: []
  evaluated_conditional_edges: []
contract:
  scenario_registry_identity: <registered identity | not_applicable_pre_registry for an eligible non_behavioral shared producer>
  scenario_ids: []
  semantic_contract_sha256: []
execution:
  evidence_workspace:
    strategy: fresh_detached_worktree | verified_tree_export
    external_inputs:
      manifest_sha256: <resolved dependency/tool/fixture/base-artifact content manifest hash>
      resolver_profile: <validator-owned cargo | pnpm | swiftpm | mix | composed profile ID>
      mount_topology_sha256: <canonical logical path/mode/target mapping hash>
      access: read_only
    run_outputs:
      identity: <sha256 of packet/candidate-or-tree/gate/attempt/run-nonce>
      root: <newly empty absolute path outside every repository worktree/source>
      initial_empty_attestation_sha256: <sha256>
      cache_policy: cold_empty | read_only_content_addressed
    effective_config_sha256: <canonical effective environment/config hash>
    preflight_policy_sha256: <validator-owned clean-run policy hash>
  gate_matrix:
    path: docs/production/readiness-gates.json
    blob_sha: <git-blob-sha>
    canonical_sha256: <sha256>
  resolved_author_direct_gates: []
  resolved_merge_direct_gates: []
  owned_baseline_exception: <validator-owned task/assertion/observation hash | none>
  inherited_regression_characterization: <TST-000 observation hash | none>
  resolved_final_release_qualification: []
  fixture_namespace: <unique namespace | not_required>
  resource_reservation: <lease IDs | not_required>
git_authorization:
  mode: prepare_patch | commit_only | delegated_integration
  authority_ref: <user/delegated approval ID>
  commit_destination: <branch | null>
  push_authorization_ref: <separate approval ID | null>
deliverables:
  changed_artifacts: []
  evidence_schema_version: <version>
  handoff_note: notes/execution/<task-id>/<execution-scope-id>/a<attempt>.md
  controller_resolution: notes/execution/<task-id>/<execution-scope-id>/a<attempt>.resolution.json
review:
  contract_reviewer: <not the author>
  integration_reviewer: <required for integration task or null>
  reviewer_base_sha: <required-merge-base>
```

The scheduler canonicalizes this YAML as JSON and records its SHA-256. The packet hash appears in the
scope-qualified execution note and handoff. The image index digest is insufficient on its own: the
harness resolves the declared OCI index for the declared OS/architecture/variant before startup and
fails unless the resulting platform-manifest digest equals the packet and evidence value. A platform
matrix uses separate packet identities.

The scheduler resolves the declared read-set globs before reserve; the author-run audited read-set
capture is immutable evidence before review. A new runtime read outside that set is an unowned input
and stops the attempt. Merge-preview checks use both resolved declared paths and this observed report,
not a hand-waved assertion that two diffs look disjoint.

Every red, green, direct-gate, merge-preview, qualification, and reviewer rerun is executed from a
new evidence source, never from the mutable author or control checkout. A committed candidate uses a
new detached/linked worktree at the packet's exact commit. A `prepare_patch` candidate is applied by
exact bytes to a fresh detached base; the runner verifies its expected Git tree, exports only that
tree into a newly empty source directory, and binds the exported file/mode/content manifest to the
Git-tree ID. Immediately before each evidence command, the runner proves the source identity again.
For a committed worktree, `HEAD` and tree match the packet, the index and tracked worktree are
unchanged, `git status --porcelain=v1 --untracked-files=all` is empty, and a read-only ignored/
untracked inventory such as `git clean -ndx` is empty **before** declared read-only mounts attach. For
an exported tree, a full rescan likewise matches the bound tree manifest before mounts. After allowed
mounts attach, the overlay inventory must equal the packet's mount topology exactly and contain no
other path; the post-command check proves the same mounts and bytes remain unchanged. Dependencies,
generated tools, immutable fixture inputs
and prebuilt artifacts are exposed only through the packet's content-digested, read-only external-
input manifest and canonical mount/resolver topology. Each command gets a newly empty external run
root keyed by packet, candidate/tree, gate, attempt and run nonce; build outputs, writable fixture
state, logs, raw artifacts and cold caches live there. A cache is either cold/empty for that run or a
read-only content-addressed input in the manifest—never a mutable shared input. Immediately after the
command, the runner repeats the applicable repository/tree-mutation checks. The evidence row binds
both raw attestations, the fresh-source identity, external-input manifest and mount topology, empty
run-root attestation, exact command/candidate/tree, and canonical effective environment/config hash;
a dirty result, undeclared or writable overlay, dependency/mount/config mismatch, reused or
nonempty output root, or missing attestation is `fail`. The reviewer independently recreates this
source and resolves the same external-input manifest/topology into its own new run root instead of
trusting the author's directory or status claim.

The validator owns and mutation-tests resolver profiles for Cargo (`--locked`, external
`CARGO_HOME`/`CARGO_TARGET_DIR`), pnpm (frozen lockfile, attested external store and resolved
dependency tree), SwiftPM (pinned `Package.resolved` with external scratch/dependency state), and
Mix/Hex (locked dependencies with external home/build/deps state). If a package manager requires a
source-visible dependency mount such as `node_modules`, that is the sole permitted overlay: the
packet declares its absent-from-Git mount point, it is read-only during evidence, and every linked
file/directory byte plus logical target and mode is covered by the external-input/topology hashes.
Generated source, behavior config, test hooks, mutable tools, and writable caches are never admitted
through this exception. An unsupported resolver profile is `blocked`, not an invitation to reuse a
developer checkout.

The final candidate may have the red patch as parent and may contain multiple task-scoped commits.
Review verifies the complete first-parent range from `required_merge_base` and that the candidate
descends from `candidate_must_descend_from`; it never requires the final candidate to be a direct
child of base. A behavior-changing implementation packet has a non-optional immutable red patch.
Input change is classified by the revocation and merge-preview rules in sections 7–8; no author or
integrator rebases, squashes, amends, or otherwise rewrites reviewed commits to keep old evidence.

## 3. File and semantic ownership

One task owns one principal write boundary from the manifest. An author may modify only
`allowed_paths`, explicitly owned generated outputs, and its own execution note. It must not
opportunistically repair, reformat, or update adjacent artifacts. A reviewer treats an unowned path
as rejection, not as harmless cleanup.

The scheduler checks both collision types before reserve:

| Collision | Examples | Rule |
| --- | --- | --- |
| File | same Rust module, schema, fixture, lockfile, manifest | one reserved writer; other writers wait or use an additive owned file. |
| Semantic | same endpoint/error, template grammar, lifecycle invariant, scenario hash, PG publication schema, evidence grammar | one owner changes the contract; consumers wait for integration. |
| Runtime | same tag, port, Compose project, PG cluster/slot, DS volume, gateway namespace, app cache | unique fixture namespace and resource lease per run; candidate images are read-only. |

These are mandatory serialization points even where files differ: profile/DAG generation
(`PLAN-001`, then `GOV-005`); stable scenario registry and contract hashes (`E2E-000S`);
source journal/oracle/causal receipt semantics (`E2E-000A`); stack/resource primitives
(`E2E-000B`); reference-adapter integration (`E2E-000I`); common public protocol fields;
canonical PG18 publishable-column schema; gateway-registry authority; release image construction;
and final qualification coordinators.

If a task discovers an unlisted shared semantic resource, it stops at that discovery and reports
the resource and conflicting task. The scheduler assigns it to the declared owner or updates the DAG
through that owner. The discoverer never silently extends scope.

## 4. Worktree, branch, and commit boundaries

The shared checkout is the integration/control worktree. A dirty or untracked control checkout is
never an author worktree or a pinnable task base. Every task gets a clean linked worktree from its
pinned integration commit:

```text
readiness/integration                                     protected integration/control branch
agents/<task-id>/<execution-scope-id>/a<attempt>           author branch and one author worktree
review/<task-id>/<execution-scope-id>/a<attempt>           independent reviewer worktree at candidate SHA
```

The author branch starts with `HEAD == base.initial_head`, never merges another author branch, and
may contain only task-scoped commits. Its final handoff states author HEAD, full commit list,
changed-file list, ownership assertion, execution-note path, packet hash, raw artifact hashes, and
attempt state. The author prepares a patch and execution note by default; it commits only when the
packet's explicit `git_authorization` permits it, and never integrates its own work.

The author worktree is for editing, not evidence execution. Evidence runners create the section-2
fresh detached worktree or verified tree export for each run group and perform its clean-run
attestation before every command. A commit/tree identity without that attestation does not prove
which bytes were exercised.

The integration operator alone serializes merges: one reviewed logical task at a time onto fresh
integration HEAD. It preserves task commit SHAs in a merge or fast-forward, then performs the
regeneration sequence in section 8. “Disjoint diff” is insufficient justification for a batch merge:
only the machine-checked merge-preview refresh in section 8 may admit a candidate whose base is now
an ancestor, and manifests, toolchains, public contracts, and generated files remain semantic inputs.

## 5. Stacked genuine-red test patches

Every behavior-changing task consumes immutable stable public scenarios. Release-image scenarios assert
external behavior; same-SHA instrumented scenarios cover fsync, journal/checkpoint, catalog/rotation,
or scheduler invariants and never impersonate release-candidate evidence.

`topology.proof_kind` is mandatory and has exactly these meanings:

| Proof kind | Required observation | Legal scheduler state/use |
| --- | --- | --- |
| `genuine_red` | Immutable red patch fails at the intended semantic assertion. | May enter `red_proved` and authorize the unchanged green implementation. |
| `inherited_control` | Current behavior passes where the registered scenario explicitly allows a control. | Enter `characterized`; never label it red or consume it as a red/green pair. |
| `non_behavioral` | No public behavior contract changes. | No red patch; run its declared direct structural/control checks. |

An inherited control records its candidate, command, result, and registry reason. It is useful
characterization, but cannot waive a required red proof, be changed to accommodate implementation, or
substitute for final qualification.

A behavior stack uses two launchable packet phases; a packet never hashes an output that does not yet
exist:

```yaml
# Phase 1: may produce, but does not consume, the immutable red artifact.
packet_kind: red_artifact
provider_task: <registered test-owner task>
consumer_task: <one implementation task>
scenario_scope: { scenario_id: <id>, semantic_hash: <sha256>, execution_scope: <scope> }
base: { initial_head: <clean integration SHA>, initial_tree_sha: <tree SHA> }
output_contract: { red_patch_commit_required: true, red_evidence_required: true }

# Phase 2: emitted only after independent red review.
packet_kind: implementation
task_id: <consumer task>
red_artifact_input:
  identity: <provider-task/scenario/scope/consumer hash>
  red_patch_sha: <immutable reviewed commit>
  red_evidence_sha256: <sha256>
```

The manifest labels dependency edges `integrated` or `red_artifact`. Each red artifact is injectively
bound to provider task, consumer task, scenario/hash, execution scope/profile and base. `red_proved`
unlocks only that consumer; the green candidate must descend from the exact red commit. The controller
resolves the artifact when the reviewed green stack integrates. A test-owner task that serves several
implementations emits separate artifacts so one consumer never inherits unrelated failing tests.

1. The contract owner selects declared scenario IDs and canonicalizes source journal, independent
   oracle, external action, expected result, profile expression, cut tier, exclusions, and evidence
   schema. The registry records their SHA-256.
2. The test owner creates a **red patch** on the packet base. It adds the contract test and no
   behavior that makes it pass.
3. The recorded command must fail for the intended semantic assertion. Compile/setup failure,
   missing fixture, timeout, skip, `xfail`, filtered test, or broad expected-error handler is not
   red evidence.
4. Raw red output and a replay command are hashed. The implementation packet names the exact red
   patch commit and red-evidence hash.
5. The implementation owner begins from that exact patch, changes only its implementation boundary,
   and turns the unchanged scenario green.
6. Integration accepts the pair only when scenario IDs, semantic hash, test source hash, profile
   hash, exclusions hash, and oracle are identical. A contract change starts a new red/green pair.

Even when one authorized author performs both phases, the scheduler issues two packets with an
independent red review and frozen artifact between them; there is no combined packet with a future
red SHA. The green candidate is required to descend from the red patch, not directly from base. For
split work, the test owner never writes production implementation and the implementation owner never
weakens the test/oracle/exclusions. A qualification runner may add adapters and evidence capture, but
cannot alter a scenario's journal, oracle, expected outcome, exclusion, or hash.

The registry must bind at least `scenario_id`, semantic contract hash, test-owner task,
implementation-owner task(s), integration runner, applicability expression, public oracle, source
journal hash, cut tier, and evidence-schema version. It rejects duplicate IDs or semantic ownership,
unregistered hash changes, and a runner result missing a selected scenario.

## 6. Candidate and qualification identity

An execution claim is a tuple, not prose:

```text
(task ID, execution scope/profile hash, attempt, author candidate SHA, source tree SHA, base SHA,
 profile manifest blob/hash or shared-producer scope, scenario-registry identity,
 scenario IDs + semantic hashes, proof kind and red patch SHA,
 engine/DS/gateway/PG OCI index digest + OS/architecture/variant + resolved platform-manifest digest,
 config/effective-environment hash, toolchain hash, external-input/mount-topology hashes,
 clean-source and empty-run-root attestation hashes,
 fixture namespace, seed/workload manifest hash, commands, raw-result hashes)
```

Images use content-addressed `name@sha256:...` references. An OCI index alone is not evidence: raw
artifacts retain the declared platform and platform-manifest digest resolved from that index. A mutable
tag, local image ID without a digest, platform mismatch, or rebuilt image is not evidence. Candidate producers emit immutable bytes before
qualification; `RLS-001` later signs and attests precisely those bytes and cannot rebuild them.
Configuration is canonicalized after redacting secrets, while retaining names/presence/version
information needed to distinguish behavior. It includes profile, templates, publication/admission
manifests, images, test mode, and behavior-affecting values. Raw artifacts must exclude credentials,
signed URLs, raw predicates, and production data.

A candidate release-image execution has instrumented hooks disabled. Its focused invariant execution
uses an instrumented binary from the same source SHA and records a distinct identity. Each wait has a
named barrier plus diagnostic deadline; sleeps do not create ordering. For E2E, the causal fence is
the source transaction receipt, server drained-through receipt, then actual target fold/cache commit
keyed by principal/template/generation. SQL equality, a tail offset, or a separate sentinel feed is
not a target-client receipt.

Each workload manifest fixes seed, operation distribution, offered/attempt budgets, operation-class
minimum admitted/committed/applied counts, template event floors, cut IDs/runs, per-operation
diagnostic deadline, global deadline, and deterministic terminal condition. A skipped, filtered,
zero-test, under-run, stale/missing identity, wrong profile/digest/config/hash, or unexpected
divergence is `fail`, never a soft warning. An unavailable external system is explicit
non-promotable `blocked`. The signed pre-run divergence allowlist is part of the tuple; changing it
begins a new stage. P999 needs 100,000 observations in its operation class.

For app evidence, record the real app commit, vendored `ElectricSync` subtree hash, and proven
upstream-base/patch provenance. A sibling package is separate conformance unless the exact
relationship is proved.

## 7. Agent assignment envelope and gates

Enqueue an agent only with a complete packet and this instruction envelope:

```text
You own <TASK-ID>, execution scope <SCOPE>, attempt <N>. Read AGENTS.md and this packet before acting.
Use only <WORKTREE> on branch <BRANCH>, beginning at <BASE-SHA>.
Allowed paths: <LIST>. All other paths are forbidden, including AGENTS.md.
Profile/hash or shared-producer scope: <IDENTITY>. Scenarios/hashes: <LIST>.
Proof kind: <KIND>; consume the immutable red patch <SHA> when genuine_red.
Lease/generation: <LEASE>/<GENERATION>; check it at every phase boundary.
Run author direct gates. Never claim pass for skipped, filtered, missing, stale,
wrong-digest/config/profile, or under-run evidence.
Do not change task graph/registry semantics unless this packet owns them.
Prepare notes/execution/<TASK-ID>/<SCOPE>/a<N>.md with packet hash, inputs, commands/results,
raw hashes, paths, risks, and state. Commit or push only under the packet's explicit Git
authorization; do not integrate. Stop on collision, lease revocation, changed prerequisite, hash
mismatch, failed red proof, or external blocker.
```

`PLAN-001` generates `docs/production/readiness-gates.json`. Every task/gate row pins a gate ID,
phase (`author_direct`, `merge_direct`, or `final_release_qualification`), applicability expression,
command/config identity, owning task, and any permitted baseline assertion. Packets reference the
matrix blob/canonical hash and resolve it for their scope; they may add stricter gates but cannot omit,
move, or weaken a generated row. Validation rejects an inherited result without the exact `TST-000`
observation hash and a baseline exception outside its generated owner/assertion.

A named baseline-repair exception requires its exact base failure/assertion green plus every runnable
direct gate; it cannot conceal another failure, skip a suite, or waive qualification. For ordinary
engine implementation, local format/Rust/TypeScript/conformance lanes are author/merge gates, while
the external Electric and browser lanes are final qualification unless the generated matrix says the
packet owns or modifies that lane. An unavailable final lane blocks promotion but does not deadlock
the task that installs or repairs it: its direct/merge evidence can pass while qualification remains
explicitly `blocked`. No packet author may choose this classification.

Git authority is explicit and narrower than task assignment. `prepare_patch` is the default: the
author prepares a candidate patch and scope-qualified note but does not commit or push.
`commit_only` requires an explicit user/delegated authority reference and permits only a task-scoped
local commit to the stated destination. `delegated_integration` identifies the separately authorized
operator; it does not by itself give the author commit authority. Push always requires the separate
`push_authorization_ref`, and integration requires separate operator authority. An internal packet,
branch name, or review approval is never a substitute for user/delegated Git authorization.

A `genuine_red` implementation cannot launch until its immutable red-patch SHA was created under a
`red_artifact` packet by an authorized contract owner/integrator. If the same author performs both
phases, each packet needs the authority appropriate to its own commit and the red review must finish
before the implementation packet exists. This is a provenance requirement, not implied permission to
push or integrate. A `red_artifact` packet therefore requires `commit_only`; `prepare_patch` cannot
produce the checkout identity the independent red reviewer must execute.

A prepared patch is immutable review input, not mutable worktree state. Produce exact binary/full-index
Git-diff bytes against the packet's base, compute its SHA-256, expected result tree and changed-file
manifest hash, store the bytes at the packet's immutable artifact reference, and bind all of those plus
raw evidence hashes into the handoff. The reviewer records that exact tuple. The authorized integrator
verifies the bytes/hash/base/tree, clean indexed apply and expected result tree before committing; any
byte, base, apply, path-manifest or resulting-tree mismatch invalidates the attempt and requires fresh
review.

Each gate/evidence row also records the section-2 pre/post clean-run attestation hashes and effective
environment/config hash, external-input manifest/mount-topology hashes, and unique initially empty
run-root identity/attestation. The validator rejects evidence produced from the author/control
checkout, from a reused source or run root, with an undeclared or writable source-visible overlay,
with a mutable or unresolved external input, or without an exact commit/tree/export/dependency/config match.
Test-generated source mutations fail the row even when the test assertion itself passed.

Every attempt note uses the same identity and completion vocabulary:

```yaml
attempt_key: [<task-id>, <execution-scope-id>, <attempt>]
packet_sha256: <sha256>
lease: { id: <id>, admitted_generation: <integer>, status: active | renewed | revoked }
topology: { initial_head: <sha>, red_patch: <sha | null>, candidate_commit: <sha | null>, prepared_patch_sha256: <sha | null> }
proof: { kind: genuine_red | inherited_control | non_behavioral, state: red_proved | characterized | not_applicable }
identities: { profile_or_scope: <id>, registry: <id>, config: <sha>, toolchain: <sha>, images: <OCI/platform tuple> }
gates: { author_direct: <results>, merge_direct: <results>, qualification: <results | blocked> }
handoff_state: ready_for_review | fail | blocked
handoff_reason: <required unless ready_for_review>
```

The author note is immutable handoff evidence and never contains terminal `pass`. The controller
writes the distinct, content-addressed `.resolution.json` only after integration acceptance,
rejection/blocking, or invalidation; it binds the handoff hash, controller-event-log hash, integration
SHA/generation, gate report and terminal `pass | fail | blocked | invalidated`. If stored in Git, it is
a separate operator-owned authorized commit and never amends the reviewed task candidate. A local
green run is never `pass`, and `blocked` never converts missing final qualification into completion.

An engine-touching packet prints its entire resolved gate matrix, then runs every generated direct
gate and records command, exit status, duration, candidate SHA and raw log. It also records every
qualification lane, including an unavailable Electric/browser lane as `blocked`; absence from direct
gates is never absence from the release closure. Other packets run all generated gates associated
with the artifacts they modify. A missing tool is not a waiver, but neither may an unavailable final
lane be relabelled as a direct gate to deadlock the task that makes it runnable.

## 8. Independent review, serialized integration, and regeneration

Reservations are control-plane leases, not advisory labels. Reserve atomically compares and swaps
`(task_id, execution_scope_id, attempt, scheduler_generation, integration_head, lease_id, available)`
to one live lease. Review admission, refreshed-review admission, and integration admission repeat the
same comparison. An agent checks its lease before each phase boundary and before publishing evidence;
a reviewer or integrator rejects an expired, revoked, mismatched, or non-current lease even when its
tests are green.

On every accepted integration, the scheduler increments its generation and atomically classifies live
leases. A changed declared predecessor, allowed path/read set, semantic resource, scenario/contract/
oracle/exclusion, profile closure, config, image index/platform-manifest, toolchain, artifact identity,
or ownership decision revokes the affected lease. The controller notifies/cancels the agent and writes
`notes/execution/<task-id>/<execution-scope-id>/a<attempt>.resolution.json` with terminal
`invalidated`, the immutable handoff hash when one exists, new generation, intervening commit range,
reason, and superseding packet when present. It cannot reach review or integration after revocation.

An unaffected live lease is not silently left on the old generation. The controller machine-checks
the intervening diff against its declared paths/read set and semantic inputs, then atomically renews
the lease onto the new generation with a refresh record; otherwise it revokes it. The packet's initial
base remains unchanged, the agent observes the renewed lease before continuing, and the eventual
candidate still requires the merge-preview procedure below. If the later observed read-set capture
intersects the intervening range, that discovery hard-invalidates the renewed attempt.

Lease liveness is explicit and bounded; it is not qualification monitoring. The agent sends an
authenticated heartbeat/renew request at or before the packet interval with packet hash, lease ID,
current generation, monotonic nonce and phase. The controller rechecks generation, relevant inputs,
ownership/resource reservations and revocation state before issuing a content-hashed renewal whose
new expiry is at most 300 seconds away. The agent must observe that acknowledgement before its next
phase boundary or evidence publication. The controller cannot renew silently on an agent's behalf.
Control-plane loss, missed heartbeat or absent acknowledgement expires the lease and stops the
attempt. Every request/ack/revocation is appended to the immutable controller event log; the handoff
binds its final log hash.

The contract reviewer is never the author and works from an independent reviewer worktree. Before
approval, the reviewer verifies:

- a freshly created detached worktree at the reviewed commit (or verified tree export for a prepared
  patch), pre/post clean-run attestations, no extra/ignored/untracked overlay, the identical content-
  digested read-only external-input manifest and resolver/mount topology, its own newly empty external
  output/cache root, and the packet's exact effective environment/config hash for every rerun;
- base/first-parent lineage, scope/profile and registry identities, required predecessors, source
  tree, OCI index/platform-manifest, config, and toolchain identities;
- exact allowed paths and no unowned semantic changes;
- the required proof kind: genuine red patch and unchanged green contract, inherited-control
  characterization, or non-behavioral justification;
- commands and raw evidence with no skip/filter/xfail/zero/under-run laundering;
- acceptance criteria, AGENTS invariants, evidence schema, execution note, and remaining risk; and
- no credentials or private production data in commits/artifacts.

Each integration task also needs a cross-boundary reviewer: `E2E-000I`, `PG18-003A`,
`PG18-003Q`, `E2E-001Q`, `E2E-002Q`, `E2E-003CQ`, `E2E-003NQ`, `E2E-004Q`,
`E2E-005`, `CAP-003Q`, `OPS-001B`, `OPS-002`, `OPS-004`, `OPS-009`, `TST-003`,
and `MIG-004`–`MIG-009`. That reviewer checks cross-runtime assumptions, candidate versus
instrumented tier, and exact client/gateway/engine/PG alignment. Qualification reviewers never patch
the behavior they judge.

If integration HEAD still equals the packet base, the operator makes a proposed merge, runs merge
direct gates on that result, obtains independent review of that evidence, and atomically admits it
under the current lease/generation. If HEAD has advanced, a lease is eligible only for this
**merge-preview integration refresh**:

This refresh is available only to immutable committed task SHAs. A `prepare_patch` handoff has no
commit identity to preserve; if HEAD advances before its authorized integrator applies it, revoke the
attempt and issue a fresh packet/review on the new base rather than pretending its old evidence
survives.

1. Machine-check the intervening integration diff from packet base to current HEAD against the
   packet's changed paths, declared and observed read set, semantic-resource IDs, predecessors,
   scenario/registry/contract/oracle/exclusion hashes, profile closure, config, OCI index/platform
   manifest, toolchain, and artifact identities. Any intersection, changed declared input, or merge
   conflict is hard invalidation.
2. In a fresh preview worktree, merge the immutable reviewed candidate commit(s) into current HEAD
   without rebasing, cherry-picking, squashing, amending, or rewriting them. The reviewed task commit
   SHAs are preserved exactly.
3. Run merge direct gates and every machine-detected affected gate on the preview; record fresh raw
   logs and the preview SHA. Existing qualification evidence is not reused: it is provenance only,
   and final qualification must execute again against its exact candidate identity.
4. An independent reviewer checks the intersection report, preserved commit SHAs, refreshed evidence,
   preview identity, and unchanged public contract/result. Only then atomically renew the lease to the
   current generation and admit that exact preview as integration HEAD.

After every admitted merge, the integration operator must verify the integration SHA and preserved task
commits, run direct/affected gates, run the `PLAN-001` validator and any applicable selected-profile
compiler, and store their report hashes. It re-evaluates all remaining tasks and emits only legal ready
work with integrated evaluated predecessors, no ownership collision, an available lease, and a fresh
current-generation packet. A changed relevant input produces `invalidated`, not a silent rebase or
reuse of qualification evidence.

## 9. Exact stopping conditions

An active attempt stops immediately when:

- its reservation lease is revoked, expired, or no longer in the current scheduler generation;
- scope/profile or registry identity, base SHA/tree, predecessor, scenario/oracle/exclusion, config,
  OCI index/platform-manifest, artifact identity, or toolchain differs from its packet;
- it needs an unowned path/semantic resource or detects an active reservation for one;
- its required red test cannot fail specifically for the intended behavior;
- a required gate fails, skips/filters/executes zero, under-runs, misses a named deadline, or finds
  unexpected divergence;
- an external dependency or unique fixture/resource lease is unavailable, or content-addressed
  artifact identity cannot be produced; or
- review finds a contract, invariant, ownership, evidence, or security discrepancy.

The immutable scope-qualified handoff note records lease/generation, command, namespace, logs, input
identities, first divergence or missing dependency, and `ready_for_review | fail | blocked`. The
controller's separate resolution has terminal `pass | fail | blocked | invalidated`; only it writes
`pass` after authorized integration acceptance, and `invalidated` names the superseding generation/
packet. The scheduler then does exactly
one of: issue a corrected packet from current integration HEAD; assign the discovered boundary to its
declared owner; record an external non-promotable `blocked`; reject the candidate; or atomically mark
the attempt `invalidated`. There is no unspecified monitoring period. No agent may wait indefinitely,
broaden a profile, alter an allowlist/scenario, relax a gate, or keep working on stale inputs to obtain
a green result.

## 10. Completion

A task is integration-admissible only after independent review accepts packet-conforming commits or a
content-addressed prepared patch, the required proof-kind evidence (genuine red-to-green only for
`genuine_red`), exact direct/affected gate evidence at the proposed integration result, and the
immutable scope-qualified handoff note. It becomes integrated `pass` only when the controller records
the separate terminal resolution after authorized admission. Integration is not qualification.
Qualification additionally requires immutable candidate
artifacts, unchanged scenario hashes, exact selected-profile closure, finite workload completion, and
independent runner/reviewer evidence. `TST-003` determines G0–G9 for the profile; G10a, G10b,
G10c, and each hash-bound G10d stage are separate authorization decisions. G10e is later
decommission, not a substitute for an earlier gate.

The resulting concurrency model is narrow by design: agents develop independent packet-scoped
candidates in isolated worktrees, reviewers judge immutable evidence, and the scheduler regenerates
the legal ready set after every serialized integration point.
