# Fork / upstream delta

As-of: 2026-08-22 (local analysis). This note compares the checked-out
`mwildehahn/electric-circuits` `main` with
[`electric-sql/electric-circuits` `main`](https://github.com/electric-sql/electric-circuits).
It is intentionally an ancestry and source comparison, not a test or security
assessment.

## Verified facts

### Lineage and divergence

- The checked-out local commit is
  [`474577a`](https://github.com/mwildehahn/electric-circuits/commit/474577a088b95c746bd9ab2c8e4b6552a72f151f)
  (`ci: install the rustfmt component for the pinned toolchain`). The local
  branch and `origin/main` point to that same commit.
- Upstream `main` currently points to
  [`b784aaf`](https://github.com/electric-sql/electric-circuits/commit/b784aaf83b4951215ef58cfdb56660c496b9cf43)
  (`chore: remove beads/codex agent tooling`). This is the merge-base of the
  two mains.
- `git rev-list --left-right --count upstream-main...fork-main` returned
  **`0 30`**: the fork has 30 commits after upstream and is missing **zero**
  upstream commits. The source diff is available as a
  [fork compare](https://github.com/mwildehahn/electric-circuits/compare/b784aaf83b4951215ef58cfdb56660c496b9cf43...474577a088b95c746bd9ab2c8e4b6552a72f151f).
  Therefore, merging the currently observed upstream `main` is a no-op
  (equivalently, the fork already fast-forwards from it); it cannot have a
  textual merge conflict today.
- GitHub metadata says `mwildehahn/electric-circuits` is a fork whose direct
  GitHub parent is [`pgxsinkit/electric-circuits`](https://github.com/pgxsinkit/electric-circuits),
  and whose source is upstream. The parent and this fork currently resolve to
  the identical `474577a` commit (zero commits in either direction). Thus it
  is a **second-level GitHub fork/mirror**, although its Git commit graph is a
  clean direct descendant of upstream's current head. The local remote URL was
  not recorded here so that any credentials in a remote configuration cannot
  leak.

### Change volume and churn

The exact tree diff from `b784aaf` to `474577a` is **143 files: 28,722 added,
2,957 deleted, net +25,765 lines**. It adds 55 files and modifies 88. By
changed lines, the major overlap areas are:

| Local path | Added / deleted | Why it matters |
| --- | ---: | --- |
| `apps/engine/src/engine/lifecycle.rs` | +2,064 / -248 | shape creation, sharing, retention, durable lifecycle |
| `apps/engine/src/subquery.rs` | +1,771 / -343 | membership and cross-table shape delivery |
| `apps/engine/src/engine/catalog.rs` | +1,679 / -91 | durable catalog and restart contract |
| `apps/engine/src/replication.rs` | +944 / -182 | logical-replication ingest |
| `apps/engine/src/engine/sequencer.rs` | +915 / -207 | ordered commit emission and checkpointing |
| `apps/engine/src/engine/mod.rs` | +978 / -114 | engine orchestration |
| `apps/engine/src/txn_buffer.rs` | +1,092 / -0 | new spillable transaction buffer |
| `apps/engine/src/pg.rs` | +958 / -104 | streaming backfills and snapshot fencing |
| `apps/engine/src/changelog.rs` | +818 / -0 | new segmented change log |
| `apps/engine/src/engine/drift.rs` / `epoch.rs` | +790 / -0; +767 / -0 | new schema-drift and slot-epoch handling |

The other deliberately affected contracts include native HTTP/API and client
handling (`apps/api/src/{core,router}.ts`, `apps/engine/src/http.rs`,
`packages/client/src/{index,subset}.ts`), shared envelopes/types
(`packages/protocol/src/{envelope,types,sql}.ts`), and deployment/CI toolchain
files (`docker/`, `.github/workflows/ci.yml`, `rust-toolchain.toml`,
`tsconfig.json`). The diff also adds a substantial test net, especially under
`packages/conformance/src/` and `apps/engine/tests/`.

### Fork commit families

All 30 commits are post-upstream and authored 2026-08-20 through 2026-08-22.
Their commit messages and the code above support this grouping:

1. **Pre-hardening tests and guardrails (11 commits).** Native-path
   conformance defects and their fixes, durable-streams pinning, then the fork
   ADR and upstream-issue triage. Key anchors:
   [`9819013`](https://github.com/mwildehahn/electric-circuits/commit/98190139bf9c7700e6e8569e745065b36fc91203),
   [`f314de7`](https://github.com/mwildehahn/electric-circuits/commit/f314de7a9dc6387786a6e946d6537bc02ab98fd7),
   [`2ba5ab8`](https://github.com/mwildehahn/electric-circuits/commit/2ba5ab8f05b8288e999d7072c608f302225840c2),
   [`0d336cf`](https://github.com/mwildehahn/electric-circuits/commit/0d336cf161be6863a271944529532cd5cf020bfc),
   [`7ecaa05`](https://github.com/mwildehahn/electric-circuits/commit/7ecaa05ec84a97698b2a1787f2f2a8107f95ff74).
2. **Engine durability and operations (11 commits).** Close-then-delete
   retirement; schema-qualified table identity and drift retirement; slot
   epoch reset; segmented change log; bounded/spillable transaction ingest;
   readiness/SIGTERM/streamed backfills; numeric and library-write fidelity;
   subset paging. Anchors:
   [`db1822d`](https://github.com/mwildehahn/electric-circuits/commit/db1822d14936e662760c11751eefc264cd6d8c97),
   [`e19d42e`](https://github.com/mwildehahn/electric-circuits/commit/e19d42e64bb421a36b8903b2da8bb0db0fc5a75a),
   [`f1b7fc8`](https://github.com/mwildehahn/electric-circuits/commit/f1b7fc8454140260bead8a310740bab7de0f2bef),
   [`75a8091`](https://github.com/mwildehahn/electric-circuits/commit/75a80917297319079104746b8606426163ab452e),
   [`cfed863`](https://github.com/mwildehahn/electric-circuits/commit/cfed863724160cf33a5d495d544e97d6f37a9a43),
   [`56746ca`](https://github.com/mwildehahn/electric-circuits/commit/56746cad8db4b9f9b22d14c3ddc26d0dad98eec9).
3. **Restart-safe subscriptions and client contract (8 commits).** The
   catalog becomes durable-before-ack, native creation rechecks after waits,
   subscriptions gain caller identities and leases, native removals become
   durable, and TypeScript/rustfmt CI is made explicit. Anchors:
   [`e246976`](https://github.com/mwildehahn/electric-circuits/commit/e24697675a52d31f6aedefdd3c799b5d70896c16),
   [`a8219bd`](https://github.com/mwildehahn/electric-circuits/commit/a8219bdc2098864bf7edaa9737f1ef2240a7c487),
   [`08c8fed`](https://github.com/mwildehahn/electric-circuits/commit/08c8fed2355fd6cf9a9aab10c95bb614425854eb),
   [`da005a6`](https://github.com/mwildehahn/electric-circuits/commit/da005a63485888edca6c69129f011f12b3cedce2),
   [`474577a`](https://github.com/mwildehahn/electric-circuits/commit/474577a088b95c746bd9ab2c8e4b6552a72f151f).

The implementation claims and contract decisions are documented locally in
`docs/adr/0001-fork-scope-native-path.md` through
`docs/adr/0008-subscriptions-are-identified-idempotent-and-leased.md`, with
the implementation map in `docs/ARCHITECTURE.md` and `AGENTS.md`.

### Upstream/fork issue trackers

- Upstream's [issue tracker](https://github.com/electric-sql/electric-circuits/issues)
  was available. The GitHub API returned 14 open non-PR issues —
  [#3](https://github.com/electric-sql/electric-circuits/issues/3) through
  [#8](https://github.com/electric-sql/electric-circuits/issues/8),
  [#10](https://github.com/electric-sql/electric-circuits/issues/10) through
  [#17](https://github.com/electric-sql/electric-circuits/issues/17) (with
  #9 closed); its `open_issues_count` was 15 because that field includes open
  pull requests. The fork's local triage is
  `docs/notes/2026-08-21-upstream-issue-triage.md`.
- The `mwildehahn` fork's [issue tracker](https://github.com/mwildehahn/electric-circuits/issues)
  was available but returned no issues. The direct parent
  [`pgxsinkit` tracker](https://github.com/pgxsinkit/electric-circuits/issues)
  is the active-looking one: it has open follow-ups #11–#18 and two remaining
  old subset issues (#4–#5). This reconciles the local triage note's reference
  to fork issues with the otherwise empty `mwildehahn` tracker.

### Upstream changes absent from this fork

There are **no committed changes in the current upstream `main` that are
absent from the fork**; the zero-left-side divergence is the complete evidence
for that statement. Open issues are not upstream commits, so they are listed
separately rather than mislabeled as missing changes.

The local triage/ADR scope says that several still-open upstream concerns are
deliberately not completed on the fork's native path: Electric-compatible
`/v1/shape` HTTP caching ([upstream #10](https://github.com/electric-sql/electric-circuits/issues/10)),
durable-stream CDN caching ([#11](https://github.com/electric-sql/electric-circuits/issues/11)),
Postgres TLS ([#14](https://github.com/electric-sql/electric-circuits/issues/14)),
control-plane auth/CORS/debug isolation ([#15](https://github.com/electric-sql/electric-circuits/issues/15)),
and pgoutput protocol-v2 work ([#17](https://github.com/electric-sql/electric-circuits/issues/17)).
See `docs/adr/0001-fork-scope-native-path.md` and the local triage note for
the stated scope, rather than treating these as regressions introduced by the
fork.

## Inference

- This is **not an independent rewrite**: every fork commit has the upstream
  head as an ancestor, so history preservation and future cherry-picks remain
  straightforward at the Git level.
- It is nevertheless a **substantially different implementation of the
  engine's reliability boundary**. The scale of the diff, six new core engine
  modules, and changes across ingestion, persistent cataloging, sequencing,
  lifecycle, and the client/API protocol mean it should be evaluated as a
  materially different runtime, not as a cosmetic patch set.
- The ADRs make the intended product divergence explicit: native
  `POST /shapes` plus direct durable-stream reads are the maintained surface;
  the Electric `GET /v1/shape` adapter is retained but not actively developed.
  That is a key compatibility distinction for an ElectricSQL-based migration.

## Likely future merge-conflict and migration implications

- **Current upstream merge:** no conflict/no work; the merge-base is the
  upstream tip.
- **Future upstream merge:** conflicts are most likely if upstream edits the
  fork's heavily rewritten hot paths: `apps/engine/src/engine/{lifecycle,catalog,
  sequencer,mod}.rs`, `replication.rs`, `pg.rs`, `subquery.rs`, `ds.rs`,
  `schema.rs`, the new change-log/epoch/drift machinery, or shared native
  types in `packages/protocol/src/`. API/client changes are also coupled:
  `apps/api/src/`, `apps/engine/src/http.rs`, and
  `packages/client/src/{index,subset}.ts` must be merged as contracts, not
  file-by-file.
- The new files are additive and unlikely to have literal conflicts, but they
  establish persistent formats (`LogPosition`, catalog events, subscription
  IDs, epoch/table identities). Future upstream changes touching equivalent
  concepts need semantic migration/restart review even if Git reports a clean
  merge.
- For a consumer that depends on `/v1/shape`, the fork's intentional native
  focus is a migration risk: keep the Electric conformance suite in validation
  and plan compensating caching, TLS, authentication, and adapter lifecycle
  work. A native-client migration instead needs to adopt named subscription
  renewal/release and handle terminal stream closure as described in
  `docs/adr/0008-subscriptions-are-identified-idempotent-and-leased.md` and
  parent-fork [issue #17](https://github.com/pgxsinkit/electric-circuits/issues/17).
