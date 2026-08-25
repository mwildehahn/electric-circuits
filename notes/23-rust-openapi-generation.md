# Rust-generated OpenAPI decision

Date: 2026-08-23

## Finding

Yes. The engine uses Axum and already has typed Serde request/response models. Rust can generate an
OpenAPI document with `utoipa` (the current implementation emits OpenAPI 3.0.3), or infer Axum
operations with `aide`.
`PredicateJson` is already a recursive Rust enum using `Box`, so it is a better source for a real
recursive predicate schema than the current TypeScript `z.lazy` analyzer.

## Architecture clarification

The desired boundary is:

```text
Swift and other native clients
          │
   Rust Axum REST adapter (public /v1 + OpenAPI)
          │
   Rust Engine implementation

TypeScript tRPC adapter (`apps/api`)
          │
  internal REST/OpenAPI client to Rust
```

The current `apps/api/src/core.ts` remains the TypeScript adapter, despite its historical
`ElectricCore` name. It exists to keep the tRPC client working and is not on the Swift path. Swift
uses the Rust Axum API directly.

Generating the public OpenAPI document from Rust is appropriate because the Rust Axum adapter is the
request handler for the public native API. tRPC remains a compatibility adapter and crosses the
Rust boundary over HTTP. Swift never goes through `apps/api`.

## Recommended ownership

Make the Rust Axum native HTTP surface the source of truth for the public REST contract:

```text
Rust Axum /v1 native routes
  ├── /v1/openapi.json   (generated from Rust types + route annotations)
  └── /v1/shapes, /v1/subsets/query, /v1/subset-feeds, /v1/aggregates

ElectricCore (TypeScript) -> calls the Rust native REST surface
tRPC adapter             -> calls ElectricCore
Swift client             -> generated REST client + handwritten durable-stream transport
```

This avoids maintaining two independent REST contracts (one in Rust and one in `apps/api`). The
existing TypeScript REST adapter can either be removed after migration or retained only as a thin
development proxy with contract tests against the Rust document.

## Crate choice

Prefer `utoipa` + `utoipa-axum` for a checked-in, compile-time OpenAPI document. It requires explicit
`ToSchema`/`IntoParams` derives and operation annotations, but makes the public contract reviewable
and deterministic. `aide` is an alternative when route-level inference is more valuable; it still
needs explicit response/error descriptions and tends to couple documentation to router assembly.

## Important limitations

OpenAPI generation will not infer correctness by itself:

- `PredicateJson` should be annotated as a recursive `oneOf` schema, and literal values should have
  an explicit JSON scalar schema rather than an unconstrained `serde_json::Value` where possible.
- `AppError` status codes and problem-detail bodies need explicit response documentation.
- Durable-streams is a separate protocol. The document describes how a client obtains a
  `ShapeResp`; it does not replace the stream reader/reconnect contract.
- Existing unversioned engine routes and `/v1/shape` compatibility routes should not be mixed into
  the native Swift contract without an explicit migration decision.

## Implementation sequence

1. Define public Rust request/response DTOs for the native operations and make their JSON names match
   `packages/protocol` and the REST adapter. **Done:** documentation DTOs are kept separate from the
   validated runtime request types.
2. Add `utoipa`/`utoipa-axum`, derive schemas, annotate the native routes, and serve or export
   `/v1/openapi.json`. **Done:** `utoipa` emits the native document from Rust and preserves the
   recursive predicate alternatives.
3. Add an E2E test that fetches the document and validates the key paths, response codes, and
   recursive predicate schema. **Done:** in-process Axum contract tests cover registration, paths,
   OpenAPI version, and recursive schema shape.
4. Point `ElectricCore` at the versioned Rust routes and remove duplicate HTTP semantics from the
   TypeScript REST adapter. **Partially done:** the TypeScript adapter now calls versioned Rust
   routes; the gateway REST facade remains as a compatibility proxy and is intentionally deferred
   for removal after native-client migration.
5. Generate Swift types from the Rust-owned document, while keeping durable-stream transport custom.
   **Next:** implement the Swift control-plane/data-plane client against this document.
