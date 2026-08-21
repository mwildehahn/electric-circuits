# Ingest stays on pgoutput protocol v1; large transactions spill receiver-side; protocol v2 is deferred

Status: accepted (2026-08-21)

Upstream issue #17 proposes pgoutput protocol v2+ (streaming of in-progress transactions) as the real
fix for unbounded memory on large transactions. We decline for now. The engine's invariant is that one
commit reaches the change log as one atomic unit in commit order, and a streamed transaction still
cannot be appended until its commit frame arrives (commit LSN unknown, aborts possible) — so the
receiver buffers or spills under v2 exactly as under v1, while v2 additionally needs a fork of the
replication client (`pgwire-replication` hardcodes `proto_version '1'` and has no streaming option),
decoding of the stream frames and xid-prefixed DML, and interleaved in-flight transactions. Postgres
already spills its reorder buffer server-side past `logical_decoding_work_mem`; only the receiver was
ever at risk.

Instead: a per-transaction in-memory byte cap; past it the transaction's envelopes spill to a local
temporary file and are streamed back at `Commit`; the single append becomes chunked appends, each well
under the durable-streams body cap; the slot is acknowledged only after the last chunk; and the
sequencer's `(lsn, seq)` de-duplication makes re-delivery after a mid-chunk crash safe. Transaction
size never invalidates a shape.

Revisit v2 when large transactions measurably delay small ones — a latency reason, not a memory one.
