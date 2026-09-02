# Blocked: external change-log consumer lane

Task `electriccircuits-task-9iq` cannot establish its required working line.

The `mighty-circuitsd` path dependency currently resolves
`../../../electric-circuits/apps/engine` from
`/Users/mh/labs/mighty/mighty-next/docker/circuitsd` to the absent path
`/Users/mh/labs/mighty/electric-circuits/apps/engine`.  Its initial build therefore fails before
compiling this checkout.

More importantly, `origin/main` does not export the wrapper's required
`Engine::new_pg_with_setup` and `PostgresSetup` API.  The requested pinned-line backport
(`fix/upstream-txid-handoff`, ending at `8e77be8`) contains those symbols via its first commit,
`bde9b6f`.  Attempting that mandated cherry-pick conflicted in
`apps/engine/src/engine/mod.rs`; it was aborted without resolving the conflict.  Resolving it
would be work beyond a cherry-pick, so the lane brief requires stopping here.

Required operator decision: provide a clean, reconciled managed-source backport (and a valid
circuitsd path-dependency mapping), or explicitly authorize conflict resolution/rebase work.

