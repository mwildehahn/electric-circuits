# tRPC OpenAPI generator spike

Date: 2026-08-23

## Goal

Check whether the official `@trpc/openapi` alpha can provide a practical OpenAPI contract for a
native Swift client without adding a second hand-written control-plane protocol.

## Reproduction

The repository currently resolves TypeScript 7.0.2:

```text
pnpm exec tsc --version
Version 7.0.2
```

Running the official generator against `apps/api/src/router.ts` with the repository TypeScript
fails before loading the router:

```text
TypeError: Cannot read properties of undefined (reading 'String')
at .../@trpc/openapi/dist/cli.js:272
```

The alpha currently relies on the TypeScript 5.x `ts.TypeFlags` shape. The same command succeeds
when the generator is given a temporary TypeScript 5.9.2 package, without changing the workspace
dependency:

```bash
pnpm --package=@trpc/openapi@11.18.0-alpha \
  --package=typescript@5.9.2 dlx trpc-openapi \
  apps/api/src/router.ts \
  -o /tmp/electric-circuits-openapi.json \
  --title "Electric Circuits API" \
  --version 0.1.0 \
  --server-url http://127.0.0.1:0
```

## Generated surface

The document is OpenAPI 3.1.1 and includes these procedures:

```text
/schema.define   POST
/ingest.write    POST
/shapes.create   POST
/shapes.get      GET
/shapes.delete   POST
/subset.query    GET
/subset.live     POST
/aggregate.create POST
```

Simple records are generated usefully. For example, `ShapeHandle` is a typed component and
`shapes.create` has typed `table`, `columns`, and `subscription` fields. The responses retain the
tRPC result envelope (`{ result: { data: ... } }`) rather than becoming ordinary REST responses.
`subset.query` is represented as one required `input` query parameter using `deepObject` style.

## Blocking limitation

The recursive predicate schema is not represented. Both `shapes.create.where` and
`subset.query.where` are emitted as an unconstrained `{}` schema. The source is the recursive
`z.lazy` predicate AST in `apps/api/src/router.ts`. A Swift client generated from this document
would therefore have no type-safe model or validation for the most important query input.

The generated paths are also procedure names (`/shapes.create`, `/subset.query`, …), and the
durable-stream protocol is not part of this document. Those are acceptable implementation details
for a temporary tRPC adapter, but they are not a stable native client contract.

There is a second integration gap: this repository's `apps/api/src/server.ts` currently installs
the ordinary `createHTTPServer` tRPC handler. Generating an OpenAPI document does not expose those
routes by itself. We would still need to mount the package's OpenAPI HTTP adapter (and decide how
it coexists with the existing tRPC handler), then add request/response E2E tests. The spike did not
change the running server.

## Decision

Do not make the generated document the sole Swift API contract yet. It is still useful for a narrow
slice:

1. Use generated OpenAPI types for simple control operations whose schemas are complete (shape
   handles, shape lookup/delete, schema/write calls where appropriate).
2. Keep the predicate AST as a handwritten, Codable Swift model until the server exposes a
   JSON-Schema/OpenAPI-friendly representation. The Swift client should send that model through a
   small transport seam rather than depend on an untyped generated dictionary.
3. Keep durable-streams handwritten; it is a separate streaming protocol and is not described by
   the tRPC OpenAPI generator.
4. Do not pin the entire workspace back to TypeScript 5.x just to run this alpha. If we retain the
   generator, run it in a dedicated, pinned tooling command and fail CI when generation changes.

Before production adoption, choose one of these concrete follow-ups:

- refactor/annotate the predicate input so the generator emits a real discriminated schema and
  add a generated-Swift compile test, plus mount and test the OpenAPI handler; or
- add a small native REST/OpenAPI facade with explicit predicate and stream-adjacent contracts,
  while leaving the internal tRPC router unchanged.

No workspace dependencies or production routes were changed by this spike.
