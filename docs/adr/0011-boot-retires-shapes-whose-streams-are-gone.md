# Catalog restore retires shapes whose streams are gone

Status: accepted (2026-09-04)

The durable catalog is the restart authority for shape records, while each `shape/*` stream is a
recreatable cache of a Postgres projection. Shape creation enqueues the catalog `Created` before it
`PUT`s the stream, and neither is awaited before the other, so a process kill can leave a durable
`Created` event for a stream that was never created. (The inverse window, a stream with rows and no
`Created`, is a create-site concern tracked separately; this ADR covers only the boot side.) This happened to `s549` (a plain
`public.calendar_event` shape) in the 2026-09-04 production incident: the catalog named it, but
durable-streams had no `shape/s549`, and every boot exited 78 before serving any other shape.

As decided in [`electriccircuits-dec-8tf`](cite:electriccircuits-dec-8tf), Resume preflight treats a
definitive `HEAD` answer of missing (404/410) or `closed: true` as a stale derived record. The
record is removed from the install set, `Dropped { id }` is enqueued as the retirement intent
(ordered ahead of `Retired` on the catalog writer, not awaited, the same stance as the existing
drift and subquery drops), and its `(id, stream_path)` enters the existing close-then-delete path,
which records `Retired` after storage accepts the idempotent deletion. A transport error while
checking a stream still aborts the restore; the boot boundary classifies it as retryable and backs
off rather than exiting. All store-level readiness and segment checks are unchanged.

## Considered options

- Keep refusing boot on a missing or closed stream: preserves all-or-nothing restore, but turns loss
  of one derived cache into an outage of every healthy shape and requires operator catalog surgery.
- Silently omit the record: lets boot continue, but leaves the stream-retirement obligation absent
  from the durable catalog and can strand an orphan stream.
- Recreate the missing stream under the old id: risks serving a newly seeded projection on stale or
  mismatched stream state and violates the id high-water invariant.
- Retire the stale record during Resume: preserves healthy shapes, makes the durable lifecycle
  explicit, and reuses the already-qualified retirement/retry machinery.

## Consequences

- The first boot with this behavior self-heals the incident shape `s549`; no catalog edit or special
  flag is required. Healthy records restore normally, while each missing/closed record is retired
  independently with no count threshold.
- Retirement remains crash-safe: `Dropped` precedes close/delete on the writer, a failed retirement
  is queued for retry or picked up by the next boot from its unmatched intent, and a kill before the
  pair lands is re-retired by the next boot's HEAD (at the cost of a second warn and count).
- A warning identifies the shape id, stream path, and reason (`missing` or `closed`). The engine
  counter `catalog_restore_retired_total` records the process total, and StatsD emits the same
  counter tagged by `reason`.
- A definitive storage answer is intentionally distinguished from an unavailable transport answer;
  an uncertain `HEAD` still prevents restore from publishing potentially stale state. Storage that
  answers 404 for a stream it still physically holds (a quarantined sidecar, for instance) is taken
  at its word; that is a storage-side contract to keep honest, not an engine threshold to add.
