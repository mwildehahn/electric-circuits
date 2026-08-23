# Contract protocol

Use this reference when creating or reviewing a behavior-changing black-box E2E contract.

## Select the contract before the test framework

Write a stable scenario whose observable promise would survive replacing the circuit, routing tier,
catalog layout, task graph, or client implementation. Give it a scenario ID, selected profile and
capability expression, a semantic contract hash, public setup/action/outcome, exclusions, source
journal, independent oracle, fault-cut tier, diagnostic deadline, evidence schema, and named test,
implementation, integration, and reviewer owners.

An excluded capability is a contract too: prove that it is rejected before it creates engine,
storage, or database work. Do not let a broad happy-path test imply transaction atomicity, subset,
aggregate, migration, native-sink, or compatibility support that the selected profile does not
enable.

## Causal fence and oracle

For replicated behavior, the mutating PostgreSQL transaction writes the changed data and a
harness-issued `SourceCommitID`; write the sentinel last in that transaction. The comparison is
legal only after this receipt chain:

```text
source.committed(commit-id)
  -> server.drainedThrough(commit-id, including deferred work)
  -> target read starts after that receipt
  -> client/cache fold commits through its returned tail
  -> client.appliedTailAfter(commit-id, principal, template, generation)
  -> independent SQL/reference result at that source prefix
```

This causal-fence protocol is **target infrastructure for `E2E-000A`, not a current
implementation claim**. Its target contract requires a harness-only marker relation in the
immutable, explicit test publication. The relation is never a public template. The harness observes
the marker only after the transaction's terminal envelope, and the server emits its receipt only
after all work causally preceding the marker, including deferred work, has completed. The harness
must prove the contract rejects an unpublished marker, a marker observed before transaction end,
and a receipt that skips deferred work. The public client need not receive the marker.

The oracle is a checked-in SQL/projection/key definition or separate reference evaluator, authored
independently from the production template/compiler. It either holds the source prefix in a
repeatable-read view or evaluates the operation journal only through the commit ID. It must notice
tenant/predicate mistakes, missing versus NULL, scalar and key fidelity, deletes/tombstones,
generations, and later-write contamination.

Private replication positions, changelog state, and deferred-work gauges may implement
`server.drainedThrough`, but are diagnostics. They are not a public target receipt. A separate
sentinel stream, a raw row-map comparison before application commit, or SQL after unrelated writes
cannot substitute for the chain. If a compatibility transport cannot attach an in-lane receipt,
quiesce writes and require an explicit per-template caught-up plus cache-commit receipt.

## Genuine red-to-green provenance

1. Freeze the scenario and oracle hash, then add the contract patch without behavior that can make
   it pass.
2. Run the exact focused command on that frozen red-patch tree. Its intended public assertion must
   execute and fail; capture expected versus actual output and the first semantic divergence.
3. Preserve the red patch, replay command, red source/tree SHA, profile/config/toolchain and
   image digests, journal/seed, raw-log hash, and the clean evidence runner's pre/post source and
   effective-config attestation hashes. The run comes from a newly created detached worktree at the
   exact commit (or verified prepared-tree export), never the editing checkout. Immutable dependency/
   tool/fixture inputs and read-only package-manager mounts are content/topology-attested; each run's
   writable builds, caches, fixtures and evidence use a unique initially empty external root.
4. Implement from that exact red patch. The green run uses the same scenario, test source,
   exclusions, oracle and hashes. A semantic contract change creates a new red/green pair.
5. Prove the oracle is live with a deliberately bad adapter, mutation, or disabled-hook case where
   feasible. An environmental failure, compile error, timeout, skip, permanent expected failure,
   inverted assertion, or mock-only failure does not establish red.

When test and implementation have different owners, the contract author supplies a stacked red
patch. The implementer may not weaken the test or oracle, and the qualification runner may not
repair the scenario or implementation it evaluates.

## Faults, clocks, and isolation

Create order by awaiting named events: arrival, hold, release/cancel/kill, and terminal receipt.
Each wait has one diagnostic deadline that records phase, commit ID, receipts, redacted request IDs,
component state, logs, and resources. A sleep may not manufacture a race or prove convergence.
Injected clocks cover lease/TTL/retry/retention/rotation at `t-1`, `t`, and `t+1`; real elapsed time
is reserved for a timeout that is itself a public PG or wire contract.

External candidate cuts belong at process, network/request/response, storage, volume, readiness,
cache transaction, and source-commit boundaries. Same-SHA instrumented hooks may test fsync,
checkpoint, catalog, scheduler, or rotation mechanics, but never stand in for release-image E2E.
Run a hooks-disabled equivalence smoke.

Each stack owns a unique database/schema, slot and publication, storage namespace/volume, network
and ports, gateway tenant, local cache, temp/artifact directory, process group, source journal and
fault gate. Cleanup proves that it removed only those resources.

## Required matrix when relevant

Cover the source transaction/snapshot fence, replay and partial transaction boundaries, response
loss around lifecycle acknowledgment, cancellation, component and storage restart, schema/drift/
TRUNCATE/slot continuity, security and revocation, slow reader/backpressure, and resource limits.
State the only permitted terminal outcomes: exact continuation, safe duplicate replay, completed
retirement, typed reset/refetch, or admission rejection. Record any narrower proof obligation rather
than silently omitting a matrix cell.
