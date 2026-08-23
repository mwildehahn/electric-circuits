import assert from 'node:assert/strict'
import test from 'node:test'
import { buildPlanningPacket, evaluate, manifest, outputIdentities, ready, scheduledReady, selectedDependencies, validateEvidence, validateLease, validateManifest, validateOutcome, validatePacket, validateRedArtifactAdmission } from './readiness-plan.js'

// Acceptance corpus: 100 deterministic randomized ready-task scheduler runs cover COMPAT_V1 and
// NATIVE_CORE with NATIVE_AGGREGATE, NATIVE_SUBSET, NATIVE_TXN_ATOMIC, and NATIVE_REPLICA_SINK.
// It mutates heartbeat/control-plane/generation/silent renewal plus author_checkout, source_reused,
// initially-empty run root, post source mutation, and effective config mismatch provenance.

const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value)) as T
const throws = (reason: string, operation: () => unknown) => assert.throws(operation, new RegExp(reason))
const digest = 'a'.repeat(40)
const evidence = () => ({ source_strategy: 'fresh_detached_worktree', source_commit: digest, source_tree: digest, pre_attestation_sha256: digest, post_attestation_sha256: digest, external_input_manifest_sha256: digest, mount_topology_sha256: digest, run_root_identity: 'run-a', empty_run_root_attestation_sha256: digest, effective_config_sha256: digest, source_clean: true, mount_read_only: true, run_root_new_empty: true, post_source_unchanged: true, external_inputs_unchanged: true, config_matches: true })
const lease = () => ({ heartbeat_auth_ref: 'controller-root', reservation_lease_id: 'lease-a', scheduler_generation: 1, authenticated: true, generation: 1, current_generation: 1, packet_sha256: digest, heartbeat_nonce: 1, acknowledged_nonce: 1, ttl_ms: 30_000, heartbeat_interval_ms: 10_000, expires_at: Date.now() + 60_000, last_heartbeat_at: Date.now(), heartbeat_deadline_ms: 10_000, control_plane_available: true })
const controllerState = (control: ReturnType<typeof lease>, resolutions = [{ task_id: 'PLAN-001', scope_id: 'bootstrap-plan-001', outcome: 'pass', state: 'integrated', generation: 1, base: { head: '3f452e4dba00047b591e07617583aa4ec2387c2c', tree: '4621d91dad930c03b33e4775cc984d84d77c3f2d' } }]) => ({ generation: 1, integration_head: '3f452e4dba00047b591e07617583aa4ec2387c2c', integration_tree: '4621d91dad930c03b33e4775cc984d84d77c3f2d', resolutions, lease: control })
const bootstrapFixture = () => ({
  packet_version: 2, packet_kind: 'bootstrap_plan', task_id: 'PLAN-001', attempt: 9,
  execution_scope: { kind: 'bootstrap', id: 'bootstrap-plan-001', profile_scope: 'uncompiled_all' }, profile: null, release_profile_hash: null,
  authority: { integration_commit: '520751ef250abd4936720e1e7e0c620a158833a0', integration_tree: '8374caa150215bb0ad74c535769fc978e0e57af7', canonical_inputs: [{ path: 'notes/18-production-readiness-spec-reviewed.md', blob_sha: 'cb573d73e8e79b11dba829dc9d7ac2ec21533918' }, { path: 'notes/skills-research/05-parallel-agent-execution-protocol.md', blob_sha: '29d84268de4a6cf5e12868ba5738a85c624f095c' }], forbidden_inputs: ['notes/16-production-readiness-spec.md','notes/reviews/**','notes/23-swift-app-e2e-tdd-map.md','notes/24-postgres18-and-e2e-tdd-addendum.md','notes/25-pg18-e2e-differential-review-disposition.md','all PLAN-001 attempt artifacts a1 through a9','all controller, author, reviewer, cache, generated-output, and mutable checkout evidence'] },
  base: { initial_head: '520751ef250abd4936720e1e7e0c620a158833a0', initial_tree: '8374caa150215bb0ad74c535769fc978e0e57af7' },
  declared_outputs: ['docs/production/readiness-tasks.json','docs/production/readiness-task.schema.json','docs/production/readiness-gates.json','docs/production/readiness-plan.generated.md','scripts/readiness-plan.ts','scripts/readiness-plan.test.ts'].map(path => ({ path, owner_task: 'PLAN-001', identity: null })),
  future_inputs: { release_profiles: { state: 'unavailable', owner: 'GOV-005' }, scenario_registry: { state: 'unavailable', owner: 'E2E-000S' } },
  evidence_contract: { schema_version: 'evidence-v2', source_strategy: 'fresh_detached_worktree|verified_tree_export' }, review: { contract_reviewer: 'independent-contract-reviewer', integration_reviewer: 'independent-integration-reviewer' },
  control: { scheduler_generation: 8, reservation_lease_id: '4B75969E-E803-4AD1-9F24-BB731C9B163E', lease_ttl_secs: 300, heartbeat_interval_secs: 45, heartbeat_deadline_secs: 90, heartbeat_auth_ref: 'controller/root/plan-001/a9', lease_issued_at: 'now', lease_expires_at: 'later', request_mailbox: 'request', ack_mailbox: 'ack', phase_file: 'phase', stop_file: 'stop' },
  workspace: { worktree: 'worktree', branch: 'branch', run_root: 'root', dependency_snapshot: 'deps', dependency_content_sha256: digest, dependency_manifest_sha256: digest, resolver_profile: 'resolver', resolver_config_sha256: digest, mount_topology_sha256: digest, mount_attestation_sha256: digest },
  ownership: { allowed_outputs: ['docs/production/readiness-tasks.json','docs/production/readiness-task.schema.json','docs/production/readiness-gates.json','docs/production/readiness-plan.generated.md','scripts/readiness-plan.ts','scripts/readiness-plan.test.ts','notes/execution/PLAN-001/bootstrap-plan-001/a10.md'], forbidden: ['tracked','overlay','push'] },
  contract: { authorities_only: true, exact_task_inventory: 169, normative_conditional_groups: 36, normative_conditional_requirements: 56, required_models: ['model'], required_rejections: ['rejection'], test_quality: ['quality'] },
  direct_gates: ['node --import tsx --test scripts/readiness-plan.test.ts','node --import tsx --test /tmp/electric-circuits-plan-001-a4-regression.test.ts','pnpm exec tsc --ignoreConfig --noEmit --strict --target ES2022 --module NodeNext --moduleResolution NodeNext --types node scripts/readiness-plan.ts scripts/readiness-plan.test.ts','pnpm exec tsx scripts/readiness-plan.ts validate','pnpm exec tsx scripts/readiness-plan.ts ready --scope shared-pre-registry --completed PLAN-001','pnpm exec tsx scripts/readiness-plan.ts ready --scope shared-pre-registry --completed PLAN-001,GOV-001','pnpm typecheck'],
  git_authority: { mode: 'commit_only', authority_ref: 'user-2026-08-23-kick-off-first-set', push: false, integration: false }, handoff: { independent_review_required: true, controller_resolution_required: true, message_before_evidence: true, message_before_commit: true }
})

test('the authority inventory is complete, acyclic, and has exact bootstrap waves', () => {
  const plan = manifest(); validateManifest(plan)
  assert.equal(plan.tasks.length, 169); assert.equal(plan.conditional_edges.length, 36)
  assert.deepEqual(ready(plan, null, new Set(['PLAN-001']), 'shared-pre-registry'), ['GOV-001', 'TST-000'])
  assert.deepEqual(ready(plan, null, new Set(['PLAN-001', 'GOV-001']), 'shared-pre-registry'), ['CMP-000', 'GOV-002', 'GOV-003', 'SEC-008B', 'TST-000'])
})

test('all native profile subsets and compatibility evaluate machine conditions', () => {
  const features = ['NATIVE_AGGREGATE', 'NATIVE_SUBSET', 'NATIVE_TXN_ATOMIC', 'NATIVE_REPLICA_SINK']
  for (let bits = 0; bits < 16; bits++) { const profile = { lane: 'NATIVE_CORE' as const, features: features.filter((_, index) => bits & (1 << index)) }; assert.equal(evaluate('N', profile), true); assert.equal(evaluate('C', profile), false) }
  assert.equal(evaluate('C&&!N', { lane: 'COMPAT_V1', features: [] }), true)
  throws('invalid_profile', () => evaluate('true', { lane: 'COMPAT_V1', features: ['NATIVE_SUBSET'] }))
  throws('invalid_profile_expression', () => evaluate('lane == NATIVE_CORE', { lane: 'NATIVE_CORE', features: [] }))
})

test('100 seeded simulations produce complete terminal profile closures without reverse scheduling', () => {
  const plan = manifest(); const features = ['NATIVE_AGGREGATE','NATIVE_SUBSET','NATIVE_TXN_ATOMIC','NATIVE_REPLICA_SINK']; let seed = 0x12345678
  const profiles = [{ lane: 'COMPAT_V1' as const, features: [] }, ...Array.from({ length: 16 }, (_, bits) => ({ lane: 'NATIVE_CORE' as const, features: features.filter((_, index) => bits & (1 << index)) }))]
  const red = new Set(plan.tasks.flatMap(task => task.dependencies.red_artifacts.map(edge => `${edge.provider}:${edge.consumer}`)))
  for (let run = 0; run < 100; run++) { const profile = profiles[run % profiles.length]; const outcomes = new Map<string, 'pass'>([['PLAN-001','pass']]); for (let step = 0; step < plan.tasks.length + 1; step++) { const options = scheduledReady(plan, profile, outcomes, red); if (!options.length) break; for (const id of options) assert.ok(selectedDependencies(plan, plan.tasks.find(task => task.id === id)!, profile).every(dependency => outcomes.get(dependency) === 'pass')); seed = (seed * 1664525 + 1013904223) >>> 0; outcomes.set(options[seed % options.length], 'pass') } const applicable = plan.tasks.filter(task => evaluate(task.applicability, profile)).map(task => task.id).sort(); assert.deepEqual([...outcomes.keys()].sort(), applicable) }
})

test('complete deterministic closures cover COMPAT and all sixteen native feature combinations', () => {
  const plan = manifest(); const features = ['NATIVE_AGGREGATE','NATIVE_SUBSET','NATIVE_TXN_ATOMIC','NATIVE_REPLICA_SINK']
  const profiles = [{ lane: 'COMPAT_V1' as const, features: [] }, ...Array.from({ length: 16 }, (_, bits) => ({ lane: 'NATIVE_CORE' as const, features: features.filter((_, index) => bits & (1 << index)) }))]
  for (const profile of profiles) { const outcomes = new Map<string, 'pass'>([['PLAN-001','pass']]); const red = new Set(plan.tasks.flatMap(task => task.dependencies.red_artifacts.map(edge => `${edge.provider}:${edge.consumer}`))); for (let step = 0; step < 200; step++) { const next = scheduledReady(plan, profile, outcomes, red); if (!next.length) break; outcomes.set(next[step % next.length], 'pass') } const applicable = plan.tasks.filter(task => evaluate(task.applicability, profile)).map(task => task.id); assert.deepEqual([...outcomes.keys()].filter(id => id !== 'PLAN-001').sort(), applicable.filter(id => id !== 'PLAN-001').sort()); }
  throws('required_not_applicable', () => validateOutcome(plan.tasks[0], { lane: 'COMPAT_V1', features: [] }, 'not_applicable_by_profile'))
})

test('failed, blocked, N/A, reverse-wave, and unreviewed red predecessors do not unlock work', () => {
  const plan = manifest(); const compat = { lane: 'COMPAT_V1' as const, features: [] }
  const failed = new Map<string, 'pass' | 'fail'>([['PLAN-001','pass'], ['GOV-001','fail']])
  assert.ok(!scheduledReady(plan, compat, failed, new Set()).includes('GOV-002'))
  const blocked = new Map<string, 'pass' | 'blocked'>([['PLAN-001','pass'], ['GOV-001','blocked']])
  assert.ok(!scheduledReady(plan, compat, blocked, new Set()).includes('GOV-002'))
  throws('required_not_applicable', () => validateOutcome(plan.tasks.find(task => task.id === 'GOV-001')!, compat, 'not_applicable_by_profile'))
  assert.ok(!scheduledReady(plan, compat, new Map(), new Set()).includes('GOV-001'))
  const allPassed = new Map(plan.tasks.map(task => [task.id, 'pass' as const])); allPassed.delete('SEC-002A')
  assert.ok(!scheduledReady(plan, compat, allPassed, new Set()).includes('SEC-002A'))
  assert.ok(scheduledReady(plan, compat, allPassed, new Set(['E2E-002R:SEC-002A'])).includes('SEC-002A'))
})

test('graph and ownership mutations have named failures', () => {
  const duplicate = clone(manifest()); duplicate.tasks[1].id = duplicate.tasks[0].id; throws('duplicate_or_invalid_task', () => validateManifest(duplicate))
  const unknown = clone(manifest()); unknown.tasks[1].dependencies.integrated.push('NOPE-001'); throws('unknown_dependency', () => validateManifest(unknown))
  const cycle = clone(manifest()); cycle.tasks[0].dependencies.integrated.push('GOV-001'); throws('cyclic_dependency', () => validateManifest(cycle))
  const conditional = clone(manifest()); conditional.conditional_edges[0].requires = ['TST-008N']; throws('noncanonical_conditional_inventory', () => validateManifest(conditional))
  const missingBoundary = clone(manifest()); missingBoundary.tasks[0].principal_write_boundary = ''; throws('invalid_principal_boundary', () => validateManifest(missingBoundary))
  const placeholder = clone(manifest()); placeholder.tasks[0].artifacts.push('TBD'); throws('mutable_placeholder', () => validateManifest(placeholder))
  const owner = clone(manifest()); owner.tasks[0].owner = 'unowned'; throws('noncanonical_task_inventory', () => validateManifest(owner))
  const semantic = clone(manifest()); semantic.tasks[1].semantic_resources[0] = semantic.tasks[0].semantic_resources[0]; semantic.tasks[1].resources.semantic[0] = semantic.tasks[0].resources.semantic[0]; throws('duplicate_semantic_resource_owner', () => validateManifest(semantic))
  const runtime = clone(manifest()); runtime.tasks[1].runtime_resources[0] = runtime.tasks[0].runtime_resources[0]; runtime.tasks[1].resources.runtime[0] = runtime.tasks[0].resources.runtime[0]; throws('duplicate_runtime_resource_owner', () => validateManifest(runtime))
})

test('bootstrap, registry, red-artifact, evidence, and lease control reject invalid inputs', () => {
  const plan = manifest()
  const bootstrap = bootstrapFixture(); throws('invalid_bootstrap_packet', () => validatePacket(bootstrap, plan))
  const alteredBootstrap = clone(bootstrap); alteredBootstrap.authority.integration_tree = digest; throws('invalid_bootstrap_packet', () => validatePacket(alteredBootstrap, plan))
  const malformed: any = clone(bootstrap); malformed.profile = {}; throws('invalid_bootstrap_packet', () => validatePacket(malformed, plan))
  const prototype: any = clone(bootstrap); delete prototype.declared_outputs; throws('invalid_bootstrap_packet', () => validatePacket(prototype, plan))
  const behaviorLease = lease(); const behavior: any = clone(buildPlanningPacket(plan, 'GOV-001', controllerState(behaviorLease), behaviorLease)); behavior.task_id = 'ENG-006'; behavior.execution_scope = { kind: plan.tasks.find(task => task.id === 'ENG-006')!.execution_scope, id: 'profile-test', profile_scope: 'shared' }; behavior.topology = { proof_kind: 'genuine_red', red_artifact_input: null }; behavior.contract = { scenario_registry_identity: null, scenario_registry: 'registered' }
  throws('scenario_registry_unavailable', () => validatePacket(behavior, plan))
  throws('author_control_evidence', () => validateEvidence({ ...evidence(), source_strategy: 'author_checkout' }))
  throws('writable_overlay', () => validateEvidence({ ...evidence(), overlay_writable: true }))
  throws('stale_generation', () => validateLease({ ...lease(), current_generation: 2 }, 1, 0))
  throws('silent_renewal', () => validateLease({ ...lease(), heartbeat_nonce: 2, acknowledged_nonce: 1 }, 1, 1))
  throws('missed_heartbeat', () => validateLease({ ...lease(), expires_at: 10, heartbeat_deadline_ms: 1 }, Date.now(), 1))
  throws('control_plane_loss', () => validateLease({ ...lease(), control_plane_available: false }, 1, 1))
  throws('invalid_lease_ttl', () => validateLease({ ...lease(), ttl_ms: 20_000 }, 1, 1))
})

test('planning-scope packets bind all six identities and defer profiles and genuine red work', () => {
  const plan = manifest(); const control = lease(); const state = controllerState(control); const gov = buildPlanningPacket(plan, 'GOV-001', state, control); validatePacket(gov, plan)
  assert.equal(buildPlanningPacket(plan, 'TST-000', state, control).topology.proof_kind, 'inherited_control')
  assert.equal(outputIdentities().outputs.length, 6)
  const stale = clone(gov); stale.output_identities[0].sha256 = digest; throws('packet_lease_binding', () => validatePacket(stale, plan))
  const staleBase = clone(gov); staleBase.base.initial_head = digest; throws('packet_lease_binding', () => validatePacket(staleBase, plan))
  const forgedScope = clone(gov); forgedScope.predecessors = []; throws('packet_lease_binding', () => validatePacket(forgedScope, plan))
  const expired: any = clone(gov); expired.control.expires_at = Date.now() - 1; throws('missed_heartbeat', () => validatePacket(expired, plan))
  throws('controller_lease_required', () => buildPlanningPacket(plan, 'GOV-001', state))
  const failedState: any = controllerState(control, [{ ...state.resolutions[0], outcome: 'fail' }]); throws('premature_task_packet', () => buildPlanningPacket(plan, 'GOV-001', failedState, control))
  const wrongScopeState: any = controllerState(control, [{ ...state.resolutions[0], scope_id: 'forged' }]); throws('premature_task_packet', () => buildPlanningPacket(plan, 'GOV-001', wrongScopeState, control))
  throws('premature_task_packet', () => buildPlanningPacket(plan, 'GOV-002', state, control))
  throws('release_profile_unavailable', () => buildPlanningPacket(plan, 'NATIVE-ADR-001', state, control))
  throws('scenario_registry_unavailable', () => buildPlanningPacket(plan, 'ENG-006', state, control))
})

test('post-bootstrap planning packets bind the controller integration head/tree', () => {
  const control = lease(); const head = '3f452e4dba00047b591e07617583aa4ec2387c2c'; const tree = '4621d91dad930c03b33e4775cc984d84d77c3f2d'
  const state = controllerState(control, [{ task_id: 'PLAN-001', scope_id: 'bootstrap-plan-001', outcome: 'pass', state: 'integrated', generation: 1, base: { head, tree } }])
  state.integration_head = head; state.integration_tree = tree
  const packet: any = buildPlanningPacket(manifest(), 'GOV-001', state, control)
  assert.equal(packet.base.initial_head, head); assert.equal(packet.base.integration_tree, tree); assert.equal(packet.authority.integration_commit, head)
  assert.doesNotThrow(() => validatePacket(packet, manifest()))
  const forged = clone(packet); forged.base.initial_head = '3'.repeat(40); forged.authority.integration_commit = '3'.repeat(40)
  assert.throws(() => validatePacket(forged, manifest()), /stale_dispatch_base|packet_lease_binding/)
  const forgedState: any = clone(state); forgedState.integration_head = '3'.repeat(40); forgedState.integration_tree = '4'.repeat(40)
  assert.throws(() => buildPlanningPacket(manifest(), 'GOV-001', forgedState, control), /stale_controller_state/)
  const full: any = { ...packet, candidate_identity: {}, execution: {}, ownership: {}, deliverables: {} }
  assert.doesNotThrow(() => validatePacket(full, manifest()))
  full.base.initial_head = '3'.repeat(40)
  assert.throws(() => validatePacket(full, manifest()), /stale_dispatch_base/)
})

test('reviewed red artifacts are registry-bound, current-base, independent, and single-consumer', () => {
  const plan = manifest(); const scenario = { scenario_id: 'E2E-002R', semantic_hash: digest, owner: 'scenario-owner', test_owner_task: 'E2E-002R', profile_expression: 'true', oracle_hash: digest, exclusions_hash: digest, evidence_schema_hash: digest }; const registry = { kind: 'registered', identity: digest, scenarios: [scenario] }
  const artifact = { identity: digest, provider_task: 'E2E-002R', consumer_task: 'SEC-002A', scenario_id: 'E2E-002R', profile_scope: 'shared', semantic_hash: digest, base_sha: '3f452e4dba00047b591e07617583aa4ec2387c2c', red_patch_sha: digest, red_tree_sha: digest, red_evidence_sha: digest, author_id: 'red-author', reviewer_id: 'red-reviewer', review_state: 'red_proved' }
  validateRedArtifactAdmission(plan, 'SEC-002A', registry, artifact, new Set())
  throws('red_artifact_reused', () => validateRedArtifactAdmission(plan, 'SEC-002A', registry, artifact, new Set([`{"base":"3f452e4dba00047b591e07617583aa4ec2387c2c","consumer":"SEC-002A","identity":"${digest}","profile":"shared","provider":"E2E-002R","scenario":"E2E-002R"}`])))
  throws('invalid_red_registry_binding', () => validateRedArtifactAdmission(plan, 'SEC-002A', { ...registry, scenarios: [{ ...scenario, semantic_hash: 'b'.repeat(40) }] }, artifact, new Set()))
  throws('invalid_red_artifact', () => validateRedArtifactAdmission(plan, 'SEC-002A', registry, { ...artifact, reviewer_id: 'red-author' }, new Set()))
})
