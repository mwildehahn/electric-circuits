# Generated readiness plan

- Authoritative task inventory: 169
- Normative conditional edges: 36
- Future release profiles: unavailable until GOV-005
- Future scenario registry: unavailable until E2E-000S

## First ready sets

- `shared-pre-registry` after `PLAN-001`: `GOV-001`, `TST-000`
- `shared-pre-registry` after `PLAN-001,GOV-001`: `CMP-000`, `GOV-002`, `GOV-003`, `SEC-008B`, `TST-000`

## Ownership and gate matrix

- 169 principal boundaries, 507 phase rows, and one owner/applicability binding per row.
- Direct, merge, and qualification commands are chosen by task capability family; PLAN-001 author direct runs its contract suite.

## Deterministic identities

The non-self-referential projection below binds the five peer outputs. The `identity` command emits all six current output identities, including this report, for packet handoff.

- docs/production/readiness-tasks.json: git blob ac88ad63b545a28ad091fb1198883f0091c0c761; SHA-256 99845325d5d3dc19937cb783e932236c157e72f6947c9d83a0c00116760436b4
- docs/production/readiness-task.schema.json: git blob 7dd4023fbaaddd13c3c2b1c8ff2b95d0e5addfa1; SHA-256 4843615b8ad5b0ee3ef61ec42281a349a44658f635ea129d5d1f60d59b11a6c6
- docs/production/readiness-gates.json: git blob 04e879171c01336dacd95fc522ecad72d9403089; SHA-256 6e00f7f126eaa1780f8d026c75b9f53dfda1aa21c21a41774ea7a990b9b894c2
- scripts/readiness-plan.ts: git blob 5e14168e619be0e4022580263f62941bc00125fe; SHA-256 4b063ed29db6fd8b41772ec3ec41eac445b4a285aa48c4446ba4d8128cc19a46
- scripts/readiness-plan.test.ts: git blob 1324ff4bdc3028d460a7d8933bad09b40e4168b5; SHA-256 d4a5a3d008754b7c65659b209f3bd3e66b396ed0442240ca1eba5574934377cd

Use `pnpm exec tsx scripts/readiness-plan.ts identity` to emit the complete six-output identity set.
