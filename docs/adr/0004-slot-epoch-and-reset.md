# The replication slot is bound to a catalog epoch; losing it resets every shape, automatically by default

Status: accepted (2026-08-21)

Upstream creates the slot at boot if missing and never re-checks it, so a slot lost to a restore,
`max_slot_wal_keep_size`, a major upgrade, or an operator is silently recreated at the current head and
every shape misses the gap with no signal. We record a `SlotBound { system_identifier, timeline_id,
slot }` event in the durable catalog when the slot is first created, and on every (re)connect — not
only at boot — verify that the slot exists, is not `lost`, and the system identifier matches. No
`SlotBound` event means a genuine first boot. Any mismatch is an **epoch break**: the only correct
recovery is a new slot and a full resync of every shape.

The default policy is **auto-reset** (Electric parity): retire every shape stream (close, then
delete), recreate the slot, record a new `SlotBound`, and start the new epoch; clients re-subscribe and
rebuild. `ELECTRIC_CIRCUITS_RESET_ON_SLOT_LOSS=false` selects **refuse** instead: ingest stops, shape
reads degrade fail-closed with a named reason, and the reset is an explicit operator action.
Reconnects use exponential backoff with jitter.

## Consequences

`ELECTRIC_CIRCUITS_RESET_ON_SLOT_LOSS` (default `true`) selects the policy; `false` makes a break a
named degraded state (`slot_lost` / `slot_wal_lost` / `system_identifier_mismatch`) in which ingest
stops, `/v1/health` reports `degraded` and every shape route answers 503, and `POST /epoch/reset` on
the control plane is the operator's recovery — it runs exactly the same reset the auto policy runs
and resumes ingest (409 when the epoch is not broken; the operation is destructive and would have
nothing to fix). `GET /replication/lsn` grows an `epoch` object — `state` (`ok`/`broken`), `reason`,
`systemIdentifier`, `timelineId`, `slot`, `boundAt` — which is where both policies are observed, and
`epoch_breaks_total` / `epoch_resets_total` count them. A reset while counts pipelines are running
exits the process (75) for the same reason schema drift does: the circuit is seeded once at boot and
cannot be rebuilt across the gap. A durable catalog that cannot be read at boot is boot-fatal — an
unreadable log is not a log without a binding, and deciding either way from it is how a slot gets
created at the WAL head beside shapes that were never dropped.

## Considered options

- Refuse by default (a pathological state is refused; recovery is a deliberate human act): equally
  valid and one flag away. Auto-reset was chosen to match Electric's production behaviour and keep an
  unattended deployment self-healing, at the cost of an unscheduled backfill storm.
- Timeline/failover handling beyond the identifier check: deferred — the first deployment has one
  primary and no promotion.
