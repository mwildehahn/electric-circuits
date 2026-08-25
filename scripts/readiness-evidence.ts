import { createHash } from 'node:crypto'
import { execFileSync, spawnSync } from 'node:child_process'
import { existsSync, lstatSync, mkdirSync, readdirSync, readFileSync, realpathSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'

export type Json = Record<string, unknown>

export type CommandSpec = {
  id: string
  argv: string[]
  cwd?: string
  env?: Record<string, string>
  required?: boolean
}

export type CommandResult = {
  id: string
  argv: string[]
  cwd: string
  started_at: string
  finished_at: string
  exit_code: number
  signal: string | null
  stdout_sha256: string
  stderr_sha256: string
  stdout: string
  stderr: string
  status: 'pass' | 'fail' | 'blocked'
  blocker?: string
}

export type EvidenceRow = {
  source_strategy: 'fresh_detached_worktree' | 'verified_tree_export'
  source_commit: string
  source_tree: string
  pre_attestation_sha256: string
  post_attestation_sha256: string
  external_input_manifest_sha256: string
  mount_topology_sha256: string
  run_root_identity: string
  empty_run_root_attestation_sha256: string
  effective_config_sha256: string
  source_clean: boolean
  mount_read_only: boolean
  run_root_new_empty: boolean
  post_source_unchanged: boolean
  external_inputs_unchanged: boolean
  config_matches: boolean
  overlay_writable?: boolean
  run_root_reused?: boolean
  source_dirty?: boolean
  post_source_mutated?: boolean
  config_mismatch?: boolean
  external_input_mutated?: boolean
}

const hash = (bytes: string | Uint8Array) => createHash('sha256').update(bytes).digest('hex')
export const canonicalJson = (value: unknown): string => JSON.stringify(value, (_key, entry) => {
  if (!entry || typeof entry !== 'object' || Array.isArray(entry)) return entry
  return Object.fromEntries(Object.entries(entry).sort(([a], [b]) => a.localeCompare(b)))
})
export const sha256 = (value: unknown) => hash(typeof value === 'string' || value instanceof Uint8Array ? value : canonicalJson(value))

const git = (cwd: string, ...args: string[]) => execFileSync('git', args, { cwd, encoding: 'utf8' }).trim()
export const sourceIdentity = (cwd: string) => ({ commit: git(cwd, 'rev-parse', 'HEAD'), tree: git(cwd, 'rev-parse', 'HEAD^{tree}') })

/** Create an evidence-only detached worktree. The caller owns cleanup and must never edit it. */
export const createDetachedWorktree = (repository: string, commit: string, destination: string) => {
  if (existsSync(destination)) throw new Error('reused_source')
  mkdirSync(dirname(destination), { recursive: true })
  execFileSync('git', ['worktree', 'add', '--detach', destination, commit], { cwd: repository, encoding: 'utf8' })
  const identity = sourceIdentity(destination)
  if (identity.commit !== commit) throw new Error('wrong_commit_tree')
  return identity
}

export const removeDetachedWorktree = (repository: string, destination: string) => {
  execFileSync('git', ['worktree', 'remove', '--force', destination], { cwd: repository, encoding: 'utf8' })
}

export const sourceAttestation = (cwd: string, allowedIgnored = new Set<string>()): Json => {
  const status = git(cwd, 'status', '--porcelain=v1', '--untracked-files=all')
  const ignored = git(cwd, 'clean', '-ndx').split('\n').filter(Boolean).map(line => line.replace(/^Would remove /, ''))
  const undeclaredIgnored = ignored.filter(path => !allowedIgnored.has(path) && ![...allowedIgnored].some(prefix => path.startsWith(`${prefix}/`)))
  return { status, ignored, undeclared_ignored: undeclaredIgnored, clean: status.length === 0 && undeclaredIgnored.length === 0, digest: sha256({ status, ignored, undeclared_ignored: undeclaredIgnored }) }
}

export const assertRunRoot = (root: string, expectedIdentity?: string) => {
  const resolvedRoot = resolve(root)
  if (!root || !(root.startsWith('/tmp/') || root.startsWith('/private/tmp/') || resolvedRoot.startsWith('/tmp/') || resolvedRoot.startsWith('/private/tmp/'))) throw new Error('run_root_outside_external_tmp')
  if (existsSync(root)) {
    if (readdirSync(root).length !== 0) throw new Error('nonempty_run_root')
    if (expectedIdentity && realpathSync(root) !== root) throw new Error('reused_run_root')
  } else mkdirSync(root, { recursive: true, mode: 0o700 })
  const identity = sha256(`${root}:${Date.now()}:${process.pid}`)
  return { identity, empty_attestation: sha256(readdirSync(root)) }
}

export const validateExternalInputs = (manifest: Json, initial: string, current: Json, topology: Json) => {
  if (manifest.access !== 'read_only') throw new Error('writable_external_inputs')
  if (sha256(current) !== initial) throw new Error('external_input_mutated')
  if (topology.writable === true) throw new Error('writable_overlay')
  if (topology.declared !== true) throw new Error('undeclared_overlay')
  return true
}

export const validateEvidenceRow = (row: EvidenceRow) => {
  if (!['fresh_detached_worktree', 'verified_tree_export'].includes(row.source_strategy)) throw new Error('invalid_source_strategy')
  const checks: Array<[string, boolean]> = [
    ['dirty_source', row.source_clean !== true || row.source_dirty === true],
    ['writable_mount', row.mount_read_only !== true || row.overlay_writable === true],
    ['nonempty_run_root', row.run_root_new_empty !== true || row.run_root_reused === true],
    ['post_source_mutation', row.post_source_unchanged !== true || row.post_source_mutated === true],
    ['external_input_mutated', row.external_inputs_unchanged !== true || row.external_input_mutated === true],
    ['effective_config_mismatch', row.config_matches !== true || row.config_mismatch === true],
  ]
  for (const [reason, bad] of checks) if (bad) throw new Error(reason)
  return true
}

export const buildEvidenceRow = (args: {
  source: { cwd: string; expectedCommit: string; expectedTree: string }
  runRoot: string
  externalManifest: Json
  externalManifestDigest: string
  mountTopology: Json
  mountTopologyDigest: string
  effectiveConfig: Json
  expectedConfigDigest: string
  allowedIgnored?: Set<string>
}): EvidenceRow => {
  const before = sourceAttestation(args.source.cwd, args.allowedIgnored)
  const identity = sourceIdentity(args.source.cwd)
  if (identity.commit !== args.source.expectedCommit || identity.tree !== args.source.expectedTree) throw new Error('wrong_commit_tree')
  const run = assertRunRoot(args.runRoot)
  validateExternalInputs(args.externalManifest, args.externalManifestDigest, args.externalManifest, args.mountTopology)
  if (sha256(args.mountTopology) !== args.mountTopologyDigest) throw new Error('mount_topology_mutated')
  if (sha256(args.effectiveConfig) !== args.expectedConfigDigest) throw new Error('effective_config_mismatch')
  const after = sourceAttestation(args.source.cwd, args.allowedIgnored)
  const row: EvidenceRow = {
    source_strategy: 'fresh_detached_worktree', source_commit: identity.commit, source_tree: identity.tree,
    pre_attestation_sha256: String(before.digest), post_attestation_sha256: String(after.digest),
    external_input_manifest_sha256: args.externalManifestDigest, mount_topology_sha256: args.mountTopologyDigest,
    run_root_identity: run.identity, empty_run_root_attestation_sha256: run.empty_attestation,
    effective_config_sha256: args.expectedConfigDigest, source_clean: before.clean === true && after.clean === true,
    mount_read_only: args.mountTopology.writable !== true, run_root_new_empty: true,
    post_source_unchanged: before.digest === after.digest, external_inputs_unchanged: true, config_matches: true,
  }
  validateEvidenceRow(row)
  return row
}

export const runCommand = (spec: CommandSpec): CommandResult => {
  const cwd = spec.cwd ?? process.cwd()
  const started = new Date().toISOString()
  const result = spawnSync(spec.argv[0]!, spec.argv.slice(1), { cwd, env: { ...process.env, ...spec.env }, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 })
  const stdout = result.stdout ?? ''
  const stderr = result.stderr ?? ''
  const exitCode = typeof result.status === 'number' ? result.status : 1
  const blocker = result.error?.message ?? (result.signal ? `signal:${result.signal}` : undefined)
  return {
    id: spec.id, argv: spec.argv, cwd, started_at: started, finished_at: new Date().toISOString(),
    exit_code: exitCode, signal: result.signal ?? null, stdout_sha256: hash(stdout), stderr_sha256: hash(stderr), stdout, stderr,
    status: result.error ? 'blocked' : exitCode === 0 ? 'pass' : 'fail', ...(blocker ? { blocker } : {}),
  }
}

export const baselineCommands = (root: string): CommandSpec[] => [
  { id: 'format', argv: ['cargo', 'fmt', '--check'], cwd: root },
  { id: 'typecheck', argv: ['pnpm', 'typecheck'], cwd: root },
  { id: 'engine-tests', argv: ['pnpm', 'engine:test'], cwd: root },
  { id: 'vitest', argv: ['pnpm', 'test'], cwd: root, env: { ELECTRIC_CIRCUITS_ENGINE_PREBUILT: '1' } },
  { id: 'conformance', argv: ['pnpm', 'test:conformance'], cwd: root },
  { id: 'fuzz', argv: ['pnpm', 'test:fuzz'], cwd: root },
  { id: 'swift-boundaries', argv: ['bash', '../electric-sync-swift/Scripts/check-dependency-boundaries.sh'], cwd: root },
  { id: 'swift-tests', argv: ['swift', 'test'], cwd: resolve(root, '../electric-sync-swift') },
  { id: 'electric-oracle', argv: ['./electric-conformance/run.sh', 'oracle'], cwd: root },
  { id: 'electric-property', argv: ['./electric-conformance/run.sh', 'property'], cwd: root },
  { id: 'electric-subqueries', argv: ['./electric-conformance/run.sh', 'subqueries'], cwd: root },
  { id: 'browser-demo', argv: ['pnpm', 'demo:linearlite'], cwd: root },
]

export const runBaseline = (root: string, commands = baselineCommands(root)) => commands.map(runCommand)

export const writeBaseline = (path: string, value: unknown) => {
  mkdirSync(dirname(path), { recursive: true })
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`)
}

if (process.argv[1] && realpathSync(process.argv[1]) === realpathSync(new URL(import.meta.url).pathname)) {
  const command = process.argv[2] ?? 'self-test'
  if (command === 'run') {
    const root = resolve(process.argv[3] ?? process.cwd())
    const output = resolve(process.argv[4] ?? join(root, 'docs/production/validation-baseline.json'))
    const results = runBaseline(root)
    writeBaseline(output, { schema_version: 'evidence-v2', generated_at: new Date().toISOString(), source: sourceIdentity(root), commands: results.map(({ stdout, stderr, ...result }) => result), blockers: results.filter(result => result.status !== 'pass').map(result => ({ id: result.id, status: result.status, reason: result.blocker ?? `exit:${result.exit_code}` })) })
  }
}
