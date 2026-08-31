//! Opt-in persisted writer ownership for managed blue/green deployments.
//!
//! The production migration owns the `electric_circuits.writer_ownership` table. This module never
//! creates it: a missing table is a fail-closed boot/configuration error, not an opportunity for an
//! engine process to grant itself authority.

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use tokio_postgres::Client;

use crate::store_identity::StoreBound;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedDeploymentConfig {
    pub revision: String,
    pub initial_active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ownership {
    pub coordination_key: String,
    pub generation: i64,
    pub owner_revision: String,
    pub phase: OwnershipPhase,
    pub handoff_id: Option<String>,
    pub source_commit_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnershipPhase {
    Active,
    Quiesced,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Claim {
    Active(Ownership),
    Standby(Option<Ownership>),
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum OwnershipError {
    #[error("managed deployment ownership conflict")]
    Conflict,
    #[error("managed deployment is not configured")]
    Disabled,
    #[error("control admission was open; acquire a fresh source receipt and retry quiesce")]
    PrecloseRequired,
}

/// The coordinator is unavailable or returned malformed state. It is distinct from a compare and
/// set conflict: callers may retry this response, but must not advance a handoff on it.
#[derive(Debug, thiserror::Error)]
#[error("managed deployment ownership storage is unavailable: {source}")]
pub struct OwnershipBackend {
    #[source]
    source: anyhow::Error,
}

fn backend<T>(result: Result<T>) -> Result<T> {
    result.map_err(|source| anyhow::Error::new(OwnershipBackend { source }))
}

/// SHA-256 of the canonical StoreBound fields, a NUL separator, and the logical slot name.
///
/// The serialization is intentionally explicit: serde JSON permits representation changes while this
/// value is a database primary key shared by independently built revisions.
pub fn coordination_key(bound: &StoreBound, slot: &str) -> String {
    let canonical = format!(
        "store_id={}\nstore_generation={}\nprotocol_version={}\nlayout_version={}\ndurability_mode={}\nwal_shard_count={}\nstream_lane_count={}\nfilesystem_uuid={}\nstack_namespace={}\ningest_epoch={}\nquery_generation={}",
        bound.store.store_id,
        bound.store.store_generation,
        bound.store.protocol_version,
        bound.store.layout_version,
        bound.store.durability_mode,
        bound.store.wal_shard_count,
        bound.store.stream_lane_count,
        bound.store.filesystem_uuid,
        bound.stack_namespace,
        bound.ingest_epoch,
        bound.query_generation,
    );
    let mut hash = Sha256::new();
    hash.update(canonical.as_bytes());
    hash.update([0]);
    hash.update(slot.as_bytes());
    format!("{:x}", hash.finalize())
}

fn row_to_ownership(key: &str, row: &tokio_postgres::Row) -> Result<Ownership> {
    let phase: String = row.try_get("phase").context("read ownership phase")?;
    let phase = match phase.as_str() {
        "active" => OwnershipPhase::Active,
        "quiesced" => OwnershipPhase::Quiesced,
        other => bail!("writer ownership row has invalid phase '{other}'"),
    };
    Ok(Ownership {
        coordination_key: key.to_string(),
        generation: row.try_get("generation").context("read ownership generation")?,
        owner_revision: row.try_get("owner_revision").context("read ownership owner revision")?,
        phase,
        handoff_id: row.try_get("handoff_id").context("read ownership handoff id")?,
        source_commit_id: row.try_get("source_commit_id").context("read ownership source receipt")?,
    })
}

async fn read(client: &Client, key: &str) -> Result<Option<Ownership>> {
    let row = client
        .query_opt(
            "SELECT generation, owner_revision, phase, handoff_id::text AS handoff_id, source_commit_id::text AS source_commit_id \
             FROM electric_circuits.writer_ownership WHERE coordination_key = $1",
            &[&key],
        )
        .await
        .context("read writer ownership");
    let row = backend(row)?;
    backend(row.as_ref().map(|row| row_to_ownership(key, row)).transpose())
}

/// Claim the initially absent row only when the operator explicitly enables bootstrap. A conflict
/// never overwrites an owner; it simply identifies a standby process.
pub async fn claim_or_observe(client: &Client, config: &ManagedDeploymentConfig, key: &str) -> Result<Claim> {
    if config.initial_active {
        let inserted = client
            .query_opt(
                "INSERT INTO electric_circuits.writer_ownership \
                 (coordination_key, generation, owner_revision, phase) \
                 VALUES ($1, 1, $2, 'active') \
                 ON CONFLICT (coordination_key) DO NOTHING \
                 RETURNING generation, owner_revision, phase, handoff_id::text AS handoff_id, source_commit_id::text AS source_commit_id",
                &[&key, &config.revision],
            )
            .await
            .context("bootstrap writer ownership");
        let inserted = backend(inserted)?;
        if let Some(row) = inserted {
            return Ok(Claim::Active(backend(row_to_ownership(key, &row))?));
        }
    }
    let ownership = read(client, key).await?;
    Ok(match ownership {
        Some(row) if row.phase == OwnershipPhase::Active && row.owner_revision == config.revision => Claim::Active(row),
        other => Claim::Standby(other),
    })
}

pub async fn status(client: &Client, key: &str) -> Result<Option<Ownership>> {
    read(client, key).await
}

pub async fn quiesce(
    client: &Client,
    key: &str,
    expected_owner: &str,
    generation: i64,
    handoff_id: uuid::Uuid,
    source_commit_id: uuid::Uuid,
) -> Result<Ownership> {
    let handoff = handoff_id.to_string();
    let source = source_commit_id.to_string();
    let row = client
        .query_opt(
            "UPDATE electric_circuits.writer_ownership \
             SET phase = 'quiesced', handoff_id = $4::uuid, source_commit_id = $5::uuid, updated_at = statement_timestamp() \
             WHERE coordination_key = $1 AND owner_revision = $2 AND generation = $3 AND phase = 'active' \
             RETURNING generation, owner_revision, phase, handoff_id::text AS handoff_id, source_commit_id::text AS source_commit_id",
            &[&key, &expected_owner, &generation, &handoff_id, &source_commit_id],
        )
        .await;
    let row = backend(row.context("quiesce writer ownership"))?;
    if let Some(row) = row {
        return backend(row_to_ownership(key, &row));
    }
    let existing = read(client, key).await?;
    match existing {
        Some(row)
            if row.phase == OwnershipPhase::Quiesced
                && row.owner_revision == expected_owner
                && row.generation == generation
                && row.handoff_id.as_deref() == Some(handoff.as_str())
                && row.source_commit_id.as_deref() == Some(source.as_str()) =>
        {
            Ok(row)
        }
        _ => bail!(OwnershipError::Conflict),
    }
}

pub async fn promote(
    client: &Client,
    key: &str,
    successor_revision: &str,
    expected_owner: &str,
    generation: i64,
    handoff_id: uuid::Uuid,
    source_commit_id: uuid::Uuid,
) -> Result<Ownership> {
    let handoff = handoff_id.to_string();
    let source = source_commit_id.to_string();
    let row = client
        .query_opt(
            "UPDATE electric_circuits.writer_ownership \
             SET owner_revision = $2, generation = generation + 1, phase = 'active', updated_at = statement_timestamp() \
             WHERE coordination_key = $1 AND owner_revision = $3 AND generation = $4 AND phase = 'quiesced' \
               AND handoff_id = $5::uuid AND source_commit_id = $6::uuid \
             RETURNING generation, owner_revision, phase, handoff_id::text AS handoff_id, source_commit_id::text AS source_commit_id",
            &[&key, &successor_revision, &expected_owner, &generation, &handoff_id, &source_commit_id],
        )
        .await;
    let row = backend(row.context("promote writer ownership"))?;
    if let Some(row) = row {
        return backend(row_to_ownership(key, &row));
    }
    let existing = read(client, key).await?;
    match existing {
        Some(row)
            if row.phase == OwnershipPhase::Active
                && row.owner_revision == successor_revision
                && row.generation == generation + 1
                && row.handoff_id.as_deref() == Some(handoff.as_str())
                && row.source_commit_id.as_deref() == Some(source.as_str()) =>
        {
            Ok(row)
        }
        _ => bail!(OwnershipError::Conflict),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store_identity::StreamScope;

    #[test]
    fn coordination_key_is_stable_and_fences_each_slot_or_lineage() {
        let bound = StoreBound::coupled_v1(&StreamScope::in_process_test_scope());
        let first = coordination_key(&bound, "circuits_slot");
        assert_eq!(first.len(), 64);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()));
        assert_eq!(first, coordination_key(&bound, "circuits_slot"));
        assert_ne!(first, coordination_key(&bound, "another_slot"));
        let mut other = bound.clone();
        other.query_generation = "other-query".to_string();
        assert_ne!(first, coordination_key(&other, "circuits_slot"));
    }
}
