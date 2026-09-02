# The change log is a supported read contract; external consumers pin its segments

Status: accepted (2026-09-02)

Mighty's kernel needs the ordered committed changes, including old rows, from each published
plugin database. Polling a transactional outbox is a second database reader with a wake-up delay,
although the engine already writes that same ordered data to its per-source segmented change log.
The change log therefore becomes a supported external-consumer contract, as decided in
[`electriccircuits-dec-44f`](cite:electriccircuits-dec-44f): an external reader discovers the
addressable route and current position, reads unmodified positioned durable-stream pages, and
explicitly reports a durable `(segment, offset)` position. The ingestor remains the only writer.

An explicit position pins every segment at or after it. The deletion floor is the minimum of the
durable sequencer checkpoint, dormant-shape resume positions, and external consumer positions.
Consumer pins are cataloged as `ConsumerPinned` and released by `ConsumerEvicted`; a pin is visible
only after its catalog event lands. A stale consumer is evicted durably before its formerly pinned
segments may be deleted. If that durable eviction fails, the next re-plan still sees the pin and
defers deletion. A restored pin whose segment is not retained refuses boot rather than publishing a
consumer that cannot resume.

## Considered options

- Keep the kernel on its transactional outbox: preserves a second reader, polling delay, and a
  second durability/feed protocol for bytes already in the change log.
- Add a per-source sink that copies every decoded change into another stream: duplicates the bytes
  and introduces another ordered writer without improving the reader contract.
- Model the kernel feed as a table shape: gives state snapshots and materialized-view semantics,
  not an ordered old-row change feed.

## Consequences

The reader receives at-least-once delivery and must follow the sequencer's own rules: discard
`__circuits.control` envelopes by **type**, hold a transaction until `last`, deduplicate with a
`(lsn, seq)` highwater, and cross a rotation only after its closed segment is drained. The engine
does not claim stronger delivery semantics. Reads never create state; callers choose namespaced
consumer ids and explicitly `PUT`/`DELETE` their pins.

Retention remains bounded by `ELECTRIC_CIRCUITS_CHANGES_RETAIN_SECS`: the retain window wins over
an abandoned consumer. An evicted reader is no longer listed and reads a deleted segment as `410
Gone`, which requires re-syncing from state instead of replaying history. Consumers do not delay
rotation or affect readiness. Status exposes the route, current position, consumers, and their
segment lag so an operator can diagnose retention pressure without treating it as engine health.
