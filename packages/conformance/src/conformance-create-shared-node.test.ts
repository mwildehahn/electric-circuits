// A membership change that lands while a subquery shape is being created must reach that shape.
//
// A subquery create commits its dependency edges at registration (phase A), then backfills the outer
// rows from a Postgres snapshot (phase B), then installs the shape (phase C). When the inner node is
// SHARED with an already-live shape, a flip on it during phase B is propagated at once — and reaches
// the new shape's edge before the shape is installed. Nothing buffers it for the new shape: a fresh
// node's inner deltas are buffered on the node, outer deltas on the pending shape, but a shared node's
// flip is neither. If the inner change committed after the backfill's snapshot, the new shape never
// sees those rows.
//
// This is the shape a private-tier consumer creates constantly: K shapes per subject sharing one
// membership node, every one after the first exposed during its own backfill.
//
// Made deterministic with the snapshot itself: the backfill takes its REPEATABLE READ snapshot with
// its fence statement and only then reads the outer table, so an `ACCESS EXCLUSIVE` lock on that
// table parks the create AFTER its snapshot is fixed. The inner change then commits, propagates
// through the shared node (observed on the live sibling), and only then is the create released.

import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import type { Schema } from '@electric-circuits/protocol'

import { bootHarness, drainEngine, type Harness } from './harness.js'
import { createShape, lockTable, pgQuery, sleep, streamKeys, waitFor, waitForLockWaiter } from './engine-native.js'

const schema: Schema = {
  tables: {
    parent: { columns: { id: { type: 'int' }, active: { type: 'bool' } }, primaryKey: 'id' },
    child: { columns: { id: { type: 'int' }, parent_id: { type: 'int' } }, primaryKey: 'id' },
    child2: { columns: { id: { type: 'int' }, parent_id: { type: 'int' } }, primaryKey: 'id' },
  },
}

const activeParents = { table: 'parent', project: 'id', where: { col: 'active', op: 'eq', value: true } } as const

describe('native: a shared-node flip during a create reaches the created shape', () => {
  let h: Harness
  beforeAll(async () => {
    h = await bootHarness(schema)
  }, 60000)
  afterAll(async () => await h?.shutdown())

  it('a move-in committed after the backfill snapshot lands on the new shape', async () => {
    await pgQuery(h, 'INSERT INTO parent (id, active) VALUES (1, true), (2, false)')
    await pgQuery(h, 'INSERT INTO child (id, parent_id) VALUES (1, 1), (2, 1), (3, 1), (4, 2), (5, 2), (6, 2)')
    await pgQuery(h, 'INSERT INTO child2 (id, parent_id) VALUES (1, 1), (2, 1), (3, 1), (4, 2), (5, 2), (6, 2)')
    await drainEngine(h)

    // The live sibling: seeds the shared node and lets us watch the flip propagate.
    const x = await createShape(h, { table: 'child', where: { col: 'parent_id', in: activeParents } })
    await drainEngine(h)
    expect(await streamKeys(x.streamUrl)).toEqual(['1', '2', '3'])

    // Park Y's create after its snapshot, before it reads the outer table.
    const lock = await lockTable(h, 'child2')
    const yCreate = createShape(h, { table: 'child2', where: { col: 'parent_id', in: activeParents } })
    let y
    try {
      await waitForLockWaiter(h, "Y's backfill to block on the table lock")
      // The inner change: parent 2 becomes active → value 2 enters the shared node.
      await pgQuery(h, 'UPDATE parent SET active = true WHERE id = 2')
      // Propagated through the node: the live sibling has its move-ins. Y's edge was visited in the
      // same walk.
      await waitFor(
        async () => (await streamKeys(x.streamUrl)).join(',') === '1,2,3,4,5,6',
        'the flip to reach the live sibling',
      )
      await sleep(300)
    } finally {
      await lock.release()
    }
    y = await yCreate
    await drainEngine(h)
    expect(await streamKeys(y.streamUrl)).toEqual(['1', '2', '3', '4', '5', '6'])
  }, 60000)
})
