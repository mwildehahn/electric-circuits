# Reconciliation blocked: managed-source TLS follow-up

Task `electriccircuits-task-9iq` authorizes a reconciliation PR that cherry-picks
`bde9b6f` and `397b723` onto `origin/main`. The `bde9b6f` cherry-pick completed
after resolving only the three explicitly authorized hunks in
`apps/engine/src/engine/mod.rs`, preserving main's `StoreAdmission` boundary and
adding `PostgresSetup` plus `Engine::new_pg_with_setup`.

Cherry-picking `397b723` stops with conflicts outside that authorized boundary:

- `Cargo.lock`
- `apps/engine/Cargo.toml`
- `apps/engine/src/pg.rs`
- `apps/engine/src/replication.rs`

The task says to stop rather than resolve anything larger than the three
`engine/mod.rs` hunks. The cherry-pick was aborted without resolving those files.
An operator must either authorize this expanded TLS reconciliation or provide a
replacement commit already compatible with current `origin/main`.
