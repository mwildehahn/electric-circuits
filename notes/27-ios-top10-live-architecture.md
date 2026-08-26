# iOS LinearLite: snapshot + live top-10 architecture

The Electric Circuits contract deliberately separates the two halves:

1. `POST /v1/subsets/query` runs one bounded Postgres query and returns `{ rows, lsn }`.
   `orderBy` and `limit` belong to this ephemeral query; the engine does not retain top-N state.
2. `POST /v1/subset-feeds` creates a changes-only feed for the base predicate. It has no ordering or
   limit and is read from its durable-stream handle.

The client must create the feed, capture its `stream-next-offset` before the snapshot query, query the
top ten (`modified DESC`, with the engine's primary-key tie-break), then start the feed from that
offset. The feed is the gap fence. For a fixed top-ten window, a row falling out must be replaced by
the eleventh row; a changes-only feed does not emit unchanged rows below the old boundary. The
correctness-first Swift demo therefore re-runs the same bounded query for each non-empty live batch,
replaces the shape-scoped GRDB page and cursor in one transaction, and only then advances the stream
cursor. This is intentionally simple and exact, not a server-side top-N circuit.

The production optimization seam is client-side window maintenance: apply per-pk LSN fences, compare
the ordered boundary, and query a refill page when a top-ten row leaves. It must preserve the same
feed-before-snapshot ordering and cannot claim exact fixed top-N semantics without a refill path.

Source references: `docs/ARCHITECTURE.md` §7, `docs/ivm-engine-internals.md` §6, and
`packages/client/src/subset.ts`.
