# Electric oracle harness provenance

This directory vendors only the two test-support modules needed by the deterministic
`electric-conformance/run.sh oracle` suite. They are byte-for-byte copies of the last reachable
upstream versions that expose `Support.OracleHarness.test_against_oracle/4`:

- Repository: `https://github.com/electric-sql/electric.git`
- Reachable upstream branch: `origin/rob/postgres-oracle`
- Source commit: `633115ba05b0c436732b50e4b4a3dc0c78206594`
- License: Apache License 2.0, copied verbatim as [`LICENSE`](LICENSE) from that commit's root `LICENSE`

| Vendored path | Upstream path | Git blob | SHA-256 |
| --- | --- | --- | --- |
| `packages/sync-service/test/support/oracle_harness.ex` | `packages/sync-service/test/support/oracle_harness.ex` | `e4698f56d5398ee1b3710e87554a857f8f75dad4` | `1afad1859c75c84641bfbfdd504094dcfdc403dc5a1d922dd3447ef40ecb569d` |
| `packages/sync-service/test/support/oracle_harness/shape_checker.ex` | `packages/sync-service/test/support/oracle_harness/shape_checker.ex` | `b6bafe7f88d3f60909145e55a8b53fa744ef00de` | `532b236b20398fab90cec7b7950a8a2a324c26a4ed87aa81cb8e4da2a2d7c34d` |
| `LICENSE` | `LICENSE` | — | `0d542e0c8804e39aa7f37eb00da5a762149dc682d7829451287e11b938e94594` |

`SHA256SUMS` is checked by `run.sh` before either module is copied into the disposable checkout.
The suite itself remains pinned to Electric main commit
`2f11f91d6c580e47fb57924f5d3f7954329314d8`; the vendor commit is a source provenance only and is
not used as the Electric runtime checkout.
