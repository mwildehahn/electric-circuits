import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: {
    globalSetup: ['./vitest.global-setup.ts'],
    // Sibling agent worktrees live under .claude/worktrees and carry their own copies of the
    // test files (without node_modules) — never collect them from this checkout.
    // Script gates use node:test and run through their explicit package scripts; Vitest treats
    // their registrations as no suites, so keep the runners separate. `test:node` is the supported
    // product Node gate; `test:readiness-plan` is a deliberately non-gating planner audit whose
    // known-red state must remain visible when explicitly invoked.
    exclude: [
      '**/node_modules/**',
      '**/.claude/worktrees/**',
      'scripts/postgres-image-version.test.ts',
      'scripts/readiness-evidence.test.ts',
      'scripts/readiness-plan.test.ts',
    ],
    // Most conformance files boot an engine subprocess, a durable-streams server, and a dedicated
    // logical-replication database in the shared PostgreSQL fixture.  Running several such stacks
    // at once starves their drain/readiness deadlines; one fork makes that process topology
    // explicit while still running every collected file exactly once.
    pool: 'forks',
    poolOptions: { forks: { minForks: 1, maxForks: 1 } },
    testTimeout: 60000,
    hookTimeout: 60000,
  },
})
