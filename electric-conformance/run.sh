#!/usr/bin/env bash
# Run ElectricSQL's own conformance tests against electric-circuits's /v1/shape adapter.
#
# The test files in this directory are Elixir tests that execute INSIDE an ElectricSQL checkout
# (they use Electric's official Electric.Client and the vendored, formerly-official
# OracleHarness/ShapeChecker Postgres comparison support).
# This script wires everything up: it locates (or clones) an Electric checkout, copies the test
# files into its sync-service test tree, builds our release engine, and runs the chosen suites.
#
#   electric-conformance/run.sh [oracle|property|subqueries|all]   # default: all
#
# Env:
#   ELECTRIC_DIR    path to a disposable ElectricSQL checkout (default: ../electric next to this repo;
#                   cloned from ELECTRIC_REPO if absent). Existing checkouts must already be at
#                   ELECTRIC_REF; the runner never checks out or otherwise changes them.
#   ELECTRIC_REPO   clone source (default https://github.com/electric-sql/electric)
#   ELECTRIC_REF    required checkout identity (default: the pinned Electric main commit below)
#   ORACLE_RUNS / ORACLE_SHAPE_COUNT / ORACLE_BATCH_COUNT / ORACLE_MUTATIONS_PER_TXN
#                   property-test tunables (passed through)
#
# Requirements: elixir/mix, a Rust toolchain, and PostgreSQL 18 binaries (initdb/pg_ctl) on PATH —
# the launcher boots its own ephemeral Postgres.
#
# Note: copying overwrites `test/integration/subquery_*_test.exs` in the Electric checkout with
# the electric-circuits variants (same test bodies, swapped setup) — use a throwaway clone if you
# don't want the checkout modified.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(dirname "$here")"
suite="${1:-all}"

# The tests spawn our launcher (adapter + engine + ephemeral Postgres) through a BEAM port. Give each
# invocation an empty ownership manifest so cleanup can signal only the PIDs and PG data directory that
# this run created; do not sweep shared paths or unrelated listeners.
cleanup_file="$(mktemp "${TMPDIR:-/tmp}/electric-conformance-cleanup.XXXXXX")"
cleanup() {
  if [ -s "$cleanup_file" ]; then
    adapter_pid="$(node -e 'const fs=require("fs"); const m=JSON.parse(fs.readFileSync(process.argv[1])); process.stdout.write(String(m.adapterPid || ""))' "$cleanup_file" 2>/dev/null || true)"
    pg_ctl="$(node -e 'const fs=require("fs"); const m=JSON.parse(fs.readFileSync(process.argv[1])); process.stdout.write(m.pgCtl || "")' "$cleanup_file" 2>/dev/null || true)"
    pg_data="$(node -e 'const fs=require("fs"); const m=JSON.parse(fs.readFileSync(process.argv[1])); process.stdout.write(m.pgData || "")' "$cleanup_file" 2>/dev/null || true)"

    for pid in "$adapter_pid"; do
      case "$pid" in
        '' | *[!0-9]*) ;;
        *) kill -TERM "$pid" 2>/dev/null || true ;;
      esac
    done

    for _ in 1 2 3 4 5 6 7 8 9 10; do
      adapter_alive=0
      case "$adapter_pid" in '' | *[!0-9]*) ;; *) kill -0 "$adapter_pid" 2>/dev/null && adapter_alive=1 ;; esac
      [ "$adapter_alive" -eq 0 ] && break
      sleep 0.1
    done

    for pid in "$adapter_pid"; do
      case "$pid" in
        '' | *[!0-9]*) ;;
        *) kill -KILL "$pid" 2>/dev/null || true ;;
      esac
    done

    # The adapter owns its DS child in the normal graceful path. If a forced adapter exit skipped
    # that path, terminate only the engine/DS PIDs and fresh `el-econf-ds-*` root in this manifest.
    # This is intentionally not a process-name sweep or a shared-temp cleanup.
    "$repo/node_modules/.bin/tsx" "$repo/packages/bench/src/electric-adapter-cleanup.ts" "$cleanup_file"

    if [ -n "$pg_ctl" ] && [ -n "$pg_data" ]; then
      "$pg_ctl" -D "$pg_data" -m immediate -w stop >/dev/null 2>&1 || true
    fi
  fi
  rm -f "$cleanup_file"
}
trap cleanup EXIT

ELECTRIC_REPO="${ELECTRIC_REPO:-https://github.com/electric-sql/electric}"
ELECTRIC_REF="${ELECTRIC_REF:-2f11f91d6c580e47fb57924f5d3f7954329314d8}"
ELECTRIC_DIR="${ELECTRIC_DIR:-$repo/../electric}"

if [ ! -d "$ELECTRIC_DIR/packages/sync-service" ]; then
  echo "==> cloning $ELECTRIC_REPO -> $ELECTRIC_DIR"
  git clone --depth 1 "$ELECTRIC_REPO" "$ELECTRIC_DIR"
  git -C "$ELECTRIC_DIR" fetch --depth 1 origin "$ELECTRIC_REF"
  git -C "$ELECTRIC_DIR" checkout --detach FETCH_HEAD
fi
sync="$ELECTRIC_DIR/packages/sync-service"

expected_electric_commit="$(git -C "$ELECTRIC_DIR" rev-parse "$ELECTRIC_REF^{commit}")"
actual_electric_commit="$(git -C "$ELECTRIC_DIR" rev-parse HEAD)"
if [ "$actual_electric_commit" != "$expected_electric_commit" ]; then
  echo "ELECTRIC_DIR must be a disposable checkout at $expected_electric_commit; found $actual_electric_commit" >&2
  exit 2
fi

echo "==> building the release engine"
(cd "$repo" && cargo build --release -p electric-circuits-engine)

echo "==> copying test files into $sync"
cp "$here/electric_circuits_oracle_test.exs" \
   "$here/electric_circuits_oracle_property_test.exs" \
   "$here/subquery_move_out_test.exs" \
   "$here/subquery_dependency_update_test.exs" \
   "$sync/test/integration/"
cp "$here/el_ivm_setup.ex" "$sync/test/support/"

vendor="$here/vendor/electric-oracle-harness"
echo "==> verifying vendored Electric oracle harness"
(cd "$vendor" && shasum -a 256 -c SHA256SUMS)
mkdir -p "$sync/test/support/oracle_harness"
cp "$vendor/packages/sync-service/test/support/oracle_harness.ex" "$sync/test/support/"
cp "$vendor/packages/sync-service/test/support/oracle_harness/shape_checker.ex" \
   "$sync/test/support/oracle_harness/"

export ELECTRIC_CIRCUITS_DIR="$repo"
export ADAPTER_CLEANUP_FILE="$cleanup_file"
cd "$sync"
[ -d deps ] || (echo "==> mix deps.get" && mix deps.get)

case "$suite" in
  oracle)
    mix test test/integration/electric_circuits_oracle_test.exs ;;
  property)
    mix test test/integration/electric_circuits_oracle_property_test.exs ;;
  subqueries)
    mix test test/integration/subquery_move_out_test.exs test/integration/subquery_dependency_update_test.exs ;;
  all)
    mix test test/integration/electric_circuits_oracle_test.exs
    mix test test/integration/electric_circuits_oracle_property_test.exs
    mix test test/integration/subquery_move_out_test.exs test/integration/subquery_dependency_update_test.exs ;;
  *)
    echo "usage: $0 [oracle|property|subqueries|all]"; exit 2 ;;
esac
