# Fork ownership and upstream posture

Status: accepted
Date: 2026-08-23
Decision ID: GOV-001

## Decision

Circuits is an independently owned fork/project. The owning team accepts
indefinite responsibility for the Circuits source, releases, security response,
and user support. A release does not depend on adoption, review, or a merge by
either `electric-sql/electric-circuits` or `electric-sql/electric`.

This is the **self-owned-fork** branch of GOV-001. If the named ownership roles
cannot be staffed, the release decision is **no-ship** until they are restored;
there is no intermediate “wait for upstream” state.

## What this project is (and is not)

The old bidirectional ElectricSQL implementation was historically replaced by
Electric Next, whose work moved back into the current `electric-sql/electric`
repository. Current Electric remains a separately released upstream project.
Circuits is a separate Electric-compatible project with its own native engine,
gateway, durable-stream, and client surfaces. Protocol/client compatibility is
not a claim of shared implementation, state, deployment, support, roadmap, or
official succession.

No reviewed dated primary source designates Circuits as the official successor
to current Electric or announces a migration/support commitment between them.
The repository README and migration specification therefore describe Circuits
as a separate, independently owned project; they must not be changed to imply
official succession unless a dated primary-source announcement is cited.

## Ownership and decision rights

The following role assignments are part of the release-control contract. A
release profile must bind accountable people to these roles before production
traffic is enabled; the role names remain stable if personnel change.

| Responsibility | Accountable role | Decision rights |
| --- | --- | --- |
| Product/fork owner | Circuits Product Owner | Owns scope, support promises, and the ship/no-ship decision. |
| Release owner | Circuits Release Maintainer | Approves versioning, release artifacts, rollback, and publication of release evidence. |
| Security owner | Circuits Security Maintainer | Triage authority for vulnerabilities, embargoes, advisories, dependency/license exceptions, and emergency fixes. |
| Support owner | Circuits Support/SRE Maintainer | Owns supported-topology statements, incident response, operator runbooks, and customer-facing support disposition. |
| Protocol/compatibility reviewer | Circuits Protocol Maintainer | Reviews wire/client compatibility changes and upstream compatibility claims. |

No single task agent may approve its own security exception or release. The
Release Maintainer records approvals and the Security Maintainer may block a
release for an unresolved critical security issue.

## Upstream intake and merge policy

Upstream is a source of information and optional contribution, not an
execution dependency.

1. The Support/SRE Maintainer or Protocol Maintainer records each relevant
   upstream issue, release, advisory, or breaking change in the local backlog
   with its canonical URL, observed date, affected Circuits surface, and a
   disposition: `fixed_with_test`, a named local task, `product_exclusion`, or
   `upstream_only`.
2. The Release Maintainer reviews upstream changes on each planned release and
   before emergency releases. A rebase, cherry-pick, or upstream PR is accepted
   only after local tests, persistence/protocol compatibility review, and
   release evidence are rerun. Git ancestry or a clean textual merge is not
   evidence of semantic compatibility.
3. Commits offered upstream remain independently reviewable and may be
   upstream-shaped, but Circuits can release its own commit, tag, and image
   without waiting for upstream response. Rejected, unanswered, or delayed
   upstream PRs do not block a local decision.
4. Changes that alter Circuits-only native behavior are reviewed and released
   locally even when no upstream issue exists. Changes to the Electric adapter
   retain the separate compatibility review and oracle gates.

## Security, dependency, and license intake

The Security Maintainer owns a private embargo channel and a public security
advisory process. Reports from upstream, package registries, OS/base images,
users, or automated scanners enter a local security record with affected
component/version, severity, exploitability, disclosure deadline, owner,
mitigation, and fixed-version evidence. Critical/high findings receive an
emergency release decision; lower-severity findings enter the next planned
release according to the security SLA selected by the release profile.

Cargo, npm/pnpm, Swift, container, and operating-system dependency updates are
reviewed for vulnerability, license, provenance, and reproducibility impact.
The Security Maintainer approves license exceptions and records their expiry;
the Release Maintainer blocks an artifact whose SBOM, lockfile, scanner result,
or exception record is missing. `SEC-008B` owns the executable scan/update gate;
this ADR establishes who owns the decision and where upstream advisories enter
that gate.

## If upstream never adopts the fork

The self-owned decision remains in force: Circuits continues as a separately
versioned project, maintains its documented support envelope, and carries its
own incident, security, dependency, and compatibility obligations. Upstream
adoption may reduce maintenance cost or provide useful patches, but it is not
required for release, support, or rollback. If the Product Owner determines
that the team can no longer staff the roles above, the only permitted outcome
is an explicit no-ship/no-new-release decision followed by a documented support
sunset; silently transferring responsibility to upstream is not allowed.

## Maintainer questions (non-blocking)

These questions should be sent to the relevant Electric maintainers when useful,
but an unanswered response never blocks Circuits execution:

- Are there any planned succession, deprecation, or roadmap statements affecting
  current Electric or Electric-compatible projects?
- Which compatibility guarantees, if any, are intended across current Electric,
  ElectricSQL clients, and Circuits' native API?
- Which upstream advisories, release channels, or support contacts should a
  Circuits maintainer monitor?
- Would upstream accept specific, independently tested Circuits patches, and
  under what review/licensing process?

## Consequences

This decision makes ownership and support obligations explicit and permits
release planning to proceed without an upstream response. It also accepts the
cost of maintaining a distinct release/security/support operation and requires
every future compatibility or upstream-derived change to carry local evidence.
GOV-002 and later release tasks define the concrete support/capacity envelope;
they may narrow what is supported but may not reintroduce an upstream-adoption
dependency.
