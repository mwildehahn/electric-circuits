# ADR-0011: Dormant reactivation is bounded

## Decision

Dormant plain shapes may be reactivated by replaying the change log, but every Durable Streams
read is page-bounded and the engine caps concurrent replays with
`ELECTRIC_CIRCUITS_REACTIVATION_CONCURRENCY` (default `2`). Replay decoding filters by envelope
type before materializing row bodies. `changes_only` feeds are exempt from dormancy: recreating one
from a Postgres snapshot would silently discard the changes-only history promised to subscribers.

The replay span budget is configurable with `ELECTRIC_CIRCUITS_REPLAY_MIN_BYTES` (default 16 MiB)
and `ELECTRIC_CIRCUITS_REPLAY_MULTIPLIER` (default 4) and is represented in `RetentionConfig`.
Persisting per-shape backfill sizing and enforcing replay-vs-recreate admission is follow-up work;
until then the budget knobs are reserved and no shape is silently recreated.

## Alternatives considered

* **Unbounded replay:** preserves stream identity but makes cost proportional to global log growth
  and multiplies memory by the number of waking shapes. Rejected after the multi-GiB OOM incident.
* **Eviction-only:** always close/delete dormant streams and force a fresh backfill. Safe for plain
  materialized shapes, but incorrect for `changes_only` feeds and needlessly loses stream identity.
* **Reconcile in place:** rerun a backfill and reconcile against the retained stream. This can
  preserve identity and bound work, but requires a durable per-shape reconciliation protocol and is
  deferred until its causal and failure semantics are specified.

## Consequences

Replay memory is bounded by the response cap, parsed page, and scheduler permits rather than log
size or shape count. Reactivation latency can queue behind the semaphore. `changes_only` shapes
remain active and therefore consume their normal routing state until a correct recreate/reconcile
implementation exists. Durable Streams server paging (PR #4) improves the normal page size, but
the engine-side cap remains mandatory defense in depth.
