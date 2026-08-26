import { describe, expect, it } from 'vitest'

import { MAX_COUNT, parseArgs } from './seed-tasks.js'

describe('LinearLite task seeder arguments', () => {
  it('requires a bounded count and applies safe defaults', () => {
    expect(parseArgs(['--count', '2'])).toEqual({
      count: 2,
      titlePrefix: 'Swift live task',
      projectId: undefined,
      status: 'todo',
      priority: 'medium',
    })
    expect(() => parseArgs([])).toThrow('--count is required')
    expect(() => parseArgs(['--count', String(MAX_COUNT + 1)])).toThrow(`at most ${MAX_COUNT}`)
  })

  it('accepts inline options and rejects invalid values', () => {
    expect(parseArgs(['--count=3', '--project=7', '--status=in_progress', '--priority=urgent', '--title-prefix=From Swift=v2'])).toEqual({
      count: 3,
      titlePrefix: 'From Swift=v2',
      projectId: 7,
      status: 'in_progress',
      priority: 'urgent',
    })
    expect(() => parseArgs(['--count', '1', '--status', 'active'])).toThrow('one of')
    expect(() => parseArgs(['--count', '1', '--unknown'])).toThrow('unknown option')
  })

  it('supports help without requiring a database', () => {
    expect(parseArgs(['--help'])).toBe('help')
  })
})
