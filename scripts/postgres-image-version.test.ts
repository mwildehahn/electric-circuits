import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'

import { postgres18Tools } from './postgres18.js'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const expectedImage = 'postgres:18.6'
const surfaces = ['docker/compose.yaml', 'docker/compose.electric.yaml', 'docker/README.md', 'tutorials/compose.yaml']
const ephemeralLaunchers = [
  'examples/linearlite/start.ts',
  'examples/web/start.ts',
  'packages/bench/src/shape-mem-matrix.ts',
  'packages/bench/src/shape-mem-scale.ts',
  'packages/loadgen/src/infra.ts',
]

test('Docker Postgres surfaces use one explicit PostgreSQL 18 minor image', () => {
  for (const relativePath of surfaces) {
    const content = readFileSync(join(root, relativePath), 'utf8')
    assert.match(content, new RegExp(`\\b${expectedImage.replace('.', '\\.')}(?:\\b|$)`), relativePath)
    assert.doesNotMatch(content, /postgres:16(?:\b|$)/, `${relativePath} still references PostgreSQL 16`)
  }

  const compose = readFileSync(join(root, 'docker/compose.yaml'), 'utf8')
  const electricCompose = readFileSync(join(root, 'docker/compose.electric.yaml'), 'utf8')
  const tutorials = readFileSync(join(root, 'tutorials/compose.yaml'), 'utf8')
  assert.match(compose, /postgres:\n(?:.|\n)*?image:\s*postgres:18\.6/)
  assert.match(electricCompose, /postgres:\n(?:.|\n)*?image:\s*postgres:18\.6/)
  assert.match(tutorials, /postgres:\n(?:.|\n)*?image:\s*postgres:18\.6/)
  assert.match(compose, /- pg-data:\/var\/lib\/postgresql\s*\n/)
  assert.doesNotMatch(compose, /- pg-data:\/var\/lib\/postgresql\/data\s*\n/)
})

test('ephemeral launchers select PostgreSQL 18 tools explicitly', () => {
  for (const relativePath of ephemeralLaunchers) {
    const content = readFileSync(join(root, relativePath), 'utf8')
    assert.match(content, /postgres18Tools/, `${relativePath} does not use the PostgreSQL 18 resolver`)
    assert.doesNotMatch(content, /execFileSync\(['"](?:initdb|pg_ctl)['"]/, `${relativePath} selects PostgreSQL from PATH`)
  }

  const tools = postgres18Tools()
  assert.match(execFileSync(tools.initdb, ['--version'], { encoding: 'utf8' }), /PostgreSQL\) 18\./)
})
