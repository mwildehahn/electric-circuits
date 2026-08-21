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

## Considered options

- Refuse by default (a pathological state is refused; recovery is a deliberate human act): equally
  valid and one flag away. Auto-reset was chosen to match Electric's production behaviour and keep an
  unattended deployment self-healing, at the cost of an unscheduled backfill storm.
- Timeline/failover handling beyond the identifier check: deferred — the first deployment has one
  primary and no promotion.
