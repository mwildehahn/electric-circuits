# tRPC OpenAPI and Swift client assessment

Date: 2026-08-23

## Repository context

This repository currently uses `@trpc/server` `^11.18.0` and Zod `^4.4.3` in `apps/api`.

## Findings

tRPC now has an official `@trpc/openapi` package. It generates an OpenAPI 3.1 document from a tRPC
router, and the current tRPC documentation labels it **alpha**. It supports queries and mutations;
subscriptions are currently excluded. The generated API still follows tRPC's HTTP protocol: GET
inputs are encoded as `?input=<JSON>`, and generated clients must handle tRPC's envelope and any
configured transformer. It is therefore OpenAPI-described tRPC, not a general REST translation layer.

The old community `trpc-openapi` package is archived and targets tRPC v10/Zod 3, so it is not suitable
for this repository. `trpc-to-openapi` is a maintained third-party v11/Zod 4 fork, but it is not needed
for the initial evaluation because the official alpha now covers schema generation.

Apple's `swift-openapi-generator` can generate typed Swift clients from OpenAPI 3.0/3.1 documents and
has an iOS-compatible URLSession transport. It does not natively understand tRPC's envelope; a Swift
adapter/middleware must preserve tRPC input serialization, response envelope decoding, and error
decoding.

## Recommendation

Revise IOS-000 as follows:

1. Add `@trpc/openapi` `11.18.0-alpha` as a development dependency.
2. Generate and check the native control-plane OpenAPI document from the router.
3. Add a Swift OpenAPI-generated client for shape create/renew/delete and application writes.
4. Add a small Swift tRPC envelope adapter and test it against captured server responses.
5. Keep the durable-stream reader handwritten in `ElectricCircuitsSync`; it is a separate long-poll
   protocol and is not a tRPC subscription.
6. Reassess whether a clean REST facade is needed only after a real generated Swift call works.

The generated spec should be treated as a compatibility artifact: validate it in CI, diff it for
breaking changes, and pin the tRPC/OpenAPI generator version. Do not make the Swift app depend on
unstable TypeScript router internals at runtime.

## Decision boundary

Using the OpenAPI-generated tRPC client is appropriate for the first vertical slice if:

- the alpha generator produces accurate schemas for our recursive predicate input and shape handles;
- the Swift envelope adapter is small and deterministic;
- authentication and error statuses are represented correctly;
- the generated client can be regenerated reproducibly in CI.

If any of those fail, add explicit REST routes over the same `ElectricCore` methods. The durable
stream contract and GRDB sync design do not change either way.
