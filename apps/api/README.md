# @electric-circuits/api

The API gateway sits beside the Rust engine and durable-streams. `ElectricCore` is the
TypeScript adapter used by the gateway (the historical name is retained for compatibility); it
forwards lifecycle/query calls to the Rust engine's native Axum contract. The gateway exposes a
tRPC compatibility adapter and an optional REST/JSON compatibility adapter:

- tRPC procedures remain the surface used by `@electric-circuits/client`.
- REST resources under `/v1` on this gateway are a compatibility facade for existing integrations.
- Swift and other native clients should call the Rust engine directly, where `/v1` and
  `/v1/openapi.json` are canonical.
- Both gateway adapters call the same TypeScript adapter; neither adapter owns synchronization logic.

The underlying operations are:

- **writes** (`ingest.write`) append State-Protocol envelopes directly to the durable-streams
  `table/<name>` stream (the engine tails it; used in library mode — in Postgres mode apps write
  SQL to Postgres instead);
- **schema and shape/subset/aggregate lifecycle** are forwarded to the engine's native Axum HTTP
  (`/schema`, `/v1/shapes`, `/v1/subsets/query`, `/v1/aggregates`);
- **reads never pass through this server**: a create returns a `ShapeHandle` (`shapeId`,
  `streamPath`, `streamUrl`) and the client reads the durable stream directly.

The Electric-compatible `GET /v1/shape` endpoint is served by the **engine**, not here. Architecture:
[docs/ARCHITECTURE.md](../../docs/ARCHITECTURE.md).

## tRPC procedures (`src/router.ts`)

| Procedure | Kind | Purpose |
|---|---|---|
| `schema.define` | mutation | define the schema (tables, columns, primary keys) |
| `ingest.write` | mutation | apply one change: `{ table, op, pk, row?, txid? }` |
| `shapes.create` | mutation | register a materialized, live shape (`table`, `where?`, `columns?`) — identical creates share one stream, ref-counted |
| `shapes.get` / `shapes.delete` | query / mutation | look up / drop (decrement) a shape or feed |
| `subset.query` | query | one-shot `SELECT … ORDER BY … LIMIT/OFFSET` page + snapshot LSN (ephemeral, nothing stored) |
| `subset.live` | mutation | open a changes-only live tail feed on a base predicate (no backfill) |
| `aggregate.create` | mutation | live scalar COUNT/SUM/AVG/MIN/MAX (`fn`, optional `col`) over a filter |

The predicate input is the shared AST from [`@electric-circuits/protocol`](../../packages/protocol/README.md):
leaf comparisons, `isNull`, `and`/`or`/`not`, and `IN (SELECT …)` subqueries.

## REST compatibility resources (`src/rest.ts`)

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/v1/shapes` | create or renew a materialized shape |
| `GET` | `/v1/shapes/{id}` | look up a shape handle |
| `DELETE` | `/v1/shapes/{id}?subscription=…` | release a named subscription |
| `POST` | `/v1/subsets/query` | run a one-shot subset query |
| `POST` | `/v1/subset-feeds` | open a changes-only subset feed |
| `POST` | `/v1/aggregates` | create a live scalar aggregate |

The Rust engine publishes the canonical contract and generated document at
`GET <engineUrl>/v1/openapi.json`. The gateway facade uses ordinary JSON bodies and
problem-detail-style errors for compatibility. Durable-stream reads remain a separate client
transport using the `streamUrl` in the returned handle; they are not tRPC subscriptions.

## Starting a server

```ts
import { createApiServer } from '@electric-circuits/api'

const api = await createApiServer({
  dsUrl: 'http://127.0.0.1:8791',     // durable-streams server
  engineUrl: 'http://127.0.0.1:7010', // electric-circuits-engine control plane
  port: 8790,                         // omit for an ephemeral port
  host: '0.0.0.0',                    // default 127.0.0.1
})
console.log(api.url)
await api.close()
```

`docker/api-server.ts` is a complete standalone entrypoint (env: `DS_URL`, `ENGINE_URL`,
`API_PORT`, `BIND_HOST`) — it is what the `api` service in [docker/](../../docker/README.md) runs.
For embedding without HTTP, `createCore` (`src/core.ts`) exposes the same operations as plain
async methods.
