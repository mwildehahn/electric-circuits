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
timeout; `0` waits forever) and then retires the old identity, returning the same typed recreate
outcome with reason `JoinTimedOut`. Create callers fall through to a fresh shape id; read/join
callers receive the surface-specific recreate signal. The detached replay is discarded when it
settles.

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
size or shape count. The response cap is sized from the store's own readiness, not from a fixed
number: a store that advertises a page gets 16 MiB (four of its pages, so the cap is never reached),
and a store that advertises none gets 64 MiB, because it answers a read with the whole remainder of
the stream and a cap below the backlog does not bound memory — the sequencer retries the identical
read and it fails identically forever, so no data flows at all. A deployment that wants the tighter
bound guaranteed sets `ELECTRIC_CIRCUITS_REQUIRE_DS_CHUNK_CAP=1` and runs a store that pages. If a
live read nevertheless exceeds its cap, the sequencer records a typed cap failure, increments
`sequencer_read_cap_failures_total`, logs an error, latches the engine `degraded`/not-ready status,
and halts further reads until restart rather than retrying the same page forever. Reactivation latency can queue behind the semaphore. `changes_only` shapes
remain active and therefore consume their normal routing state because recreation would lose their
dormant-period history. Durable Streams server paging (PR #4) improves the normal page size, but
the engine-side cap remains mandatory defense in depth. Cross-segment span calculation HEADs each
segment, subtracts the parked byte offset on the first, sums complete intermediate segments, and
stops at the ingestor's current tail (`Engine::changes_position`), which over-estimates the span by
the sequencer's lag — deliberately, since an admission budget must never under-count.

The replay itself is fenced at a different, exact point: the change-log position the sequencer
carries back in the `BeginShape` ack, which is where that shape's pending buffer starts collecting.
Everything before the fence is the replay's to deliver, everything after it is the buffer's, and the
two meet with neither a gap nor a dependency on how far apart the ingestor's tail and the
sequencer's cursor are. A fence captured earlier — the ingestor tail at touch time, before the
admission HEADs and the sequencer's state lock — would leave the envelopes processed in between to
neither path: the replay stops at the fence (and against a store that pages, its last page ends
short of the tail), and the buffer did not exist yet. Overlap is harmless because the replay appends
absolute per-pk rows; a gap is permanent.
