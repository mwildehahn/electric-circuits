# Subscriptions are identified, idempotent, and leased

Status: accepted (2026-08-22)

A shape's subscribers were an anonymous refcount: `POST /shapes` incremented, `DELETE /shapes/{id}`
decremented, and the catalog recorded `Joined`/`Left` as counter arithmetic. That cannot survive an
ambiguous outcome — a storage response lost after the append committed made the writer's retry count
a join twice; an HTTP response lost after the engine committed made the client's retry release twice
(deleting the shape under another live subscriber); a create whose response was lost left a
subscriber nobody could ever release; and because releases were not idempotent they could not safely
be made durable-before-ack, so a crash could resurrect an acknowledged purge or forget an acknowledged
release. We decided that a **subscription is a first-class identity**: the caller names it (a
client-chosen `subscription` id on `POST /shapes`, echoed in the response and carried by
`Created`/`Joined`/`Left`), repeating a create or a release with the same id is a no-op rather than a
second count, every catalog event carries an event id the fold de-duplicates, and the records that
**create** something a client is promised — `Created`, and the `Joined` of a new claim — are durable
before they are acknowledged. The removals (`Left`, `Dropped`, and a renewal's `Joined`) stay
answered-at-once: making them wait would leave a caller unable to release or purge anything at all
while storage is down — the purge that would free a shape a create is parked on included — and the
lease below reconverges a record that never landed, within one idle window, which a wait cannot
improve on. A subscription is also a **lease**: it counts as live only if it was created
or renewed (the same `POST` with the same id) within the shape idle window
(`ELECTRIC_CIRCUITS_SHAPE_IDLE_SECS`), because on the native path reads go straight to
durable-streams and the engine cannot see them — the refcount is the only liveness signal it has, and
an unrenewed one is not a signal. Server-minted ids (no client idempotency on create) and refcounts
without leases were considered and rejected: the first leaves the lost-create phantom unrecoverable,
the second lets any dead client pin a shape forever.

## Consequences

- The native API gains one optional field and one query parameter: `POST /shapes { …, subscription }`
  (omitted → the engine mints one and returns it; the caller then has no idempotency on that create)
  and `DELETE /shapes/{id}?subscription=…` (omitted → the legacy anonymous decrement, kept only for
  callers that never learned their id; it is not retry-safe and the docs say so). The response carries
  `subscription`. Consumers (pgxsinkit's `engine-client.ts`, the published client) send the id on
  every create/release and renew on their own cadence — pgxsinkit on its 5-minute token re-mint.
- A subscription that lapses (not renewed within the idle window) is released by the retention
  sweeper exactly as an explicit `Left` would be; a shape whose last live subscription lapsed goes
  dormant and follows the ordinary lifecycle. A still-interested client that renews late simply
  re-subscribes (it may find the shape dormant or evicted and get a fresh one — the ADR-0007
  contract).
- Catalog events carry an `eid`; the fold ignores an `eid` it has already applied, so the writer's
  retry-in-place (no event is ever dropped) can never double-apply anything — joins, leaves, drops,
  rotations, segment deletions alike. A catalog written before this ADR is boot-fatal, like the
  pre-ADR-0002 and pre-ADR-0006 formats (greenfield; nothing shipped).
- A join that is abandoned mid-wait (client disconnect while storage is unavailable) compensates with
  a `Left` for its own subscription id — idempotent, so it cannot steal anyone else's claim.
- What this does not do: it does not observe reads. Liveness is the caller's renewal, full stop; a
  client that holds a handle and never renews is, after the idle window, a client that left.
- **Removals are answered before their record is durable** (`Left`, `Dropped`). The loss window is
  closed from the other side instead: the restore brings a subscription back with its **lease age**,
  so a `Left` that never landed is re-applied by the sweeper within one idle window, and a `Dropped`
  that never landed leaves a shape whose (stale) subscriptions lapse the same way — with a repeat
  purge a no-op. What is genuinely given up is the *immediacy* of a purge across a crash: a shape
  purged while the catalog was unavailable is back for at most one idle window after a restart,
  before the leases reclaim it.
- **A lease RENEWAL's `Joined` is queued, not awaited.** Durable-before-ack exists for a record whose
  loss would make an acknowledged subscription vanish; a renewal's claim is already in the log, and
  making every renewal a storage round trip would put a client's heartbeat on the critical path of
  an outage it has no part in. (A `/v1/shape` handle's renewal is not even written: the poll renews
  it in memory, because an Electric handle does not survive a restart.)
- **The engine mints an id for a create that names none** (as decided) — and marks it, with a `~`
  prefix a caller may not invent. The legacy anonymous `DELETE` releases a minted claim before a
  named one, so a caller that never learned the protocol cannot take the claim of one that did. The
  mark is not a wall: a `~` id the engine currently holds is the caller's own claim, and renewing or
  releasing with it is ordinary — only an UNKNOWN `~` id is refused (400), because that one could
  only have been forged.
- **The lease boundary belongs to the client**: the clock is wall-clock seconds, so a claim lapses
  once its age is strictly greater than the window. Lapsing at `>=` would end a one-second window
  after a millisecond.
