// The jemalloc decay settings that keep freed replay pages from ratcheting RSS are baked into the
// allocator at BUILD time (`JEMALLOC_SYS_WITH_MALLOC_CONF`); the prefixed `_rjem_` build does not
// read `MALLOC_CONF` after process start. They therefore only reach production if the image build
// sees `.cargo/config.toml`. `apps/engine/src/mem.rs`'s `jemalloc_decay_config_is_reported` proves
// the values are in effect for a cargo-built binary; this proves the shipped image is built the
// same way.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')

test('the engine image bakes the jemalloc decay configuration', () => {
  const cargoConfig = readFileSync(join(root, '.cargo/config.toml'), 'utf8')
  assert.match(cargoConfig, /JEMALLOC_SYS_WITH_MALLOC_CONF\s*=/, '.cargo/config.toml must bake the allocator config')
  assert.match(cargoConfig, /dirty_decay_ms:1000/, '.cargo/config.toml must set dirty_decay_ms')
  assert.match(cargoConfig, /muzzy_decay_ms:1000/, '.cargo/config.toml must set muzzy_decay_ms')

  const dockerfile = readFileSync(join(root, 'docker/Dockerfile.engine'), 'utf8')
  const build = dockerfile.slice(0, dockerfile.indexOf('cargo build'))
  assert.match(
    build,
    /^COPY\s+\.cargo\s+\.cargo\s*$/m,
    'docker/Dockerfile.engine must copy .cargo before `cargo build`, or the release image is built with jemalloc defaults',
  )
})
