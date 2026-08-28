# Multi-stack Durable Streams namespaces and engine handoff

Status: **reviewed design outline; not implemented or qualified**

As-of date: 2026-08-27

This note refines the deployment topology for installations that share one Durable Streams service
between several Circuits stacks and unrelated application streams. It also separates the deploy
handoff that the current engine can support from a future no-processing-gap architecture.

The canonical production-readiness authority remains
[`18-production-readiness-spec-reviewed.md`](18-production-readiness-spec-reviewed.md). This note does
not create a second task graph. Namespace work belongs under `DST-001`, `ENG-012`, `LEAD-001`, and
`OPS-001A/B`; continuous handoff would require a new future profile and its own reviewed task closure.
The full deployment contract is
[`2026-08-27-aws-durable-streams-and-engine-deployment-design.md`](../docs/superpowers/specs/2026-08-27-aws-durable-streams-and-engine-deployment-design.md).

## Conclusions

1. Add an explicit Durable Streams namespace to the Circuits fork. One physical Durable Streams
   service may then host multiple independent Circuits stacks, provided each stack has a distinct,
   immutable namespace.
2. Namespacing does not make two engine writers safe inside one namespace. The initial profile still
   permits one active engine per namespace and logical replication slot.
3. Use stop-completely/start-new engine replacement for the initial profile. The current candidate is
   not inert: it can acquire the slot before the old sequencer and catalog writer finish draining.
   Durable Streams remains available, so existing reads continue while control and change propagation
   pause.
4. A literal no-processing-gap deploy requires separating replication ingest from query-engine
   generations and adding downstream writer fencing. That is a future architecture, not a deployment
   configuration change.
5. Prefixes prevent collisions but are not security isolation. A shared Circuits/agent service needs
   an authenticated prefix-and-verb boundary in front of the loopback-only storage server.
6. Treat the Durable Streams volume as persistent state. Losing it can be recovered only as an
   explicit whole-generation reset and client rehydrate, not as an ordinary task restart.

## 1. Circuits stream namespace

### Current collision

The engine addresses these logical paths globally:

```text
meta/catalog
changes/<segment>
shape/<id>
```

`stack_id` is currently an observability tag, not a storage namespace. Two independent engines using
the same Durable Streams endpoint therefore fold and append the same catalog, allocate overlapping
shape ids, share change-log segments, and may retire each other's streams.

### Target mapping

Keep the paths above as engine-local logical paths and qualify them once at the Durable Streams port:

```text
logical:  meta/catalog
physical: circuits/v1/<namespace>/stores/<store-generation>/queries/<query-generation>/meta/catalog

logical:  changes/7
physical: circuits/v1/<namespace>/stores/<store-generation>/queries/<query-generation>/changes/7

logical:  shape/s42
physical: circuits/v1/<namespace>/stores/<store-generation>/queries/<query-generation>/shape/s42
```

Non-Circuits streams remain outside this prefix. For example, an application can use
`agent-runs/v1/<store-generation>/<opaque-run-id>` on the same service without becoming part of a
Circuits catalog or engine epoch.

Introduce `ELECTRIC_CIRCUITS_DS_NAMESPACE` rather than deriving storage identity from `stack_id`, a
slot name, hostname, or endpoint URL. The namespace is an operator-owned stable identifier such as
`indexed-dev-chat` or `indexed-prod-chat`; instance ids remain ephemeral and endpoint URLs may change
during replacement.

The production profile must require a non-empty namespace and validate it before any storage request.
Development may retain the unprefixed layout temporarily for compatibility. Changing a production
namespace selects a different store generation and must not be treated as an in-place restart.

### Implementation boundary

Put the translation below `DsClient`'s logical semantics and above the concrete store request:

```text
catalog / changelog / lifecycle / sequencer
                   |
                   | logical path
                   v
                DsClient
                   |
                   | namespace-qualified physical path
                   v
          DurableStreamStore adapter
```

This keeps catalog records, retention accounting, routing maps, errors, and most tests expressed in
logical paths. `stream_url()` or its replacement read locator must return the qualified public path;
it must not accidentally expose the unqualified logical path.

The namespace must also participate in the `StoreBound` identity proposed in
[`29-durable-stream-provider-evaluation.md`](29-durable-stream-provider-evaluation.md). An engine must
refuse a mismatched namespace/store generation before adopting a slot or serving a shape. A namespace
typo must not silently look like a safe new first boot in production.

### Required proof

- Two engine stacks with distinct namespaces can allocate `s1`, rotate `changes/0`, append catalogs,
  restart, and retire streams without observing or mutating one another.
- Retirement and orphan reconciliation cannot enumerate or delete outside the configured namespace.
- The same namespace refuses concurrent ownership until the leadership/fencing contract allows it.
- Returned client stream URLs contain the qualified physical path and an authenticated gateway cannot
  substitute another namespace.
- Empty, malformed, traversal-like, changed, or unexpected namespaces fail before storage or
  PostgreSQL mutation.
- The pinned Durable Streams server is qualified for nested stream paths and preserves their exact
  identity across every operation.

## 2. Initial deploy: stop completely, then start new

The present engine owns replication ingest, catalog restore, in-memory query state, shape emission,
and control traffic in one process. PostgreSQL permits only one active walsender for a logical slot,
but that slot is not a sufficient ownership barrier. The old process releases its walsender before
all sequencer and catalog work is guaranteed to have stopped, while a pre-started candidate retries
full setup and can proceed as soon as the slot is free.

```text
1. Stop new control admission and commit a named source fence `F`.
2. Signal the old engine. It becomes unready and drains or abandons its current unit according to the
   qualified shutdown contract. A final checkpoint is an optimization; recovery may replay from the
   last durable checkpoint.
3. Wait for ECS `STOPPED` and independently confirm the former container/PID can no longer execute.
   Loss of the walsender, readiness, or network contact is not termination evidence.
4. Only then start the new engine on the same storage and PostgreSQL lineage.
5. The new engine verifies lineage, acquires the slot, folds the catalog, restores state, and remains
   unready until a durable `drainedThrough(F)` receipt includes deferred output work.
6. Route new control requests only after that receipt.
```

Durable Streams is not restarted in this sequence. Existing clients can continue reading their shape
streams while the engine changes owner; those streams are temporarily stale rather than unavailable.
New shape creation and PostgreSQL-to-stream freshness pause from old-engine drain until candidate
readiness. The slot and Durable Streams logs retain work across the pause.

Do not express this as a normal rolling service with “old stays until new is ready”: it can deadlock,
and the current retrying candidate is not passive. A pre-started candidate becomes safe only after a
fail-closed activation gate prevents all PostgreSQL and Durable Streams reads/mutations until a
controller releases it following confirmed old-process termination. Liveness means “process is
running”; readiness means “this process owns the verified lineage and has drained through the named
source fence.”

This handoff still needs fault tests around every drain/acquire/restore/readiness transition. In
particular, pause the old process after walsender release but before its final sequencer/catalog
mutation and prove no successor mutation occurs before old-process termination.

## 3. Future deploy: stable ingestor plus warm generations

Eliminating even the replication-processing pause requires changing the process architecture:

```text
PostgreSQL logical slot
          |
          v
singleton circuits-ingestor
          |
          v
circuits/v1/<stack>/stores/<store-generation>/ingest/<ingest-epoch>/changes/<segment>
          |
          +-------------------------+
          |                         |
          v                         v
active query generation       candidate query generation
catalog + shapes/<id>          catalog + shapes/<id>
          |                         |
          +-----------+-------------+
                      v
          authenticated generation router
                      |
                      v
                    clients
```

The ingestor is the stable singleton that owns the PostgreSQL slot and advances slot feedback only
after a complete source transaction is durable in the shared input log. Deploying a query-engine
binary no longer transfers the slot, so ingest continues while a candidate starts.

Each query generation has isolated catalog, derived state, checkpoints, and output streams. A
candidate restores or seeds the selected template/shape demand, tails the shared input log to a named
source fence, and can be shadow-compared before promotion.

Promotion needs a durable state machine:

```text
created -> seeding -> catching_up -> caught_up -> promoted -> draining -> retired
```

At promotion, the gateway atomically selects the new generation for newly issued handles. Existing
handles stay immutably bound to the old physical stream during a bounded drain and then return typed
`410 generation_expired`; an old opaque offset is never interpreted against a remapped stream. After
every client-visible lease expires, the old catalog and output streams can be retired.

This is not safe without fencing. A partitioned old process must be unable to create, append, close,
delete, or run retention GC after promotion. The durable generation-authorization record and
recoverable two-phase transition in the full deployment specification's section 8.3 allow active and
seeding generations to write only their own paths, then atomically make the former generation
read-only at the storage boundary before new handles route to the candidate.

The future profile must additionally define:

- ownership and upgrade compatibility for the stable ingestor;
- bounded retention and independent consumer checkpoints on the shared input log;
- how shape demand is mirrored or recreated in a candidate generation;
- exact catch-up and shadow-comparison fences;
- atomic gateway generation selection and rollback;
- split-brain, delayed request, candidate crash, promotion crash, and garbage-collection cuts;
- limits for simultaneously retained generations and duplicated derived/output state.

It is blocked until the downstream mutation token, durable consumer leases/checkpoints, xid-aware
snapshot fence, demand journal, generation-router compare-and-swap, and independent caught-up/drained
receipts are all implemented and qualified. LSN comparison alone cannot decide whether a transaction
was visible in a PostgreSQL backfill snapshot.

Until those contracts are implemented and qualified, stop-completely/start-new is the supported
replacement path.

## 4. Why the Durable Streams task volume is not disposable

PostgreSQL is the source of truth for table rows, so a fresh shape can be backfilled after storage
loss. That does not make the Durable Streams volume a disposable cache. It also contains:

- the catalog epoch and slot binding;
- the change-log position and de-duplication highwater;
- shape-id allocation history, lifecycle records, subscriptions, and retirement intents;
- active and dormant shape streams addressed by already-issued client handles; and
- any unrelated durable streams, such as in-progress agent-run messages, that are not reconstructable
  from PostgreSQL until their final result is committed elsewhere.

If storage disappears while the PostgreSQL slot retains an advanced `confirmed_flush_lsn`, the
current engine can observe an empty catalog, classify the boot as new, and adopt that already-advanced
slot. New shapes can converge by backfilling current rows, but old handles and offsets refer to a lost
generation. Resetting shape ids can also reuse an old physical `shape/sN` path for a different query
while a client still holds the former handle. Recovery therefore requires a new public/store
generation, invalidation of every old handle, and a full client rehydrate before serving—not silent
first-boot adoption.

A database/backfill stampede is a real secondary consequence: every connected client may recreate
feeds at once, multiplying snapshot reads, output writes, and catch-up work. Admission limits and
jitter can reduce that load, but they do not repair the lost generation or non-reconstructable agent
events.

For the first profile, replacement tasks must remount the same persistent volume. A task-scoped volume
that is deleted on every termination is suitable only after an explicitly authorized whole-generation
reset workflow, and should not be the normal deploy path.
