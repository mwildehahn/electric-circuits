//! Real-PostgreSQL regression for the managed writer-ownership CAS protocol.
//!
//! The Indexed migration owns this DDL in production. This test provisions the exact relation as
//! a fixture only and is deliberately ignored unless the caller supplies an isolated PG instance.

use electric_circuits_engine::deployment::{self, Claim, ManagedDeploymentConfig, OwnershipError};
use electric_circuits_engine::pg;
use uuid::Uuid;

const DDL: &str = "
CREATE SCHEMA IF NOT EXISTS electric_circuits;
CREATE TABLE IF NOT EXISTS electric_circuits.writer_ownership (
  coordination_key char(64) PRIMARY KEY,
  generation bigint NOT NULL CHECK (generation >= 1),
  owner_revision text NOT NULL CHECK (length(owner_revision) BETWEEN 1 AND 255),
  phase text NOT NULL CHECK (phase IN ('active', 'quiesced')),
  handoff_id uuid,
  source_commit_id uuid,
  updated_at timestamptz NOT NULL DEFAULT statement_timestamp(),
  CHECK (phase = 'active' OR (handoff_id IS NOT NULL AND source_commit_id IS NOT NULL))
);";

#[tokio::test]
#[ignore = "requires an isolated real PostgreSQL instance via ELECTRIC_CIRCUITS_TEST_PG_URL"]
async fn ownership_cas_fences_bootstrap_handoff_restart_and_reverse_transfer() -> anyhow::Result<()> {
    let url = std::env::var("ELECTRIC_CIRCUITS_TEST_PG_URL")?;
    let first = pg::connect(&url).await?;
    let second = pg::connect(&url).await?;
    first.batch_execute(DDL).await?;
    let key = format!("{:064x}", Uuid::new_v4().as_u128());
    first.execute("DELETE FROM electric_circuits.writer_ownership WHERE coordination_key = $1", &[&key]).await?;

    let active_a = ManagedDeploymentConfig { revision: "revision-a".into(), initial_active: true };
    let active_b = ManagedDeploymentConfig { revision: "revision-b".into(), initial_active: true };
    let (a, b) = tokio::join!(
        deployment::claim_or_observe(&first, &active_a, &key),
        deployment::claim_or_observe(&second, &active_b, &key)
    );
    let (owner, standby) = match (a?, b?) {
        (Claim::Active(owner), Claim::Standby(standby)) => (owner, standby),
        (Claim::Standby(standby), Claim::Active(owner)) => (owner, standby),
        other => panic!("exactly one concurrent bootstrap may become owner: {other:?}"),
    };
    assert_eq!(standby.unwrap().generation, 1);

    let handoff = Uuid::new_v4();
    let receipt = Uuid::new_v4();
    let wrong = deployment::quiesce(&first, &key, &owner.owner_revision, 2, handoff, receipt).await.unwrap_err();
    assert!(
        wrong.chain().any(|cause| cause.downcast_ref::<OwnershipError>().is_some()),
        "wrong generation must be a typed CAS conflict, got: {wrong:#}"
    );
    let quiesced = deployment::quiesce(&first, &key, &owner.owner_revision, 1, handoff, receipt).await?;
    assert_eq!(quiesced.generation, 1);
    assert_eq!(deployment::quiesce(&first, &key, &owner.owner_revision, 1, handoff, receipt).await?, quiesced);

    let successor = if owner.owner_revision == "revision-a" { "revision-b" } else { "revision-a" };
    let wrong_promote =
        deployment::promote(&first, &key, successor, &owner.owner_revision, 2, handoff, receipt).await.unwrap_err();
    assert!(
        wrong_promote.chain().any(|cause| cause.downcast_ref::<OwnershipError>().is_some()),
        "wrong generation must be a typed CAS conflict, got: {wrong_promote:#}"
    );
    let promoted = deployment::promote(&first, &key, successor, &owner.owner_revision, 1, handoff, receipt).await?;
    assert_eq!(promoted.generation, 2);
    assert_eq!(
        deployment::promote(&first, &key, successor, &owner.owner_revision, 1, handoff, receipt).await?,
        promoted
    );

    // A former revision is fenced after restart; the promoted revision alone can reclaim gen 2.
    assert!(matches!(deployment::claim_or_observe(&first, &active_a, &key).await?, Claim::Standby(_)));
    assert!(matches!(deployment::claim_or_observe(&first, &active_b, &key).await?, Claim::Active(_)));

    let reverse_handoff = Uuid::new_v4();
    let reverse_receipt = Uuid::new_v4();
    deployment::quiesce(&first, &key, successor, 2, reverse_handoff, reverse_receipt).await?;
    let reverse = if successor == "revision-a" { "revision-b" } else { "revision-a" };
    let reversed = deployment::promote(&first, &key, reverse, successor, 2, reverse_handoff, reverse_receipt).await?;
    assert_eq!(reversed.generation, 3);

    first.execute("DELETE FROM electric_circuits.writer_ownership WHERE coordination_key = $1", &[&key]).await?;
    Ok(())
}
