# Security/gateway differential review

Review date: 2026-08-22. Scope: `notes/16-production-readiness-and-swift-migration-spec.md`
against `notes/12-security-and-multitenancy.md`, `notes/05-operations-and-sre-readiness.md`,
`notes/06-circuits-wire-protocol.md`, and the current engine, API, TypeScript client, image, workflow,
and Compose implementations. This is a differential design review, not a penetration test.

## Verdict

**Do not mark the production spec reviewed or delegate its current security wave.** It has the right
outer boundary—one authenticated TLS gateway, private engine/API/DS/Postgres, server-owned query
policy—but its work packets do not yet make that boundary executable. In particular, the proposed
public protocol and Swift models still expose arbitrary predicate definitions, `SEC-004` leaves the
central proxy-versus-capability decision to an implementer, no durable principal-to-engine-claim
binding is specified, revocation has no synchronization contract, and the admin/data split is an
open-ended choice on the engine's single listener.

The first release should choose **gateway-proxied stream reads only**. Public clients should receive a
principal-bound opaque feed handle, never an engine shape id, engine subscription id, DS path, or DS
URL. The public query contract should be `template id + template version + typed allowed parameters`,
not the engine predicate AST. Direct signed DS capabilities can be designed later as a separately
gated feature.

## Verified current exposure

This is the implementation the tasks must actually transform:

| Surface | Verified behavior | Consequence |
| --- | --- | --- |
| Engine listener | One Axum router registers `/v1/shape`, `/shapes`, `/aggregate`, `/query`, shape rows/log, schema/table mutation, epoch reset, metrics reset, replication state, memory, Prometheus, and optional graph/state/trace/profile routes (`apps/engine/src/http.rs`). | Network policy cannot give the gateway data-plane access without also making admin/debug routes reachable on the same socket. |
| Electric adapter | Only `/v1/shape` checks `ELECTRIC_SECRET`; it accepts the secret in `secret` or `api_secret` query parameters. The request still supplies table, SQL `where`, parameters, and columns (`apps/engine/src/electric.rs`, `apps/engine/src/config.rs`). | Passing authenticated compatibility traffic through unchanged bypasses server-owned query templates and puts a deployment secret in a URL. |
| Native engine API | `/shapes`, `/aggregate`, and `/query` accept caller-selected tables, predicates, projections, subqueries, limits, and subscriptions with no principal middleware. Shape responses contain `streamPath` and absolute `streamUrl`. | The native wire structs are internal privileged operations, not a safe public contract. |
| tRPC API | Every procedure is `t.procedure`; `schema.define` and `ingest.write` are reachable alongside shape/subset/aggregate operations (`apps/api/src/router.ts`). Context contains only `core`. | The API cannot be relabelled as the public gateway. |
| Node core | Library-mode writes append directly to `changes/<segment>` in DS, while other methods forward to the engine (`apps/api/src/core.ts`). | A gateway identity must never inherit the API service's DS append authority. |
| TypeScript client | Lifecycle goes through tRPC, then reads the returned raw `streamUrl` or a configured `dsBaseUrl` plus `streamPath` (`packages/client/src/index.ts`, `packages/client/src/subset.ts`). | Current client architecture treats DS paths as bearer locators; it supplies no tenant authorization. |
| Engine transports | The engine creates a default `reqwest::Client`, Postgres uses `NoTls`, and the engine has one plain-HTTP listener (`apps/engine/src/ds.rs`, `apps/engine/src/pg.rs`, `apps/engine/src/main.rs`). | `SEC-006` requires real client/server transport work, not deployment documentation alone. |
| Compose/images | Compose publishes PG, DS, engine, and API; DS defaults to memory; images use mutable bases and generally run as root; the Node image runs `pnpm install` without `--frozen-lockfile`; the publish workflow omits the DS image and has no SBOM/signing gate. | The supplied artifacts are demonstrably development-only and cannot be the starting point for a production overlay. |

## Ranked findings and required spec changes

### P0.1 — The public query contract contradicts the tenant policy

`SEC-003` correctly says clients must not provide arbitrary SQL/AST, but `PROTO-001` still specifies
predicates, tables, projections, and subqueries; `SWF-002` requires a public predicate AST;
`SWF-006` exposes a shape definition; and `CMP-003` tells the compatibility provider to encode
`where`, params, and columns. Against the current handlers, implementing those tasks literally would
publish the unsafe engine contract through an authenticated proxy. Authentication would identify the
caller but would not constrain what the caller can ask the engine to replicate.

Required edits:

1. Add **`SEC-000 — Freeze the public trust and authorization contract`**, priority blocker, before
   `PROTO-001`. It must select:
   - public request = `{templateId, templateVersion, allowedParameters, idempotencyKey}`;
   - principal = immutable `{issuer, subject, securityDomain/tenant, session, roles}` derived only
     from verified authentication plus the server authorization store;
   - public feed identifier = gateway-owned opaque handle;
   - engine table/predicate/projection/subscription and DS location = internal fields;
   - authorization source, policy-version semantics, and revocation barrier;
   - proxy-only stream reads for release one; and
   - no cookie authentication/browser support unless `SEC-009` explicitly enables it.
2. Change `PROTO-001` to define separate **public gateway** and **private engine** schemas. Remove
   table refs, predicate AST, arbitrary projections, DS `HEAD`, DS offsets-as-URLs, `streamPath`, and
   `streamUrl` from the public create response. A stream resume token may remain opaque in the public
   gateway response.
3. Change `SWF-002`, `SWF-006`, `SWF-008`, and `SWF-009` to model generated/typed template calls.
   Predicate AST support may exist only in an explicitly unsafe development/internal product that
   the production package cannot initialize.
4. Change `CMP-001` to produce an exact canonical request matcher for each Electric call site.
   Change `CMP-002`/`CMP-003` so the compatibility gateway maps only those canonical requests to a
   template and constructs the internal Electric request itself. It must not forward the incoming
   `table`, `where`, `params`, or `columns` as authority.

Executable acceptance for `SEC-000`/`PROTO-001`:

- Compile every checked-in public OpenAPI operation and assert that no request/response schema has a
  field named `table`, `where`, `predicate`, `columns`, `subquery`, `streamPath`, `streamUrl`,
  `shapeId`, or engine `subscription`; an explicit reviewed exception list must be empty in
  production mode.
- For every template fixture, mutate each incoming query/header/body field, add duplicate query
  keys, reorder parameters, change encoding, add an unknown projection, and substitute a nested
  subquery. The gateway either produces the same canonical internal request or rejects it before an
  engine request is recorded; it never produces a broadened internal predicate.
- A template whose server-owned predicate is `tenant = principal.securityDomain` is exercised with
  two tenants and overlapping primary keys. Captured engine requests contain the authenticated
  tenant value, never the client-supplied value, on the outer table and every subquery leg.
- A production Swift package build has no public initializer accepting `TableRef` or predicate AST.

### P0.2 — `SEC-004` leaves an architecture fork and conflicts with G1

G1 says only the gateway is public. `SEC-004` asks an assignee to choose either proxying or signed DS
capabilities, while the explicit decisions repeat that ambiguity. A direct capability design would
require a public DS verifier or a second public proxy, DS key distribution, method/path canonicalization,
expiry, revocation, and audit—none of which the current DS provides. This is not interchangeable with
proxying and cannot fit one work packet. `PROTO-001`'s “durable stream read/HEAD” wording also risks
standardizing DS itself as the mobile API.

Required edits:

- Replace `SEC-004` with **`SEC-004 — Proxy all public feed reads at the gateway`**. The gateway owns
  `HEAD`/read/long-poll semantics at `/client/v1/feeds/{opaqueFeed}` and translates privately to an
  exact DS path. It authenticates and authorizes every request, permits only `GET` and `HEAD`, strips
  hop-by-hop headers, applies response limits, propagates cancellation, and rewrites only the
  allowlisted `stream-*` headers defined by the public protocol.
- Add **`SEC-004F — Evaluate direct read capabilities`** as a post-release, excluded task. It cannot
  enter the support matrix until DS or a dedicated verifier has path-canonical, method-bound,
  audience-bound, short-lived credentials plus revocation and audit. It does not close G1 or G2.
- Delete the capability alternative from section 17 for the first release and make direct DS
  origin reachability a production configuration error.

Executable acceptance for revised `SEC-004`:

- From the client network, a port/service scan reaches only the gateway; DNS and network-policy
  fixtures for engine, API, DS, PG, metrics, and admin listeners produce no connection.
- Given a valid token and owned feed, `GET` and `HEAD` reproduce the allowed body/status/header
  contract. `POST`, `PUT`, `PATCH`, `DELETE`, `OPTIONS` outside the gateway CORS contract, alternate
  path encodings, `..`, encoded slash/backslash/NUL, duplicate `offset`, absolute-form targets, and
  `changes/*` or `meta/catalog` substitutions cause no DS request.
- Substitute another principal, tenant, session, feed id, or a retired feed while keeping every
  other byte identical. The response is the documented non-enumerating denial and the DS request
  recorder sees zero requests.
- Hold a DS response until gateway authorization is revoked, acknowledge the revocation barrier,
  then release the DS response. The gateway emits zero body bytes and cancels the upstream request.

### P0.3 — Principal, public-handle ownership, idempotency, and revocation are unspecified

`SEC-002` says “normalized principal,” but does not define claim validation, tenant selection,
session identity, token replay assumptions, or the authorization store. The engine's subscription is
deliberately free-form and provenance-free; shape ids are predictable; equal shapes share globally.
Neither is a client capability. The spec also lacks a durable gateway ownership registry even though
its acceptance promises that a gateway restart neither duplicates claims nor loses offsets.

`SEC-003`'s revocation sentence is not enough. A JWT that was valid at poll start, a response already
being read from DS, and an engine claim that continues renewing are three separate effects. Also, no
server can make an untrusted offline phone erase data it was previously authorized to receive; the
threat model must state that residual fact instead of implying universal revocation.

Required edits:

1. Split `SEC-002`:
   - **`SEC-002A — Validate public credentials and normalize principals`**: exact issuer/audience/
     authorized-party/algorithm/key-id/expiry/not-before rules; session and security-domain lookup;
     no mobile client secret; bearer replay risk recorded; App Attest/DPoP either selected as an
     abuse signal/proof-of-possession control or explicitly not a tenant authorization boundary.
   - **`SEC-002B — Implement the durable gateway feed registry`**: transactional mapping from
     `{principal security domain, subject, session, public idempotency key, template version}` to
     `{opaque feed id, internal shape id, internal subscription, policy version, state}`. The
     gateway, not the client, generates internal subscription names. Define reconciliation after a
     crash between engine create/release and registry commit.
   - **`SEC-002C — Implement synchronized policy/session revocation`**: stop new creates and renewals,
     cancel in-flight reads before bytes are committed, stop internal renewals, release the exact
     internal claim, and record completion. A policy change that broadens/narrows a template creates
     a new generation rather than reusing an old stream offset.
2. Make `PROTO-002` public idempotency apply to the gateway idempotency key scoped to the normalized
   principal and template. Named engine subscriptions remain private.
3. Add `SEC-002B`/`SEC-002C` to G2 and as dependencies of `SWF-003`/`SWF-004` real-stack tests.

Executable acceptance:

- A token matrix covers wrong issuer, audience, authorized party, signature algorithm, key id,
  signature, expiry, not-before, tenant membership, disabled session, and an unknown security
  domain. Every rejection happens before registry, engine, DS, or PG activity.
- The same public idempotency key repeated by the same principal/template produces one registry row
  and one engine claim. Reusing it under another subject, session, tenant, or template cannot join,
  renew, read, release, enumerate, or learn whether the first feed exists.
- Inject a crash after each registry/engine create and release boundary. On restart, reconciliation
  yields exactly one live owned claim or no claim, matching the last acknowledged public result;
  there is no orphan renewal task.
- Process a deterministic `SessionRevoked` or `PolicyVersionChanged` event and wait for its explicit
  barrier acknowledgement. After the barrier, all new requests fail, a withheld in-flight page
  releases no bytes, and the internal named claim is released exactly once.
- The threat model explicitly states: data already delivered to a hostile/offline client cannot be
  cryptographically recalled; revocation bounds future server delivery and best-effort deletion by
  cooperative app caches.

### P0.4 — The gateway and engine authorization model has no durable tenant accounting boundary

`SEC-007` and `ENG-008`/`ENG-010` require per-tenant budgets, but the engine currently receives no
trusted tenant/security-domain metadata. A shape record can be shared, the catalog and `shape/*`
paths carry no tenant owner, and output-byte accounting is process-local. Multiple gateway replicas
also cannot enforce a tenant-wide limit using local counters. The draft never chooses how shared
shape cost is charged or how tenant quota state survives restart.

There is also a dependency cycle: `SEC-007` depends on `ENG-007`–`ENG-010`, while `ENG-008` and
`ENG-009` depend on `SEC-007`.

Required edits:

- Add **`SEC-007A — Specify admission identities and resource charging`**, depends on `SEC-000` and
  `GOV-002`. It assigns every budget to a concrete enforcement point and defines charging for a
  shared shape, create-in-progress, backfill, subquery node, aggregate group, long poll, response
  byte, and retained output byte. Choose either no cross-security-domain shape sharing or a tested
  multi-owner charging ledger.
- Make `ENG-007`–`ENG-010` depend on `SEC-007A`, not `SEC-007`.
- Rename the current implementation task **`SEC-007B — Enforce distributed gateway admission and
  quotas`**, depending on `SEC-007A` and the required engine accounting tasks. It must use an atomic
  shared ledger/reservation protocol across gateway replicas and reconcile reservations after
  cancellation/crash.
- Thread a trusted internal `securityDomain`/charge-owner metadata field through the private create
  contract, catalog, diagnostics, and metrics only if engine-side accounting needs it. Reject that
  header from any non-gateway workload identity and never accept it on the public API.

Executable acceptance:

- For every resource in `capacity-target.yaml`, a table names the unit, reservation point, commit
  point, release point, owner, hard limit, and error code. A schema check fails if any configured
  limit lacks one of those fields.
- Run two gateway replicas against one atomic quota store. Race `limit+1` creates behind a barrier;
  exactly `limit` are admitted and the rejected request creates no registry row, engine claim,
  stream, catalog record, PG snapshot, or dangling reservation.
- Inject cancellation/crash before and after every reservation/commit/release operation and run the
  reconciliation command. Usage equals the independently enumerated live registry/engine state.
- Drive the fixed mixed-load fixture with attacker operations at every hard limit and victim
  operations from `capacity-target.yaml`. Assert exact victim admission/error counts, queue maxima,
  and oracle convergence; do not use an elapsed “no noisy neighbor” observation.

### P0.5 — `SEC-005` does not choose or implement a privilege boundary

The existing engine exposes data, admin, mutation, metrics, and row-bearing debug routes on one
listener. `SEC-005` says “separate listener or remove them,” leaving the security decision to the
assignee. Merely putting that listener on a private network still lets a compromised gateway call
`/schema`, table writes, purge, metrics reset, or `/epoch/reset`.

Required edits:

1. Split `SEC-005` into:
   - **`SEC-005A — Split engine listeners and route sets`**: a private gateway data listener with
     only the private template-compiled create/renew/release/query operations; a probe listener with
     health/readiness; a scrape listener with read-only metrics; and an admin listener. Production
     builds/config do not register rows/log, graph/state/trace/memory/profile, schema define, or table
     mutation unless an explicit operator profile enables the appropriate admin route.
   - **`SEC-005B — Authenticate and authorize operator operations`**: distinct workload/operator
     identities and operation-specific permissions. Data-plane gateway identity cannot reset epoch,
     purge, mutate tables/schema, reset metrics, or read row-bearing diagnostics.
2. Modify `OPS-001` to create separate Services/NetworkPolicies for those listeners and to omit the
   tRPC API entirely unless a named internal consumer requires it. If deployed, the API identity
   gets only its declared private routes and never a DS credential capable of writing `changes/*`
   in Postgres production mode.
3. Make `ENG-012` validate the exact production route manifest, not only “public debug binds.”

Executable acceptance:

- Generate a machine-readable route manifest from the built router. Compare method+path+listener to
  a checked-in allowlist; any new production route fails CI until classified.
- For every engine route currently registered in `http.rs`, test the gateway workload identity,
  metrics identity, probe identity, read-only operator, and destructive operator. Assert the exact
  status and assert zero mutation for every denial.
- Start production mode with any data/admin/probe/scrape bind equal, any wildcard public bind for a
  non-edge listener, or any debug/table-write route enabled; config validation fails before binding.
- Compromise simulation: issue every method/path with a valid gateway mTLS identity. Only the
  private data-route allowlist is reachable; epoch/catalog/stream/table/metrics state is unchanged.

### P0.6 — CORS, CSRF, cache, redirects, and edge HTTP hardening have tests but no owner

`TST-004` says to attack CORS and redirects, but no implementation task defines their policy.
Native Swift ignores CORS. If the gateway uses only an `Authorization` header, CSRF can be excluded;
if it accepts ambient cookies, it needs a CSRF design. The draft also lacks gateway requirements for
redirects, tenant-data caching, forwarded-host trust, header/body/decompression limits, and
unauthenticated connection/rate limits. The current engine's OPTIONS response advertises methods but
does not define an origin policy.

Add **`SEC-009 — Harden the public HTTP origin`**, depends on `SEC-000`/`SEC-002A`:

- choose bearer-header-only public auth and reject auth cookies for release one, or specify an
  origin-bound SameSite+anti-CSRF token design;
- define the exact allowed origins (empty if browser clients are unsupported), methods, headers,
  credentials policy, exposed protocol headers, `Vary: Origin`, and preflight cache behavior;
- set `Cache-Control: no-store, private` on every data/error response and prohibit CDN/shared-cache
  storage;
- reject cross-origin/scheme/host redirects in the gateway and Swift transport;
- canonicalize trusted proxy headers and host, and bound request line/header count+bytes, JSON
  bytes/depth/nodes, compressed expansion, concurrent unauthenticated requests, and response bytes
  before allocation.

Executable acceptance:

- Run a table of allowed/disallowed/missing/`null` origins, simple requests, preflights, credentialed
  preflights, and cookie-bearing requests. Compare the complete CORS/cache header set byte-for-byte;
  disallowed cases generate no registry/engine/DS call.
- A redirect corpus covers 301/302/303/307/308 to same origin, alternate host, IP literal, HTTP,
  credentials-in-URL, and an internal service name. The Swift and gateway clients follow none of
  them for authenticated/data requests.
- Boundary fixtures for every byte/count/depth/compression limit exercise `limit-1`, `limit`, and
  `limit+1`; rejected requests allocate no engine/PG/DS work.
- A recording cache/proxy receives identical URLs for two tenants and cannot reuse either response;
  all data and error responses contain the specified no-store/private headers.

### P0.7 — TLS implementation and secret rotation are bundled, incomplete, and not testable as written

`SEC-006` combines public TLS, four internal transport paths, workload identity, Rust DS TLS support,
and rotation of IdP keys, DB credentials, DS identity, and certificates. The deliverable only names
the DS Rust client even though Postgres is hard-coded to `NoTls` and the engine listener is HTTP.
“Rotation succeeds with in-flight long polls” and “old credentials fail after the documented overlap”
do not define a reload mechanism or a deterministic completion point.

Required edits:

1. Replace `SEC-006` with:
   - **`SEC-006A — Implement authenticated transport on every production edge`**: public gateway
     HTTPS with normal platform trust; explicit TLS-capable PG connector with CA/hostname/client
     identity policy; DS `reqwest::ClientBuilder` CA/hostname/client identity policy; TLS or a
     verified mesh for every selected internal listener; no plaintext fallback; and an exact service
     graph (do not require gateway→API if API is absent).
   - **`SEC-006B — Implement secret/key inventory, reload, rotation, and revocation`**: inventory
     owner/storage/distribution/rotation for OIDC JWKS, gateway registry keys, DB credentials, DS/
     service identities, TLS keys, audit signing keys, backup keys, and image-signing trust roots.
     Specify dual-version overlap and an explicit activation/revocation barrier. Secrets must not be
     command arguments, image layers, ConfigMaps, crash dumps, or sanitized config/log output.
2. Make `OPS-003` depend on `SEC-006A`; make `OPS-007`'s certificate/credential runbooks depend on
   `SEC-006B`.

Executable acceptance:

- For each hop, test trusted CA+hostname+client identity success, then independently substitute an
  untrusted CA, wrong hostname, expired/not-yet-valid certificate, missing/wrong client identity,
  plaintext endpoint, and TLS-stripping redirect. The process fails closed with a stable preflight
  or readiness reason and sends no application data.
- With an injected credential/key provider, pause a long poll, install version N+1, open a new
  connection under N+1, release the old poll, acknowledge the activation barrier, revoke N, and
  assert N is rejected while N+1 works. This uses controlled operations/virtual validity, not a
  wall-clock observation.
- Restart every service after rotation using only the secret manager/materialized identity and prove
  catalog/shape continuity. Scan process args, environment diagnostics, logs, traces, artifacts, and
  images for all seeded secret canaries; find zero.

### P1.1 — Database/publication least privilege is not strong enough for a shared tenant source

`SEC-003` relies on gateway templates, while `OPS-003` still permits the current broad engine model.
Today the engine creates `FOR ALL TABLES`, defaults to `public.*`, changes replica identity, and uses
one Postgres credential for replication and query-backs. A gateway bug or engine credential
compromise therefore has a larger data set available than the approved sync catalog. Postgres RLS
does not automatically protect replication/query-backs performed as a shared engine role.

Required edits:

- Extend `OPS-003` to require an explicit relation and column data-classification manifest, an
  explicit publication (never `FOR ALL TABLES`), `ELECTRIC_CIRCUITS_PG_TABLES` with no wildcard in
  production, and separate bootstrap/DDL, replication, and query-back credentials. The runtime
  engine must not own application tables or have migration privileges.
- Extend `SEC-003` so each template is checked against that manifest and so every membership/
  subquery relation is reviewed. State plainly that RLS is not the tenant enforcement boundary
  unless a separate ADR proves request-scoped roles across backfill, live replication, query-back,
  and circuit paths.
- Add **`SEC-003B — Verify sync data minimization`** to detect new tables/columns and sensitive data
  before publication/template rollout.

Executable acceptance:

- Bootstrap a database containing approved sync columns plus canary secret/payment/audit columns
  and an unapproved table. Using each runtime credential, enumerate `SELECT`, DDL, replication,
  publication, slot, and replica-identity operations. Only the checked-in privilege matrix succeeds.
- Compare `pg_publication_tables`/`pg_publication_columns` and engine selectors to the reviewed
  manifest; any extra/missing relation or column fails preflight before the engine listener serves.
- Add an unreviewed table and column, restart/reconcile, and prove neither enters WAL consumed by the
  engine nor any template/shape response. The release workflow blocks until the manifest changes
  through review.

### P1.2 — Audit and privacy requirements are scattered and omit access, retention, integrity, and deletion

`SEC-005` audits only destructive calls. `OPS-006` promises redacted operational logs. `TST-004`
checks for leakage. No task owns a security audit schema covering authentication, authorization,
template decisions, feed grants/revocations, reads, quotas, or admin/data mutation; nor does one
define audit access, tamper evidence, retention, subject identifiers, IP handling, backup deletion,
or incident export. `SEC-001` classifies data but does not implement its lifecycle.

Add **`SEC-010 — Implement security audit and privacy controls`**, depends on `SEC-000`,
`SEC-002C`, and `SEC-005B`:

- define a versioned audit event schema with pseudonymous actor/security-domain/session aliases,
  template/version, opaque feed alias, decision/reason, operation, target class, request id, outcome,
  policy version, and integrity sequence/signature; never include credentials, DS locations, raw
  predicates/parameters, row values, or raw tenant/user identifiers;
- emit events for auth success/failure, authorization allow/deny, create/renew/read/release,
  revocation completion, quota reservation/denial, admin/debug access, schema/publication changes,
  and destructive operations;
- define separate security-audit versus operational-log access, retention, export, backup, erasure/
  legal-hold handling, and alert ownership; and
- extend the data map to mobile SQLite/WAL/SHM, DS/catalog/change WAL, spill files, PG WAL, metrics,
  traces, backups, CI failure artifacts, and support exports.

Executable acceptance:

- A fixture invokes every allow/deny/error branch and validates exactly one required audit event (or
  the documented pair for start/completion) against JSON Schema. A seeded canary for every forbidden
  field appears nowhere in audit/log/trace/metric/CI artifacts.
- Delete or expire one synthetic subject/security domain using the executable retention tool and
  inspect primary audit storage, searchable replicas, backups/index manifests, gateway registry,
  DS retention inventory, and mobile test cache. Results match the declared retention/legal-hold
  matrix; immutable audit entries use only the approved pseudonym.
- Mutate, delete, reorder, and replay audit records in a copied store. Integrity verification fails
  with the first bad sequence and does not silently accept the modified log.

### P1.3 — Supply-chain and mobile data protection are one non-delegable task

`SEC-008` mixes container runtime hardening, base images, SBOM, scanning, signing, provenance,
dependency policy across Cargo/npm/SwiftPM, Keychain, iOS file protection, logout, tenant switch,
backup/restore, and device lock. It depends on `SWF-012`, yet it is listed as part of G1 and not
scheduled until wave 5. This cannot be owned or accepted as one PR. Current evidence also needs more
than “signed/unscanned”: mutable GitHub Action tags and base tags, a non-frozen Node image install,
and the unpublished DS image all affect provenance.

Required edits:

1. Replace server portions with:
   - **`SEC-008A — Produce and verify server artifacts`**: pin base images and CI actions by digest/
     commit; frozen/locked dependency builds; publish engine, gateway, API-if-used, and DS images;
     per-image SPDX/CycloneDX SBOM; source+builder+dependency provenance; keyless or managed signing;
     deploy-time digest/signature/provenance verification; non-root/read-only/capability/seccomp
     runtime policy.
   - **`SEC-008B — Enforce dependency and vulnerability governance`**: Cargo/npm/SwiftPM/OS/base
     scanning, license policy, secret scan, update automation, severity threshold, and a versioned,
     owner+expiry-bound exception file. A scan performed but ignored does not close the task.
2. Move all mobile credential/cache requirements into expanded `SWF-012`; make `SEC-008A/B`
   independent of Swift so G1 can close before client qualification.
3. Add `SEC-008A/B` to `GOV-004`/`OPS-009` release evidence inputs and move them to wave 1/2.

Executable acceptance:

- Build every production image twice from clean checkouts with network disabled after dependency
  fetch; compare declared reproducibility outputs, SBOM package sets, source SHA, and provenance.
- Verify admission rejects: mutable tag without digest, unsigned image, wrong signer/identity,
  provenance for another source SHA, missing DS/gateway SBOM, root user, writable root filesystem,
  added Linux capability, and an expired vulnerability exception. The qualified images pass the
  same policy.
- Insert a known test secret, disallowed license fixture, vulnerable test package, unlocked
  dependency change, and unpinned CI action in isolated fixture branches; each independently blocks
  release qualification.

### P1.4 — Swift credential/cache protection needs exact platform choices

`SEC-008` and `SWF-012` say Keychain/file protection generally but do not choose accessibility,
backup, access-group, background, or database sidecar behavior. “Device-lock test” cannot be
implemented until background refresh requirements decide whether credentials must be accessible
after first unlock. TLS “trust policy from the app” also risks each adopter inventing pinning or an
ATS exception.

Required edits to `SWF-010`/`SWF-012`:

- choose Keychain accessibility explicitly. For a client that must renew/apply while backgrounded,
  use a `ThisDeviceOnly` class compatible with the declared background window; otherwise prefer a
  when-unlocked `ThisDeviceOnly` class. Do not put bearer/refresh credentials in UserDefaults,
  URL/cache metadata, app-group storage, or synchronizable Keychain unless separately required;
- define protection/backup policy for the cache database and its WAL/SHM/temp files, atomic
  tenant-generation keying, crash-safe purge, and extension/app-group access;
- use normal URLSession/ATS system trust by default, no reachability preflight, and no certificate
  pinning unless an operational pin rotation/fallback task is accepted; and
- define redirect rejection, background cancellation, and credential refresh ordering at the actor
  boundary.

Executable acceptance:

- Seed distinct access/refresh/cache canaries, exercise install, lock, allowed background phase,
  logout, tenant switch, uninstall/reinstall, encrypted device backup/restore, and app-group access.
  Inspect Keychain attributes and the DB/WAL/SHM/temp/backup outputs; each canary appears only where
  the selected matrix permits it.
- A deterministic actor test pauses requests around refresh/logout/tenant-switch. A response from an
  old principal/generation cannot write the new cache, and all old tail/renew tasks are awaited or
  invalidated before the new principal becomes visible.
- ATS/config inspection finds no arbitrary-load, cleartext, or per-domain exception in release
  artifacts; the URLSession redirect delegate rejects the complete `SEC-009` redirect corpus.

### P1.5 — Protocol/leadership dependencies contain hard cycles

These cycles prevent honest delegation and should be fixed before wave assignment:

- `PROTO-003` depends on `ENG-002`, while `ENG-002` depends on `PROTO-003`.
- `SEC-007` depends on `ENG-007`–`ENG-010`, while `ENG-008` and `ENG-009` depend on `SEC-007`.
- `OPS-004` depends on `ENG-013`, while `ENG-013` depends on `OPS-004`.

Required edits:

- Split **`PROTO-003A`** (transaction/framing contract and fixtures, depends on `PROTO-001`) from
  **`PROTO-003B`** (implementation conformance, depends on `ENG-002`); make `ENG-002` depend on
  `PROTO-003A`.
- Use the `SEC-007A`/engine/`SEC-007B` ordering specified in P0.4.
- Add **`OPS-004A`** (failover/fence contract and cut-point harness, depends on OPS-001–003), make
  `ENG-013` depend on it, and make **`OPS-004B`** (promotion implementation/drill) depend on
  `ENG-013`.

Executable acceptance:

- A script parses every `Depends on:` field, emits a task DAG, and fails on a cycle, unknown task id,
  or a wave that schedules a task before its dependencies. Check it into `TST-001`.

## Work packets that must be split before delegation

The following current packets cross too many trust boundaries or repositories for one primary
subagent/PR:

| Current task | Why it is not delegable | Required split |
| --- | --- | --- |
| `SEC-002` | IdP validation, gateway API, logging, network exposure, refresh, and crash-safe lifecycle are independent failure domains. | `SEC-002A` credential validation; `SEC-002B` durable feed registry; `SEC-002C` revocation/reconciliation. |
| `SEC-004` | Proxy and public signed-capability architectures have different products and dependencies. | Proxy-only `SEC-004`; future excluded `SEC-004F`. |
| `SEC-005` | Engine router/listener surgery, operator IdP/RBAC, deployment policy, and audit cannot share one acceptance boundary. | `SEC-005A` route/listener split; `SEC-005B` operator auth; audit in `SEC-010`. |
| `SEC-006` | Edge TLS, four internal transports, workload identity, and every secret's rotation are several implementations. | `SEC-006A` transport; `SEC-006B` secret/key lifecycle. |
| `SEC-007` | Distributed gateway admission and engine queue/disk accounting are mutually dependent and presently cyclic. | `SEC-007A` accounting contract; engine resource tasks; `SEC-007B` enforcement/reconciliation. |
| `SEC-008` | Container supply chain, four package ecosystems, and iOS storage/device behavior have unrelated tooling/owners. | `SEC-008A` server artifacts; `SEC-008B` dependency policy; mobile work in `SWF-012`. |
| `TST-004` | Auth, query policy, DS proxy, admin isolation, quotas, redaction, IaC, containers, and manual review would produce one opaque mega-suite. | Keep `TST-004` as an aggregator; give each SEC task its own negative suite and make `TST-004` run/verify their manifests plus a small cross-system scenario. |

## Gate and dependency corrections

Apply these exact gate changes after adding the tasks above:

- G1: `SEC-000`, `SEC-002A/B`, `SEC-004`, `SEC-005A/B`, `SEC-006A`, `SEC-008A/B`, `SEC-009`,
  `OPS-001`, and `ENG-012`.
- G2: `SEC-002B/C`, `SEC-003/003B`, `SEC-004`, `SEC-007A/B`, `SEC-010`, and the corresponding
  `TST-004` manifest.
- G5: public gateway contract from revised `PROTO-001/002/004`, plus `PROTO-003A/B` only when
  transaction-atomic delivery is claimed.
- G7: add revised `SWF-012` explicitly to both compatibility and native Swift release evidence when
  those clients persist credentials or tenant data.
- G8: add `SEC-006B`, `SEC-008A/B`, `SEC-010`, and `OPS-004A/B`.

Change the dependency front of section 4 to:

```text
GOV-001/002
  -> SEC-000
      -> PROTO-001/002
      -> SEC-002A -> SEC-002B -> SEC-002C
      -> SEC-003/003B
      -> SEC-004
      -> SEC-005A/B
      -> SEC-006A/B
      -> SEC-007A -> ENG-007..010 -> SEC-007B
      -> SEC-009/010
  -> protected staging gateway
  -> compatibility/native client integration
```

`OPS-001` must depend on `SEC-000`, `SEC-005A`, and the service graph from `SEC-006A`; otherwise the
platform agent can faithfully build manifests for the wrong listener and identity topology.

## Minimum go/no-go security scenario

Add this fixed scenario to the `TST-004` aggregator. It is deliberately operation-count based:

1. Deploy two gateway replicas, one engine, private DS/PG, separate probe/scrape/admin listeners,
   two tenants, two users per tenant, one revoked session, two roles, and at least one outer+subquery
   membership template plus one aggregate.
2. Execute the checked-in request corpus containing every auth token mutation, template field
   mutation, public handle substitution, stream path encoding, HTTP verb, CORS case, admin route,
   quota boundary, redirect, and secret canary.
3. At barriers during create, renew, long-poll response, release, policy change, credential rotation,
   gateway crash, and engine retirement, inject the event/failure and reconcile.
4. Compare all authorized materialized states with tenant-scoped SQL oracles; compare registry rows,
   engine claims, DS requests, audit events, quota reservations, and network connections with their
   independent expected manifests.

Acceptance is exact: zero unauthorized response bytes or metadata, zero unauthorized downstream
calls/mutations, zero leaked canaries, exactly the acknowledged owned claims, no unresolved quota
reservations, and equality with every authorized tenant oracle. No calendar soak or “observe the
logs” criterion closes this scenario.

