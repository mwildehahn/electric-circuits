# Protocol-neutral API architecture

Date: 2026-08-23

## Decision

Electric Circuits must not be coupled to tRPC. tRPC is one TypeScript compatibility adapter, not the
product protocol and not an engine dependency. The stable native boundary is an explicit versioned
HTTP/JSON control-plane API served directly by the Rust Axum engine, with an OpenAPI document owned
by that Rust adapter. Swift and other native clients connect directly to Axum.

The data plane is separate: shape handles point to durable-streams feeds, and stream consumption is
implemented by client libraries rather than hidden inside a tRPC procedure.

## Layers

```text
Postgres / durable-streams
          |
Rust engine + native Axum `/v1` + OpenAPI
       /                       \
 Swift / other native clients       TypeScript ElectricCore adapter
       |                                  |
 durable-streams client             tRPC compatibility adapter
       |                                  |
   GRDB/materializer                 @electric-circuits/client
```

### Engine

The Rust engine owns replication, shape lifecycle, retention, catalog durability, query semantics,
and durable-stream append/replay behavior. It exposes internal HTTP endpoints and the existing
Electric-compatible adapter where required. It must not import or mention tRPC, Zod, OpenAPI, Swift,
or client-specific transport concepts.

### Rust engine / native API

The Rust engine owns the protocol-neutral application service and native REST adapter. Keep its
inputs and outputs as domain DTOs, not
`Request`, `Response`, `TRPCError`, HTTP status codes, or tRPC envelopes.

Representative operations:

```text
defineSchema(schema) -> void
createShape(def, subscription?) -> ShapeHandle
getShape(id) -> ShapeHandle | null
dropShape(id, subscription?) -> void
querySubset(def) -> SubsetResult
createSubsetFeed(def, subscription?) -> ShapeHandle
createAggregate(def, subscription?) -> ShapeHandle
```

The service maps engine and durable-stream failures into a small typed domain error set, for
example `invalidArgument`, `notFound`, `conflict`, `unavailable`, and `internal`. Adapters then map
those errors into their own wire conventions.

### Rust REST adapter

This is the stable public/native contract. Use explicit resource and action paths, for example:

```text
POST   /v1/shapes
GET    /v1/shapes/{id}
DELETE /v1/shapes/{id}?subscription=...
POST   /v1/subsets/query
POST   /v1/subset-feeds
POST   /v1/aggregates
```

Use JSON request/response bodies and RFC 9457-style problem details for errors. Publish
`/v1/openapi.json` from the Rust REST adapter. The OpenAPI document should be generated from the
Rust request/response DTOs and route annotations, not inferred from a tRPC router.

The write and schema-definition operations need an explicit deployment policy. In Postgres mode,
application writes should normally go to Postgres; library-mode ingest can remain an authenticated
internal endpoint rather than being part of the public Swift API.

### TypeScript tRPC adapter

Keep the existing tRPC procedure names and TypeScript ergonomics as a compatibility surface. The
current `apps/api/src/core.ts` is this TypeScript adapter despite its historical `ElectricCore` name.
It should call the Rust native REST/OpenAPI API, with tRPC input parsing and error mapping remaining
local to `router.ts`. tRPC can be removed or replaced without changing the Rust engine or Swift
client.

### Stream transport

Do not model a durable stream as a tRPC subscription. A `ShapeHandle` contains the stream identity
and URL; clients use a dedicated durable-streams transport with explicit reconnect, replay, cursor,
backpressure, and close semantics. The Swift package should own this transport and feed decoded
changes into GRDB.

## Shared schemas and ownership

Create a small contracts module for domain DTOs, error codes, and the predicate model. It may use
Zod at the TypeScript adapter boundary, but the service should consume plain typed values. The
contracts module should not import `@trpc/server`.

The predicate AST needs one canonical JSON representation shared by REST and tRPC. Until an
OpenAPI-friendly recursive schema is available, define the JSON shape explicitly and give Swift a
handwritten `Codable` enum. Do not accept the `{}` schema emitted by the current tRPC OpenAPI alpha
as a production contract.

## Migration sequence

1. Add versioned native REST routes to the Rust Axum router and preserve the existing unversioned
   engine routes during migration.
2. Generate and serve `/v1/openapi.json` from Rust DTOs and route annotations.
3. Point the existing `apps/api/src/core.ts` TypeScript adapter at the Rust native REST API; keep
   `router.ts` as a thin tRPC adapter.
4. Add Rust REST E2E tests for shape lifecycle, idempotent subscriptions, subset query, typed errors,
   and stream-handle delivery.
5. Generate or validate the OpenAPI document in CI, including a non-empty predicate
   schema.
6. Implement Swift control-plane calls directly against Rust REST/OpenAPI and durable-stream
   consumption separately.
7. Remove the duplicate TypeScript REST proxy once Rust REST is serving the native contract.

## Non-negotiable tests

- Calling Rust REST and tRPC with the same shape definition produces equivalent `ShapeHandle` semantics.
- Repeating named create/delete requests preserves ADR-0008 idempotency through both adapters.
- REST errors map to stable problem types; tRPC errors map to equivalent domain codes.
- A handle returned by Rust REST or tRPC can be consumed through the same durable-stream client.
- The public OpenAPI document has a real recursive/discriminated predicate schema, or the contract
  deliberately limits the public predicate surface and tests that limit.
- No protocol adapter can bypass the service and mutate engine/catalog state differently.

## Temporary TypeScript REST proxy

An initial `apps/api/src/rest.ts` proxy was added while the boundary was being clarified. It is not
the native contract and should be removed or retained only as a temporary test proxy after the Rust
Axum `/v1` routes are available.
