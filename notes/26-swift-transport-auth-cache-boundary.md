# Swift transport, authentication, and cache boundary

## Decision

`ElectricCircuitsSwift` does not own authentication. It follows the existing
`electric-sync-swift` integration model: the application supplies a configured HTTP client/session,
which forwards cookies and any required headers; the server or authenticated gateway owns login,
refresh, authorization, and rejection semantics.

The Swift library must therefore provide a stable injectable transport seam and document cookie
forwarding, without adding a token-provider protocol or duplicating server auth logic.

## Production client scope

The production-critical client work is the data plane and local state:

- durable-stream reader and change-envelope decoding;
- reconnect/backoff and resume from durable stream offsets/LSNs;
- cancellation and lifecycle handling;
- provider-neutral materialization and transactional cursor application, with GRDB migrations in
  the example/provider rather than the Foundation-only core;
- high-level E2E tests covering create → backfill → live change → reconnect → restart → convergence.

Before persistence, preserve numeric fidelity for PostgreSQL values rather than reducing all JSON
numbers to `Double`, and add fixture-based checks against the Rust OpenAPI/runtime responses.

The Opus review's authentication finding is consequently reclassified from a library blocker to a
transport/documentation test requirement. It remains a deployment requirement that the native API
be reachable only through the intended authenticated edge in production.
