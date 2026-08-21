# Schema drift, TRUNCATE, and replica-identity regression retire every dependent of the affected table

Status: accepted (2026-08-21)

Upstream introspects the schema once at boot: after `ALTER TABLE … ADD COLUMN` the new column is
silently absent from every envelope, `TRUNCATE` only logs while the truncated rows linger in every
shape, and a table recreated without `REPLICA IDENTITY FULL` degrades to forwarding new-only updates.
We never keep serving known-wrong data. pgoutput already delivers a fresh `Relation` message after DDL
(column names, type OIDs, replica identity); the decoder compares it with the compiled schema, and a
60 s reconciler fingerprints the catalog to catch DDL with no following DML. On drift: re-introspect
that table, swap its compiled schema, and **retire every dependent on that table** — shapes,
aggregates, subquery inner nodes, counts pipelines — by closing then deleting their streams; clients
recreate. `TRUNCATE` retires dependents (the engine holds no row copy from which to synthesise
deletes). A replica-identity regression re-asserts `REPLICA IDENTITY FULL`, then retires dependents
(updates and deletes in between were mis-applied). Granularity is per table, never whole-engine.

## Considered options

- Additive tolerance (on `ADD COLUMN`, refresh the schema and keep the streams): rejected — rows
  already in the stream never receive the new column.
- Whole-engine reset: rejected — a migration on one table would resync every table.

## Consequences

A migration that touches a synced table costs one resync of that table's shapes — the "clients resync
at cutovers" rule, enforced rather than hoped for.
