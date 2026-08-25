# Upstream project status — 2026-08-22

## Bottom line

**Recommendation (inference): treat the public `electric-sql/electric-circuits` project as a
research/early-adopter system, not as an unqualified production dependency.** Its public
documentation describes an unusually broad and technically serious implementation and test
story, but the public delivery and governance signals are still pre-release: a new repository,
one API-reported code contributor, no tags or releases, private `0.0.0` packages, no public
milestones/discussions, and an open issue list dominated by production-hardening work. The local
checkout contains material work beyond public `main`; that is encouraging engineering evidence,
but it cannot be counted as a released upstream guarantee until it lands publicly and is covered
by a current successful CI run.

This is a point-in-time, unauthenticated review conducted on 2026-08-22. “Verified” below means
the linked public source said it; “inference” and “recommendation” are explicitly labelled.

## Verified public position

| Area | Verified fact | What it does—and does not—establish |
| --- | --- | --- |
| Project and scope | The public repository calls itself “Electric circuits - turn any static query into a live one.” Its [README](https://github.com/electric-sql/electric-circuits/blob/main/README.md) claims a Rust logical-replication engine, Postgres as system of record, an Electric-compatible `GET /v1/shape` surface, and an extended `@electric-circuits/client` surface. | It is a public, Electric-owned experiment/product repository; a feature claim is not a compatibility or support commitment. |
| Documented API | The [live-queries guide](https://github.com/electric-sql/electric-circuits/blob/main/docs/live-queries-guide.md) scopes a live query to one table plus a predicate/projection, with nested single-column `IN`/`NOT IN` subqueries; it explicitly excludes general joins and puts ordering/windowing in subset queries. The [getting-started guide](https://github.com/electric-sql/electric-circuits/blob/main/docs/getting-started.md) documents `/v1/shape`, tRPC endpoints, and live aggregates. | The API surface is described concretely enough to trial, but no published versioned API contract or deprecation policy was found. |
| Operational claims | The [deployment guide](https://github.com/electric-sql/electric-circuits/blob/main/docs/deployment-postgres.md) and live-query guide document PostgreSQL 16, logical replication, a replication slot/publication, `REPLICA IDENTITY FULL`, and three processes in production (durable streams, engine, API). The [architecture document](https://github.com/electric-sql/electric-circuits/blob/main/docs/ARCHITECTURE.md) makes detailed durability, replication, sharing, and recovery claims. | These are useful implementation and operational prerequisites. They do not state a hosted offering, availability target, support window, or production certification. |
| Test and benchmark claims | The README names Rust tests, a TypeScript conformance suite/fuzzer, Electric’s own conformance suite, and a benchmarking-fleet runner; [CONTRIBUTING](https://github.com/electric-sql/electric-circuits/blob/main/CONTRIBUTING.md) requires engine and full conformance test runs for engine changes. | Strong intent and useful validation assets; no independently published results, release qualification, or current test badge was found. |
| License and reporting | The repo has [MIT](https://github.com/electric-sql/electric-circuits/blob/main/LICENSE-MIT) and [Apache-2.0](https://github.com/electric-sql/electric-circuits/blob/main/LICENSE-APACHE) license files; its workspace declares `MIT OR Apache-2.0`. A public [security policy](https://github.com/electric-sql/electric-circuits/blob/main/SECURITY.md) exists. | Licensing is clear. The policy is a reporting channel, not a security SLA, supported-version policy, or published vulnerability process. |

The public README/guides use confident implementation language and sometimes say “in production”
when describing how to run the three processes. **Verified:** I found no explicit “GA”, “stable”,
“beta”, “alpha”, “experimental”, or “research preview” designation in those principal public
documents. **Inference:** the absence of a maturity label should not be read as a GA declaration;
the release/governance evidence below points the other way.

## Verified repository, release, and activity evidence

- GitHub’s [repository metadata](https://api.github.com/repos/electric-sql/electric-circuits)
  records a public-repository creation time of 2026-07-02, default branch `main`, 28 stars, five
  forks, and `pushed_at` 2026-07-23T15:02:33Z. The metadata’s `open_issues_count` of 15 includes
  pull requests; the issues endpoint reported 14 open issues plus one open PR at this snapshot.
- The [`main` commit history](https://github.com/electric-sql/electric-circuits/commits/main/)
  begins on 2026-06-27 and its newest commit is
  [`b784aaf`](https://github.com/electric-sql/electric-circuits/commit/b784aaf83b4951215ef58cfdb56660c496b9cf43)
  on 2026-07-23. The public REST contributor endpoint reported one code contributor, `balegas`,
  with 62 contributions. This is the API’s contribution count, not a claim that only one person
  has ever helped.
- The [branch list](https://github.com/electric-sql/electric-circuits/branches/all) contained 30
  branches (including `main`) at review time. Its most recent branch tips were also 2026-07-23;
  names show short-lived feature/refactor work rather than a maintained release branch. This
  supports an **inference** of a concentrated late-June-to-late-July development burst followed by
  roughly 30 days without a public source push as of the review date.
- [Tags](https://github.com/electric-sql/electric-circuits/tags) and
  [releases](https://github.com/electric-sql/electric-circuits/releases) were empty. Therefore
  there is no public semantic version, changelog, release artifact, or release-notes trail to
  pin in a production deployment.
- The public [issue list](https://github.com/electric-sql/electric-circuits/issues?q=is%3Aissue%20state%3Aopen)
  has no milestones; the API returned no open or closed
  [milestones](https://api.github.com/repos/electric-sql/electric-circuits/milestones?state=all&per_page=100),
  and repository metadata says GitHub Discussions are disabled. The open issues were all filed
  on 2026-07-06 and include authentication/CORS/debug isolation, TLS, connection management,
  disk growth, cache semantics, durable catalog/restart recovery, DDL/TRUNCATE handling, large
  transactions, and failover/slot loss. See, for example,
  [#3](https://github.com/electric-sql/electric-circuits/issues/3),
  [#4](https://github.com/electric-sql/electric-circuits/issues/4),
  [#5](https://github.com/electric-sql/electric-circuits/issues/5),
  [#6](https://github.com/electric-sql/electric-circuits/issues/6),
  [#7](https://github.com/electric-sql/electric-circuits/issues/7),
  [#13](https://github.com/electric-sql/electric-circuits/issues/13),
  [#14](https://github.com/electric-sql/electric-circuits/issues/14), and
  [#15](https://github.com/electric-sql/electric-circuits/issues/15).
  **Inference:** these titles are a more candid production-risk register than the README, but
  their still-open status alone cannot prove that the code on a later, unpublished branch lacks
  a fix.
- No official Electric documentation or blog post about “Electric Circuits” was found in
  exact-site searches of `electricsql.com`, `electric-sql.com`, and `blog.electric-sql.com`.
  This is a negative search result, not proof that no such publication exists.

## CI, packaging, and artifact posture

**Verified CI.** The public [CI workflow](https://github.com/electric-sql/electric-circuits/blob/main/.github/workflows/ci.yml)
runs on pushes to `main` and PRs. It pins Rust 1.96.0, runs engine tests, installs Node 22/
pnpm, then runs `pnpm test` including conformance. The public workflow does **not** run
`cargo fmt --check` or `pnpm typecheck`. The [Docker workflow](https://github.com/electric-sql/electric-circuits/blob/main/.github/workflows/docker.yml)
builds three images and is configured to publish `main`/SHA tags to GitHub Container Registry on
`main` pushes and semver tags for `v*`; PRs only build.

**Verified CI history.** The last `main` CI and Docker runs, for commit `b784aaf`, were successful
on 2026-07-23 ([CI run](https://github.com/electric-sql/electric-circuits/actions/runs/30018669446),
[Docker run](https://github.com/electric-sql/electric-circuits/actions/runs/30018669512)). The two
newest recorded runs were `action_required` runs on a 2026-07-30 Fossabot PR
([PR #47](https://github.com/electric-sql/electric-circuits/pull/47)); they do not establish a
failure of `main`, but do mean that the public workflow history is not a continuously green
release signal.

**Verified package posture.** The root [package manifest](https://github.com/electric-sql/electric-circuits/blob/main/package.json)
and the [extended-client manifest](https://github.com/electric-sql/electric-circuits/blob/main/packages/client/package.json)
are version `0.0.0` and `private: true`. At the review time, the npm registry returned 404 for
[`@electric-circuits/client`](https://registry.npmjs.org/@electric-circuits%2Fclient) and
[`@electric-circuits/protocol`](https://registry.npmjs.org/@electric-circuits%2Fprotocol). The
[engine manifest](https://github.com/electric-sql/electric-circuits/blob/main/apps/engine/Cargo.toml)
inherits workspace version `0.0.0`; no crates.io publication claim is made here. **Inference:**
the supported consumption route is source/Docker rather than a versioned npm or crate release.
The Docker workflow demonstrates a publication *mechanism*, not that a stable, public image tag
exists or is supported.

## Comparison with this workspace

The following is deliberately not attributed to public upstream. The local checkout was on
`main` at `474577a088b95c746bd9ab2c8e4b6552a72f151f`, whereas public `main` was `b784aaf`.
These local paths are evidence of current workspace contents only:

| Public concern / missing public signal | Local comparison point | Assessment |
| --- | --- | --- |
| Open public issues #3–#7 cover stale streams, restart identity, slot loss, large transactions, and schema drift. | `apps/engine/src/engine/epoch.rs` (`SlotBinding`/`SlotBound`), `apps/engine/src/engine/drift.rs`, `apps/engine/src/changelog.rs` (`ChangesRotated`), `apps/engine/src/txn_buffer.rs`, `apps/engine/src/engine/retirement.rs`, plus `apps/engine/src/engine/lifecycle.rs::recheck_after_durability` and `::reconcile_gone_shape_stream`. | **Verified local presence only.** These paths and symbols show that the checkout has explicit designs/implementation for several open-public-issue themes. It is not verification that each issue is fixed, merged, tested, or released. |
| Public CI omits formatting and TypeScript type-checking. | `.github/workflows/ci.yml` locally adds `cargo fmt --check` and `pnpm typecheck` before the full suite. | **Verified local difference.** It improves the proposed gate, but there is no corresponding successful public upstream run yet. |
| Public docs claim robust conformance but lack a release gate. | `AGENTS.md` requires `pnpm typecheck`, `pnpm engine:test`, full Vitest conformance, Electric’s oracle (`electric-conformance/run.sh oracle`), and a live demo/browser pass; the adapter source is `apps/engine/src/electric.rs`. | **Verified local process/documentation.** No tests were run for this research note, so this is not a green-build assertion. |
| Public manifests are private `0.0.0`. | `package.json` and `packages/client/package.json` remain private `0.0.0` locally. | **Verified local continuity.** There is still no local evidence of a planned versioned package release. |

The local `AGENTS.md` also states far stronger reliability invariants than the public July 23
snapshot (for example `pg::SnapshotGate`, reliable appends, durable catalog, epoch reset,
segmented changelog, and retirement completion). Treat those as useful review targets for the
workspace—not as public contractual status—until the public default branch, test history, and
release process catch up.

## Production-readiness signals

- Detailed public architecture, deployment, API, and failure-model documentation.
- A real logical-replication implementation with Docker/demo paths, a compatibility adapter, and
  explicit Postgres prerequisites.
- Multiple validation approaches claimed: unit/integration tests, oracle/conformance, fuzzing,
  benchmark runner, and Electric-suite adapter tests.
- Public CI runs engine and full conformance tests; the last `main` runs succeeded.
- Dual MIT/Apache-2.0 licensing, a contribution guide, security reporting policy, and a configured
  container-image publishing workflow.
- Local workspace evidence of additional hardening and broader CI gates.

## Warning signals

- Public repository is only about seven weeks old and its default branch has not received a public
  source push since 2026-07-23.
- One API-reported code contributor, no release branch, no public tags/releases, and no versioned
  npm/crate packages; manifests are private `0.0.0`.
- No published compatibility/versioning policy, changelog, supported-version list, SLA, or stated
  GA/stability designation.
- Fourteen open issues, filed together as a hardening sweep, include core data-safety and security
  themes; no milestones, roadmap, or Discussions forum explains their current disposition.
- Public CI is narrower than the local documented gate, and the newest workflow attempts are
  `action_required`, not a newer successful release qualification.
- Local implementation may be materially ahead of public upstream, creating a real risk that a
  trial against the public repo/image reproduces bugs that the current workspace appears designed
  to address.
