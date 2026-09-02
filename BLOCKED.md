# Blocked: circuitsd path-dependency compatibility

Task `electriccircuits-task-9iq` requires
`/Users/mh/labs/mighty/mighty-next/docker/circuitsd` to compile against this
branch before the external change-log-consumer work proceeds.

The authorized managed-source backport (`f87ec72`) is present and the
runtime source-activation guard is covered by
`apps/engine/tests/publication_rls.rs`.  The wrapper still fails to compile
with the engine's pinned Rust 1.96.0:

```text
error[E0599]: the associated function or constant `new` exists for struct
`DsClient`, but its trait bounds were not satisfied
   --> src/main.rs:636:19
636 |         DsClient::new(source_ds),
```

Current `DsClient` intentionally has no public `new`: production callers must
use the scoped, TLS-verified `DsClient::connect(DsConnectionConfig)` admission
path.  The only historical `DsClient::new` is in pre-admission commit
`bf9ed6d`; restoring it would reintroduce an unscoped production HTTP
constructor, not a permitted cherry-pick or the authorized managed-source
conflict resolution.

The Mighty wrapper needs an explicitly authorized migration to the current
Durable Streams admission/configuration API (or a separately approved,
security-preserving engine compatibility facade).  Do not begin consumer tasks
until that decision lands and the wrapper build succeeds.
