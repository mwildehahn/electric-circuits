# Engine-initiated retirement of a shape stream closes it before deleting it

Status: accepted (2026-08-21)

When the engine retires a `shape/*` stream — eviction, purge, schema drift, epoch reset — it first
appends with `Stream-Closed: true`, so a tailing long-poll is released immediately with
`stream-closed` and "closed" unambiguously means "the engine retired this shape; re-subscribe", and
only then deletes the stream. Closing is terminal, so it is never applied to a dormant shape (its
retained stream must stay appendable for reactivation) or on engine shutdown (restored shapes continue
their streams). Clients **must** treat `stream-closed`, 404 and 410 alike: re-subscribe. A final control
envelope naming the reason was considered and rejected — the client would make no different decision.
