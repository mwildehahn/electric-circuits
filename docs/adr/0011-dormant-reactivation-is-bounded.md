# ADR-0011: Dormant reactivation is bounded

## Decision

Dormant plain shapes may be reactivated by replaying the change log, but every Durable Streams
read is page-bounded and the engine caps concurrent replays with
`ELECTRIC_CIRCUITS_REACTIVATION_CONCURRENCY` (default `2`). Replay decoding filters by envelope
type before materializing row bodies. `changes_only` feeds are exempt from dormancy: recreating one
from a Postgres snapshot would silently discard the changes-only history promised to subscribers.

The replay span budget is configurable with `ELECTRIC_CIRCUITS_REPLAY_MIN_BYTES` (default 16 MiB)
and `ELECTRIC_CIRCUITS_REPLAY_MULTIPLIER` (default 4). Plain-shape catalog records persist the
last streamed backfill's row and byte estimates (missing values from older catalogs mean unknown).
At wake, the engine computes the byte span across retained segment HEADs and replays only when
`span <= max(min_bytes, multiplier * backfill_bytes)`; unknown sizing uses `min_bytes` alone.
Over-budget or unresumable shapes are retired through the closed-stream path so subscribers
recreate them from a fresh snapshot. The retention sweeper applies the same admission check before
pressure/TTL eviction.

The retirement is typed (`ReactivationRecreate`), and each surface answers it in its own recreate
vocabulary rather than as a server fault:

* **Native create/join** (`POST /v1/shapes`, `POST /v1/subset-feeds`): the request falls through to
  a fresh create in the same round trip and returns `2xx` carrying a NEW shape id. This is the only
  outcome today's iOS SDK recreates from on its own — it treats a create/join answer whose shape id
  differs from the one it asked for as a replacement and reseeds.
* **Native read routes** (`/shapes/{id}/rows`, `/shapes/{id}/log`, and a create whose fall-through
  is exhausted): `410 Gone`. `410` is what that SDK's stream reader already maps to "gone, get a
  fresh one"; `409` it classifies as a terminal failure with no reseed, and `404` is ambiguous with
  an unknown shape id.
* **Electric `/v1/shape`**: the protocol's own `must-refetch` (`409` plus the control message), the
  same answer a genuinely gone stream produces.

A wake that is admitted can still outlast the request that triggered it: a large-span replay was
measured at ~40s in production against an API gateway whose read timeout is 30s, so the client got
a 503 carrying nothing to act on. A touch therefore waits at most
`ELECTRIC_CIRCUITS_REACTIVATION_JOIN_TIMEOUT_SECS` (default 20s, deliberately under that gateway
timeout; `0` waits forever) and then returns the same typed recreate outcome with reason
`JoinTimedOut`. The shape is NOT retired and the detached replay runs to completion, so a later
touch finds it active. Unlike the over-budget reason, a timed-out join is not redone as a fresh
create inside the same request: the shape is still reactivating, so a redo would rejoin the same
replay and spend another full timeout — the overrun this exists to bound.

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
remain active and therefore consume their normal routing state because recreation would lose their
dormant-period history. Durable Streams server paging (PR #4) improves the normal page size, but
the engine-side cap remains mandatory defense in depth. Cross-segment span calculation HEADs each
segment, subtracts the parked byte offset on the first, sums complete intermediate segments, and
stops at the current processed tail.
