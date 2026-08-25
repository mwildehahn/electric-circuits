import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { assertRunRoot, canonicalJson, sha256, validateEvidenceRow, validateExternalInputs } from './readiness-evidence.js'

const goodRow = () => ({
  source_strategy: 'fresh_detached_worktree' as const,
  source_commit: 'a'.repeat(40), source_tree: 'b'.repeat(40),
  pre_attestation_sha256: 'c'.repeat(64), post_attestation_sha256: 'd'.repeat(64),
  external_input_manifest_sha256: 'e'.repeat(64), mount_topology_sha256: 'f'.repeat(64),
  run_root_identity: 'run-root-a', empty_run_root_attestation_sha256: '1'.repeat(64), effective_config_sha256: '2'.repeat(64),
  source_clean: true, mount_read_only: true, run_root_new_empty: true, post_source_unchanged: true,
  external_inputs_unchanged: true, config_matches: true,
})

const rejects = (pattern: string, fn: () => unknown) => assert.throws(fn, new RegExp(pattern))

test('canonical JSON and hashes are stable across object key order', () => {
  assert.equal(canonicalJson({ z: 1, a: { y: 2, x: 3 } }), '{"a":{"x":3,"y":2},"z":1}')
  assert.equal(sha256({ a: 1, b: 2 }), sha256({ b: 2, a: 1 }))
})

test('fresh run roots reject reused and nonempty roots', () => {
  const root = mkdtempSync('/tmp/readiness-evidence-')
  const first = assertRunRoot(root)
  assert.ok(first.identity)
  writeFileSync(join(root, 'artifact.log'), 'mutated')
  rejects('nonempty_run_root', () => assertRunRoot(root))
  rejects('run_root_outside_external_tmp', () => assertRunRoot('/var/tmp/not-allowed'))
})

test('evidence validator rejects source, mount, run-root, external and config mutations', () => {
  const fields: Array<[string, string, unknown]> = [
    ['source_dirty', 'dirty_source', true], ['overlay_writable', 'writable_mount', true],
    ['run_root_reused', 'nonempty_run_root', true], ['post_source_mutated', 'post_source_mutation', true],
    ['external_input_mutated', 'external_input_mutated', true], ['config_mismatch', 'effective_config_mismatch', true],
  ]
  for (const [field, reason, value] of fields) rejects(reason, () => validateEvidenceRow({ ...goodRow(), [field]: value }))
  assert.equal(validateEvidenceRow(goodRow()), true)
})

test('external input validator requires immutable read-only declared topology', () => {
  const manifest = { access: 'read_only' }
  const current = { resolver: 'pnpm-frozen', dependency_sha: 'abc' }
  const digest = sha256(current)
  assert.equal(validateExternalInputs(manifest, digest, current, { declared: true, writable: false }), true)
  rejects('writable_external_inputs', () => validateExternalInputs({ access: 'read_write' }, digest, current, { declared: true }))
  rejects('external_input_mutated', () => validateExternalInputs(manifest, digest, { ...current, dependency_sha: 'changed' }, { declared: true }))
  rejects('writable_overlay', () => validateExternalInputs(manifest, digest, current, { declared: true, writable: true }))
  rejects('undeclared_overlay', () => validateExternalInputs(manifest, digest, current, { declared: false }))
})

test('mutation fixtures cover tracked/staged/untracked source observations', () => {
  const attestation = (status: string, undeclared: string[]) => ({ clean: status === '' && undeclared.length === 0, status, undeclared_ignored: undeclared })
  assert.equal(attestation('', []).clean, true)
  assert.equal(attestation(' M tracked', []).clean, false)
  assert.equal(attestation('M  staged', []).clean, false)
  assert.equal(attestation('?? untracked', []).clean, false)
  assert.equal(attestation('', ['cache']).clean, false)
})

test('every scripts node:test file has an explicit product-gate or planner-audit lane', () => {
  const root = join(dirname(fileURLToPath(import.meta.url)), '..')
  const scripts = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')).scripts as Record<string, string>
  const productGate = ['scripts/postgres-image-version.test.ts', 'scripts/readiness-evidence.test.ts']
  const plannerAudit = ['scripts/readiness-plan.test.ts']
  const discovered = readdirSync(join(root, 'scripts'))
    .filter((name) => name.endsWith('.test.ts'))
    .map((name) => `scripts/${name}`)
    .sort()

  assert.match(scripts['test:node'], /Supported product Node test gate/)
  assert.match(scripts['test:readiness-plan'], /Non-gating planner audit/)
  for (const file of productGate) assert.match(scripts['test:node'], new RegExp(file.replace('.', '\\.')))
  for (const file of plannerAudit) assert.match(scripts['test:readiness-plan'], new RegExp(file.replace('.', '\\.')))
  assert.deepEqual([...productGate, ...plannerAudit].sort(), discovered)
})
