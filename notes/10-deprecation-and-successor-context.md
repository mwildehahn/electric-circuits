# Deprecation and successor context

**Status checked:** 2026-08-22 (UTC). This note uses only first-party Electric
materials: repositories, their README/history, releases, and a maintainer
discussion. “ElectricSQL” is ambiguous below: it can mean the original
bidirectional/SQLite product, or the project/brand that now ships the current
read-only Electric service.

## Bottom line

The statement “electric-sql is being deprecated and Circuits is its successor”
is **not supported by the public primary record** and conflates two separate
transitions.

1. The *old* ElectricSQL implementation was superseded in 2024 by **Electric
   Next**, a clean rebuild. Electric Next was only a temporary repository; its
   work was moved back to the present, actively maintained
   [`electric-sql/electric`](https://github.com/electric-sql/electric) repo.
2. [`electric-sql/electric-circuits`](https://github.com/electric-sql/electric-circuits)
   is a separate, newer repository in the same GitHub organisation. It does
   advertise Electric wire-protocol and ElectricSQL TypeScript-client
   compatibility, but no reviewed official source calls it the successor to
   current Electric, announces a deprecation of current Electric in its favour,
   or promises a migration/support path between them.

Thus it is accurate to call Circuits an **Electric-compatible, separate
project**. Calling it the **official successor** of current Electric is an
inference, not an announced fact.

## What was actually deprecated/replaced

### The 2024 transition

The historical repository
[`electric-sql/electric-old`](https://github.com/electric-sql/electric-old)
labels itself “Archive of the Electric repo active until July 2024.”
Its announcement says that, at the start of July 2024, the team began an
experimental new approach because of problems and developer-experience pain
points in the then-current system, and that the work was temporarily in
`electric-next`. The archived README also documents the old system as
active-active Postgres-to-SQLite replication with a local database/client write
path. That is the historical implementation which was displaced.

[`electric-sql/archived-electric-next`](https://github.com/electric-sql/archived-electric-next)
calls itself the former temporary repository for the new version of Electric,
states that it is a “clean rebuild” informed by lessons from the previous
system, and says work “is now moved back to”
[`electric-sql/electric`](https://github.com/electric-sql/electric). A
maintainer’s [6 August 2024 discussion
reply](https://github.com/electric-sql/electric/discussions/704#discussioncomment-10290340)
likewise says the temporary repository was being moved back and that the old
issues, PRs, and discussions were being closed as part of that migration.

**Confirmed relationship:** Electric Next succeeded the *old* implementation,
and the repository named `electric-sql/electric` is where that new work ended
up. “Electric Next” is not a separate current product/repository to migrate to.

### Current Electric is not publicly deprecated

The current [`electric-sql/electric`](https://github.com/electric-sql/electric)
repository is public, non-archived, and the location to which the Electric Next
repository says its work moved. At the check date it published
[`@core/sync-service@1.7.12`](https://github.com/electric-sql/electric/releases/tag/%40core%2Fsync-service%401.7.12)
on 2026-08-21. Its contributor guidance explicitly distinguishes the old
bidirectional SQLite approach from “New: Electric (read-only HTTP streaming
from Postgres) + TanStack DB (optimistic writes via API)”
([source](https://github.com/electric-sql/electric/blob/main/AGENTS.md)).

This is strong evidence that the *current* Electric project remains shipped and
developed. It is not, by itself, a contractual support commitment; it does,
however, contradict a claim that the current `electric-sql/electric` project
has been announced as deprecated.

## What Circuits is confirmed to be

The [`electric-circuits` README](https://github.com/electric-sql/electric-circuits#readme)
describes a Rust engine that consumes Postgres logical replication and maintains
live query results. It has two explicitly different surfaces:

- an Electric-compatible `GET /v1/shape` wire endpoint that works with the
  **unmodified ElectricSQL TypeScript client**; and
- `@electric-circuits/client`, which adds subset queries and live aggregates.

The same README says its Electric-compatible surface is validated against
Electric’s own oracle/property/integration tests. These are meaningful
compatibility claims about a client-facing protocol and test suite. They do
**not** say that Circuits is source-, deployment-, state-, semantic-, or
operationally drop-in compatible with current Electric; the project also has a
different engine architecture and an extended API.

The repository was created on 2026-07-02 and, at this check, has no GitHub
release. Its public metadata and activity can be checked directly through the
[official repository API](https://api.github.com/repos/electric-sql/electric-circuits).
Those facts describe a young project, not a support or deprecation policy.

## Ownership, governance, and promises

Both repositories are owned by the same GitHub organisation,
[`electric-sql`](https://github.com/electric-sql):
[`electric`](https://github.com/electric-sql/electric) is Apache-2.0, while
[`electric-circuits`](https://github.com/electric-sql/electric-circuits) is
dual-licensed MIT or Apache-2.0. Shared organisation ownership establishes a
common publisher on GitHub. It does **not** establish that they have identical
maintainers, a shared release/support policy, a product succession decision, or
any corporate/governance relationship beyond that visible ownership.

No reviewed official source supplies any of the following for a move from
current Electric to Circuits:

- a deprecation, end-of-life, or support-end date for current Electric;
- a migration guide, compatibility guarantee, or data/state migration tool;
- a promise that future Electric features or packages will appear in Circuits;
- a statement that Electric Cloud is replaced by, or interoperates with,
  Circuits; or
- a formal governance/maintenance commitment tying the two repositories
  together.

This is a statement about the specified primary sources as checked on the date
above, not proof that no future announcement can be made.

## Practical implications

- Treat **old ElectricSQL / `electric-old`** as the historically superseded
  system. The relevant successor was Electric Next, now current
  **`electric-sql/electric`**.
- Treat **current `electric-sql/electric`** as an active, separately released
  upstream unless and until Electric publishes a contrary lifecycle notice.
- Treat **Circuits** as a distinct project that can be evaluated through its
  documented `/v1/shape` compatibility and its own extended client/API. Do not
  describe a move to it as an officially supported migration without an
  explicit, dated upstream commitment.
- If a product decision depends on support, migration, Electric Cloud, or
  semantic equivalence, obtain a written statement from Electric maintainers;
  protocol compatibility alone is too narrow a guarantee.
