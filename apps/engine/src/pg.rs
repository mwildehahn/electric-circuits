//! Postgres access for the Postgres-backed mode: connection, schema introspection, replication-slot
//! setup, and consistent backfill snapshots. This replaces the engine's in-memory `table_state` —
//! current data lives in Postgres and is read back on demand (shape backfill), while ongoing changes
//! arrive via logical replication (see `replication.rs`).

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result, bail};
use tokio_postgres::{Client, NoTls};

use crate::heap_size::HeapSize;
use crate::predicate::CompiledPredicate;
use crate::schema::{ColumnDef, ColumnType, FingerprintColumn, SchemaFingerprint, TableDef, TableSchema};
use crate::table_ref::TableRef;
use crate::value::Row;

/// Connect and drive the connection on a background task. Returns the query `Client`.
/// For per-request work (backfills, query-backs, subset queries) prefer [`pool_for`] — a fresh
/// TCP+auth handshake per shape creation is the fleet benchmark's p99 driver, and thousands of
/// concurrent creations exhaust ephemeral ports.
pub async fn connect(url: &str) -> Result<Client> {
    let (client, conn) = tokio_postgres::connect(url, NoTls).await.context("connect postgres")?;
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            tracing::error!("postgres connection error: {e}");
        }
    });
    Ok(client)
}

/// Maximum connections per [`Pool`], set once at boot from `ELECTRIC_DB_POOL_SIZE` (default 20).
static POOL_SIZE: OnceLock<usize> = OnceLock::new();

/// One shared pool per distinct URL for the process lifetime.
static POOLS: OnceLock<std::sync::Mutex<HashMap<String, Pool>>> = OnceLock::new();

/// Whether the publication publishes stored generated columns (PG18 `pg_publication.pubgencols`),
/// resolved once at boot by [`inspect_publication`]. Default `false` — every server before 18, and
/// PG18's own default.
static PUBLISH_GENERATED: OnceLock<bool> = OnceLock::new();

/// Record what the publication does with stored generated columns. Call once at boot, before the
/// first [`fingerprints`].
pub fn set_publish_generated(v: bool) {
    let _ = PUBLISH_GENERATED.set(v);
}

/// Does the publication deliver stored generated columns? Decides whether the schema fingerprint
/// includes them, so that the catalog's view and the wire's `Relation` message agree by
/// construction (ADR-0005).
pub fn publish_generated() -> bool {
    *PUBLISH_GENERATED.get_or_init(|| false)
}

/// Set the per-URL pool capacity. Call once at boot, before the first [`pool_for`].
pub fn set_pool_size(size: usize) {
    let _ = POOL_SIZE.set(size.max(1));
}

/// The shared connection pool for `url` (created on first use).
pub fn pool_for(url: &str) -> Pool {
    let pools = POOLS.get_or_init(Default::default);
    let mut pools = pools.lock().unwrap();
    pools
        .entry(url.to_string())
        .or_insert_with(|| Pool::new(url.to_string(), *POOL_SIZE.get_or_init(|| 20)))
        .clone()
}

/// A small connection pool: at most `size` concurrent checkouts, idle connections reused.
/// Backfills/query-backs are self-contained `BEGIN … COMMIT` units with no session state, so
/// checkin only has to clear a possibly-aborted transaction (`ROLLBACK`, a no-op warning on a
/// clean session) before the connection is reusable.
#[derive(Clone)]
pub struct Pool {
    inner: Arc<PoolInner>,
}

struct PoolInner {
    url: String,
    idle: std::sync::Mutex<Vec<Client>>,
    sem: Arc<tokio::sync::Semaphore>,
}

impl Pool {
    pub fn new(url: String, size: usize) -> Pool {
        Pool {
            inner: Arc::new(PoolInner {
                url,
                idle: std::sync::Mutex::new(Vec::new()),
                sem: Arc::new(tokio::sync::Semaphore::new(size.max(1))),
            }),
        }
    }

    /// Check out a connection; waits if all `size` are in use. The checkout is returned to the
    /// pool (or discarded, if broken) when the guard drops.
    pub async fn get(&self) -> Result<PooledClient> {
        let permit =
            self.inner.sem.clone().acquire_owned().await.context("pg pool closed")?;
        // Reuse an idle connection if it is still healthy; otherwise dial a new one.
        let reused = self.inner.idle.lock().unwrap().pop().filter(|c| !c.is_closed());
        let client = match reused {
            Some(c) => c,
            None => connect(&self.inner.url).await?,
        };
        Ok(PooledClient { client: Some(client), inner: self.inner.clone(), permit: Some(permit) })
    }
}

/// A pooled connection checkout. Derefs to `tokio_postgres::Client`.
pub struct PooledClient {
    client: Option<Client>,
    inner: Arc<PoolInner>,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

impl std::ops::Deref for PooledClient {
    type Target = Client;
    fn deref(&self) -> &Client {
        self.client.as_ref().expect("client present until drop")
    }
}

impl Drop for PooledClient {
    fn drop(&mut self) {
        let Some(client) = self.client.take() else { return };
        let permit = self.permit.take();
        if client.is_closed() {
            return; // permit drops here, freeing the slot
        }
        let inner = self.inner.clone();
        // Clear any transaction the caller left open/aborted, then check the connection back in.
        // The permit is held until the connection is actually idle again, so live connections
        // never exceed the pool size.
        tokio::spawn(async move {
            if client.batch_execute("ROLLBACK").await.is_ok() {
                inner.idle.lock().unwrap().push(client);
            }
            drop(permit);
        });
    }
}

/// Double-quote a Postgres identifier.
pub fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Map a Postgres `data_type` (from information_schema) to our column type.
fn map_pg_type(data_type: &str) -> ColumnType {
    match data_type {
        "integer" | "bigint" | "smallint" => ColumnType::Int,
        "boolean" => ColumnType::Bool,
        "real" | "double precision" | "numeric" => ColumnType::Float,
        _ => ColumnType::Text, // text, varchar, char, uuid, timestamptz, ... -> treated as text
    }
}

/// List all base tables in `schema` that have a primary key, skipping the engine's own bookkeeping
/// table — which is `public.__el_sync` **specifically**: a user table that happens to be called
/// `__el_sync` in another schema is ordinary data and is replicated like any other. Used by "introspect all" mode (`ELECTRIC_CIRCUITS_PG_TABLES=*` → `public`,
/// `schema.*` → that schema), where the set of tables isn't known up front (e.g. driving Electric's
/// integration tests over varied schemas). The schema is a bound parameter, and `to_regclass` gets
/// the properly quoted qualified name, so an odd schema name can neither inject nor mis-resolve.
pub async fn list_tables(client: &Client, schema: &str) -> Result<Vec<TableRef>> {
    let rows = client
        .query(
            "select t.table_name from information_schema.tables t \
             where t.table_schema = $1 and t.table_type = 'BASE TABLE' \
               and not (t.table_schema = 'public' and t.table_name = '__el_sync') \
               and exists (select 1 from pg_index i \
                           where i.indrelid = to_regclass(quote_ident(t.table_schema)||'.'||quote_ident(t.table_name)) \
                             and i.indisprimary) \
             order by t.table_name",
            &[&schema],
        )
        .await
        .with_context(|| format!("list tables in schema '{schema}'"))?;
    // A relation whose name cannot be spelled as a canonical `schema.name` (a quoted identifier
    // containing a dot or a quote) has no unambiguous identity under ADR-0002, so discovery skips it
    // — loudly. It is then simply untracked: a shape on it is refused with "unknown table" at the
    // API boundary, never served wrong. An EXPLICIT list entry for such a table cannot exist at all
    // (the config parse rejects it), so this is discovery's case only.
    Ok(rows
        .iter()
        .filter_map(|r| {
            let name: String = r.get(0);
            TableRef::new(schema, &name)
                .map_err(|e| tracing::warn!("introspect-all: skipping relation '{schema}'.'{name}': {e:#}"))
                .ok()
        })
        .collect())
}

/// Read the live **schema fingerprint** of every given table in ONE round trip: `pg_class` ⨝
/// `pg_attribute`, live columns only (`attnum > 0 and not attisdropped`) in `attnum` order, plus
/// `pg_class.relreplident` (ADR-0005).
///
/// A table the query finds no row for is simply **absent** from the result — that is exactly how a
/// DROPped table is detected, by the reconciler and by the drift handler alike. The wanted set is
/// two bound `text[]` params joined through `unnest`, so neither an odd schema name nor a large
/// table set turns into interpolated SQL.
pub async fn fingerprints(
    client: &Client,
    tables: &[TableRef],
) -> Result<HashMap<TableRef, SchemaFingerprint>> {
    if tables.is_empty() {
        return Ok(HashMap::new());
    }
    let schemas: Vec<String> = tables.iter().map(|t| t.schema().to_string()).collect();
    let names: Vec<String> = tables.iter().map(|t| t.name().to_string()).collect();
    // Stored generated columns are included **iff the publication publishes them** ($3): pgoutput
    // omits them unless `pubgencols = 's'`, and a fingerprint that disagrees with the wire on this
    // would report drift on every single `Relation` message, forever (ADR-0005).
    let with_generated = publish_generated();
    let rows = client
        .query(
            "select n.nspname, c.relname, c.relreplident::text, \
                    a.attname, a.atttypid::int8, a.atttypmod \
             from pg_class c \
             join pg_namespace n on n.oid = c.relnamespace \
             join unnest($1::text[], $2::text[]) as w(s, t) on w.s = n.nspname and w.t = c.relname \
             left join pg_attribute a \
               on a.attrelid = c.oid and a.attnum > 0 and not a.attisdropped \
                  and (a.attgenerated = '' or $3::bool) \
             order by n.nspname, c.relname, a.attnum",
            &[&schemas, &names, &with_generated],
        )
        .await
        .context("read schema fingerprints")?;
    let mut out: HashMap<TableRef, SchemaFingerprint> = HashMap::new();
    for r in &rows {
        let schema: String = r.get(0);
        let name: String = r.get(1);
        let Ok(table) = TableRef::new(&schema, &name) else { continue };
        let replident: String = r.get(2);
        let entry = out.entry(table).or_insert_with(|| SchemaFingerprint {
            columns: Vec::new(),
            replident: replident.as_bytes().first().copied().unwrap_or(b'?'),
            // Filled by the primary-key pass below; a table without one keeps `Some(vec![])`, which
            // is still a KNOWN key (and different from any real one), not "unknown".
            pk: Some(Vec::new()),
        });
        // `CREATE TABLE t ()` is legal: the LEFT JOIN then yields one all-NULL column row.
        let Some(attname) = r.get::<_, Option<String>>(3) else { continue };
        let type_oid: i64 = r.get(4);
        let typmod: i32 = r.get(5);
        entry.columns.push(FingerprintColumn { name: attname, type_oid: type_oid as u32, typmod });
    }
    // Second pass: the primary key, in `indkey` order. A separate query rather than another join —
    // joining it into the column query would multiply every column row by the key width.
    let pk_rows = client
        .query(
            "select n.nspname, c.relname, a.attname \
             from pg_class c \
             join pg_namespace n on n.oid = c.relnamespace \
             join unnest($1::text[], $2::text[]) as w(s, t) on w.s = n.nspname and w.t = c.relname \
             join pg_index i on i.indrelid = c.oid and i.indisprimary \
             join pg_attribute a on a.attrelid = c.oid and a.attnum = any(i.indkey) \
             order by n.nspname, c.relname, array_position(i.indkey, a.attnum)",
            &[&schemas, &names],
        )
        .await
        .context("read primary keys")?;
    for r in &pk_rows {
        let schema: String = r.get(0);
        let name: String = r.get(1);
        let Ok(table) = TableRef::new(&schema, &name) else { continue };
        if let Some(fp) = out.get_mut(&table) {
            fp.pk.get_or_insert_with(Vec::new).push(r.get(2));
        }
    }
    Ok(out)
}

/// Introspect a table's columns (+ types), its primary key and its [`SchemaFingerprint`] from the
/// catalog. Both the column lookup and the primary-key lookup are qualified by `(table_schema,
/// table_name)` / the quoted qualified `to_regclass`, so same-named tables in different schemas
/// never cross. Errors when the table does not exist — see [`introspect_opt`] for the caller that
/// treats "gone" as an outcome rather than a failure.
pub async fn introspect(client: &Client, table: &TableRef) -> Result<TableDef> {
    introspect_opt(client, table)
        .await?
        .with_context(|| format!("table '{table}' not found in postgres"))
}

/// [`introspect`], but a table Postgres no longer has yields `Ok(None)` instead of an error: the
/// schema-drift handler must tell "this table was ALTERed" from "this table was DROPped", and
/// re-introspecting is where it finds out (ADR-0005).
pub async fn introspect_opt(client: &Client, table: &TableRef) -> Result<Option<TableDef>> {
    let (schema, name) = (table.schema(), table.name());
    let Some(fingerprint) = fingerprints(client, std::slice::from_ref(table)).await?.remove(table) else {
        return Ok(None);
    };
    let col_rows = client
        .query(
            "select column_name, data_type, udt_name, \
                    (is_identity = 'YES' or column_default is not null) as has_default \
             from information_schema.columns \
             where table_schema = $1 and table_name = $2 order by ordinal_position",
            &[&schema, &name],
        )
        .await
        .context("introspect columns")?;
    if col_rows.is_empty() {
        // `pg_class` had the relation but `information_schema` has no columns for it: a
        // zero-column table, or the table was dropped between the two queries. Either way there is
        // nothing to compile — same answer as "not found".
        return Ok(None);
    }
    let mut columns = BTreeMap::new();
    for r in &col_rows {
        let name: String = r.get(0);
        let dt: String = r.get(1);
        // udt_name (pg_type.typname, e.g. `uuid`, `int4`, `timestamptz`) is the canonical, always-castable
        // type name — used to cast bound text params to the native type in backfill SQL.
        let udt: String = r.get(2);
        // Auto-defaulted (IDENTITY or DEFAULT) → the add-row form can treat it as optional.
        let has_default: bool = r.get(3);
        columns.insert(name, ColumnDef { ty: map_pg_type(&dt), pg_type: Some(udt), has_default });
    }

    // Composite primary keys are supported (e.g. Electric's `*_tags` tables); columns are ordered by
    // their position in the index key so the synthesized row key is deterministic.
    let qualified = table.quote_qualified();
    let pk_rows = client
        .query(
            "select a.attname from pg_index i \
             join pg_attribute a on a.attrelid = i.indrelid and a.attnum = any(i.indkey) \
             where i.indrelid = to_regclass($1) and i.indisprimary \
             order by array_position(i.indkey, a.attnum)",
            &[&qualified],
        )
        .await
        .context("introspect primary key")?;
    if pk_rows.is_empty() {
        bail!("table '{table}' must have a primary key");
    }
    let primary_key: Vec<String> = pk_rows.iter().map(|r| r.get(0)).collect();
    Ok(Some(TableDef { columns, primary_key, fingerprint: Some(fingerprint) }))
}

/// `ALTER TABLE … REPLICA IDENTITY FULL` so logical decoding carries the full old row. Used at
/// boot, where waiting for the lock is the right thing to do.
pub async fn ensure_replica_identity_full(client: &Client, table: &TableRef) -> Result<()> {
    client
        .batch_execute(&format!("ALTER TABLE {} REPLICA IDENTITY FULL", table.quote_qualified()))
        .await
        .with_context(|| format!("set REPLICA IDENTITY FULL on {table}"))
}

/// [`ensure_replica_identity_full`] with a bounded wait for the `ACCESS EXCLUSIVE` lock.
///
/// The drift handler runs INLINE in the ingestor: an unbounded lock wait there would stall the
/// whole replication stream — every table's — behind one long-running reader of this one. With
/// `lock_timeout` the statement gives up instead, the caller marks the table unresolved, and its
/// retry task tries again later while ingest keeps flowing.
pub async fn ensure_replica_identity_full_bounded(
    client: &Client,
    table: &TableRef,
    lock_timeout: std::time::Duration,
) -> Result<()> {
    // `SET LOCAL` needs a transaction to be local to; a failed statement leaves it aborted, which
    // `PooledClient::drop` clears with its `ROLLBACK` before the connection is reused.
    client
        .batch_execute(&format!(
            "BEGIN; SET LOCAL lock_timeout = '{}ms'; ALTER TABLE {} REPLICA IDENTITY FULL; COMMIT;",
            lock_timeout.as_millis().max(1),
            table.quote_qualified()
        ))
        .await
        .with_context(|| format!("set REPLICA IDENTITY FULL on {table} (lock_timeout {lock_timeout:?})"))
}

/// The only output plugin the engine speaks (`replication.rs` decodes pgoutput frames).
pub const PGOUTPUT: &str = "pgoutput";

/// Create the logical replication slot (`pgoutput`) if it does not exist. A leftover slot with a
/// different output plugin (e.g. `test_decoding` from an earlier engine version) is dropped and
/// recreated — the plugin cannot be changed in place.
pub async fn ensure_slot(client: &Client, slot: &str) -> Result<()> {
    let existing = client
        .query("select plugin from pg_replication_slots where slot_name = $1", &[&slot])
        .await
        .context("check slot")?;
    if let Some(row) = existing.first() {
        let plugin: String = row.get(0);
        if plugin == PGOUTPUT {
            return Ok(());
        }
        tracing::warn!("slot '{slot}' uses plugin '{plugin}'; dropping and recreating with pgoutput");
        client
            .execute("select pg_drop_replication_slot($1)", &[&slot])
            .await
            .context("drop stale slot")?;
    }
    create_slot(client, slot).await
}

async fn create_slot(client: &Client, slot: &str) -> Result<()> {
    client
        .execute("select pg_create_logical_replication_slot($1, 'pgoutput')", &[&slot])
        .await
        .context("create slot")?;
    Ok(())
}

/// Drop the slot (if it is still there) and create a fresh one — the Postgres half of an **epoch
/// reset** (ADR-0004). Unlike [`ensure_slot`] this never adopts what it finds: a broken epoch means
/// the slot on the server is not the one the engine bound to, whatever it looks like.
///
/// Only ever called with no walsender of the engine's own attached (boot before the ingestor spawns,
/// or the ingestor's own pre-connect check), so the drop is not fighting our own connection.
pub async fn recreate_slot(client: &Client, slot: &str) -> Result<()> {
    let existing = client
        .query("select 1 from pg_replication_slots where slot_name = $1", &[&slot])
        .await
        .context("check slot")?;
    if !existing.is_empty() {
        client
            .execute("select pg_drop_replication_slot($1)", &[&slot])
            .await
            .context("drop the old epoch's slot")?;
    }
    create_slot(client, slot).await
}

/// The cluster the engine is bound to, plus its clock — the identity half of a [`SlotObservation`].
///
/// `system_identifier` is generated by `initdb` and never changes, so it is what distinguishes "the
/// same database restored from a backup" (or a different cluster reachable at the same URL) from
/// the cluster whose WAL the engine's shapes were built from. `timeline_id` moves on a
/// promotion/PITR; it is **recorded, not acted on** (single primary — ADR-0004).
pub struct ClusterIdentity {
    pub system_identifier: String,
    pub timeline_id: u32,
    /// The server's clock as ISO-8601 UTC — stamped into the binding so the durable record says
    /// *when* the epoch started, on the same clock as everything else in Postgres.
    pub now_iso: String,
}

/// Read the cluster identity. `pg_control_system()` / `pg_control_checkpoint()` are `EXECUTE`-to-
/// `PUBLIC` by default (verified on PG18: `pg_proc.proacl` is NULL and an unprivileged role can call
/// them), so the epoch check needs no privilege beyond what the engine already has.
pub async fn cluster_identity(client: &Client) -> Result<ClusterIdentity> {
    // `system_identifier` is a uint64 stored as int8, so it is read as text rather than risking the
    // sign flip; the timestamp is formatted server-side (no date crate in the engine's tree).
    let row = client
        .query_one(
            "select (select system_identifier::text from pg_control_system()), \
                    (select timeline_id from pg_control_checkpoint()), \
                    to_char(now() at time zone 'utc', 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"')",
            &[],
        )
        .await
        .context("read the postgres cluster identity (pg_control_system/pg_control_checkpoint)")?;
    let system_identifier: String = row.get(0);
    let timeline_id: i32 = row.get(1);
    let now_iso: String = row.get(2);
    Ok(ClusterIdentity { system_identifier, timeline_id: timeline_id as u32, now_iso })
}

/// One `pg_replication_slots` row, as far as the epoch verdict cares (ADR-0004).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SlotRow {
    pub active: bool,
    /// The walsender holding the slot, if any. Postgres allows exactly one.
    pub active_pid: Option<i32>,
    /// `reserved` | `extended` | `unreserved` | `lost` (PG13+; absent on older servers).
    pub wal_status: Option<String>,
    pub confirmed_flush_lsn: Option<String>,
    pub plugin: Option<String>,
}

/// Everything the epoch verdict is computed from: the slot as Postgres reports it right now (absent
/// = the slot is gone) and the cluster it would be read from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotObservation {
    pub system_identifier: String,
    pub timeline_id: u32,
    pub slot: Option<SlotRow>,
}

/// Observe the slot + the cluster identity, for the epoch check at boot and before every ingestor
/// (re)connect.
pub async fn observe_slot(client: &Client, slot: &str) -> Result<SlotObservation> {
    let id = cluster_identity(client).await?;
    // `to_jsonb(s)` rather than a column list: `wal_status` exists only on PG13+, and asking jsonb
    // for a missing key is simply `None` (same trick as `inspect_publication`).
    let row = client
        .query_opt("select to_jsonb(s) from pg_replication_slots s where s.slot_name = $1", &[&slot])
        .await
        .context("read the replication slot")?;
    let slot = row.map(|row| {
        let j: serde_json::Value = row.get(0);
        SlotRow {
            active: j.get("active").and_then(serde_json::Value::as_bool).unwrap_or(false),
            active_pid: j
                .get("active_pid")
                .and_then(serde_json::Value::as_i64)
                .map(|v| v as i32),
            wal_status: j.get("wal_status").and_then(serde_json::Value::as_str).map(str::to_string),
            confirmed_flush_lsn: j
                .get("confirmed_flush_lsn")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
            plugin: j.get("plugin").and_then(serde_json::Value::as_str).map(str::to_string),
        }
    });
    Ok(SlotObservation { system_identifier: id.system_identifier, timeline_id: id.timeline_id, slot })
}

/// Create the publication the pgoutput stream filters on, if it does not exist. `FOR ALL TABLES`
/// (requires superuser): the ingestor drops changes for untracked relations itself, and the set of
/// tracked tables can grow via introspect-all restarts without publication surgery.
pub async fn ensure_publication(client: &Client, publication: &str) -> Result<()> {
    let exists = client
        .query("select 1 from pg_publication where pubname = $1", &[&publication])
        .await
        .context("check publication")?;
    if exists.is_empty() {
        client
            .execute(&format!("create publication {} for all tables", quote_ident(publication)), &[])
            .await
            .context("create publication")?;
    }
    Ok(())
}

/// The two things about the publication that decide whether the wire can ever agree with the
/// catalog (ADR-0005).
///
/// A `Relation` message describes what the publication will DELIVER. If that is not the whole row,
/// no re-introspection can make the engine's compiled schema match it, and the engine would report
/// drift on every message forever — so a column list is refused at boot rather than discovered at
/// runtime, and generated-column publishing is folded into the fingerprint instead.
pub struct PublicationInfo {
    /// `pg_publication.pubgencols = 's'` (PG18+; absent ⇒ false).
    pub publish_generated: bool,
}

/// Check that the publication can deliver whole rows for every tracked table, and learn whether it
/// publishes stored generated columns. Errors — fatally, at boot — on a per-table column list.
pub async fn inspect_publication(
    client: &Client,
    publication: &str,
    tables: &[TableRef],
) -> Result<PublicationInfo> {
    // One row-to-jsonb read rather than a version-guarded column list: `pubgencols` exists only on
    // PG18+, and asking jsonb for a missing key is simply `None`.
    let row = client
        .query_opt("select to_jsonb(p) from pg_publication p where p.pubname = $1", &[&publication])
        .await
        .context("read publication")?
        .with_context(|| format!("publication '{publication}' does not exist"))?;
    let pubrow: serde_json::Value = row.get(0);
    let all_tables = pubrow.get("puballtables").and_then(serde_json::Value::as_bool).unwrap_or(false);
    let publish_generated =
        pubrow.get("pubgencols").and_then(serde_json::Value::as_str) == Some("s");

    // A `FOR ALL TABLES` publication cannot carry a column list at all, so only a hand-made one is
    // worth the second query.
    if !all_tables {
        let schemas: Vec<String> = tables.iter().map(|t| t.schema().to_string()).collect();
        let names: Vec<String> = tables.iter().map(|t| t.name().to_string()).collect();
        let listed = client
            .query(
                "select n.nspname, c.relname from pg_publication_rel pr \
                 join pg_publication p on p.oid = pr.prpubid \
                 join pg_class c on c.oid = pr.prrelid \
                 join pg_namespace n on n.oid = c.relnamespace \
                 join unnest($2::text[], $3::text[]) as w(s, t) on w.s = n.nspname and w.t = c.relname \
                 where p.pubname = $1 and pr.prattrs is not null \
                 order by n.nspname, c.relname",
                &[&publication, &schemas, &names],
            )
            .await
            .context("check publication column lists")?;
        if !listed.is_empty() {
            let which: Vec<String> = listed
                .iter()
                .map(|r| format!("{}.{}", r.get::<_, String>(0), r.get::<_, String>(1)))
                .collect();
            bail!(
                "publication '{publication}' has a column list on {}; the engine requires whole rows \
                 (a partial row can never be reconciled with the table's schema). Drop the column \
                 list, or stop syncing that table.",
                which.join(", ")
            );
        }
    }
    Ok(PublicationInfo { publish_generated })
}

pub struct Backfill {
    pub rows: Vec<Row>,
    /// `pg_current_wal_lsn()` of the snapshot. A transaction visible to this REPEATABLE READ snapshot
    /// committed strictly before it, so its commit LSN is `< seed_lsn` and its changes are already in
    /// `rows`; a transaction committing at/after the snapshot has commit LSN `>= seed_lsn`.
    pub seed_lsn: String,
    /// The snapshot's transaction-visibility gate — the *sound* backfill↔replication fence. See
    /// [`SnapshotGate`] for why LSN comparison alone is not.
    pub gate: SnapshotGate,
}

/// The backfill snapshot's visibility fence for replicated changes.
///
/// **Why not LSN alone:** `pg_current_wal_lsn()` is a WAL *write* position, but snapshot visibility is
/// decided at `ProcArrayEndTransaction` — which happens *after* the commit record is written and
/// fsynced. A transaction whose commit record is already in the WAL (commit LSN `< seed_lsn`) can
/// still be **invisible** to a snapshot taken during that window; skipping its replicated change by
/// LSN would drop the row from both the backfill and the live stream, permanently. Conversely a
/// visible commit can sit exactly *at* the boundary (`end_lsn == seed_lsn`) and be replayed as a
/// duplicate. Transaction-id visibility (`pg_current_snapshot()`) decides both cases exactly:
/// **skip a replicated change iff its xid was visible to the backfill snapshot.** (Every xid seen on
/// the slot is committed, so visibility reduces to: `xid < xmin`, or `xmin <= xid < xmax` and not
/// in-progress at snapshot time.)
///
/// The stored xids are `pg_current_snapshot()`'s xid8 values masked to 32 bits so they compare
/// against test_decoding's 32-bit xids; the fence spans seconds around a backfill, so epoch
/// wraparound (a ~4-billion-transaction horizon) cannot straddle it in practice.
///
/// When a change carries no parseable xid (library mode / non-PG sources), the gate falls back to
/// the LSN comparison, and with neither it never skips.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct SnapshotGate {
    /// `pg_current_wal_lsn()` at the snapshot (numeric) — the fallback fence + subset positioning.
    pub lsn: u64,
    xmin: u64,
    xmax: u64,
    xip: std::collections::HashSet<u64>,
}

impl HeapSize for SnapshotGate {
    /// Only `xip` (the in-progress xid set) owns heap; `lsn`/`xmin`/`xmax` are inline `u64`s.
    fn heap_bytes(&self) -> usize {
        self.xip.heap_bytes()
    }
}

impl SnapshotGate {
    /// A gate that never skips — for `changes_only` feeds (no backfill ⇒ forward everything) and
    /// library mode (no Postgres snapshot exists).
    pub fn passthrough() -> Self {
        SnapshotGate::default()
    }

    /// Build from `pg_current_snapshot()::text` ("xmin:xmax:xip1,xip2,…") + the snapshot LSN.
    pub fn parse(snapshot: &str, lsn: &str) -> Self {
        let mask = |v: u64| v & 0xFFFF_FFFF;
        let mut parts = snapshot.split(':');
        let xmin = parts.next().and_then(|s| s.trim().parse::<u64>().ok()).map(mask).unwrap_or(0);
        let xmax = parts.next().and_then(|s| s.trim().parse::<u64>().ok()).map(mask).unwrap_or(0);
        let xip = parts
            .next()
            .map(|s| s.split(',').filter_map(|x| x.trim().parse::<u64>().ok()).map(mask).collect())
            .unwrap_or_default();
        SnapshotGate { lsn: lsn_to_u64(lsn), xmin, xmax, xip }
    }

    /// Was committed transaction `xid` visible to this snapshot (i.e. already reflected in the
    /// backfill rows)?
    fn visible(&self, xid: u64) -> bool {
        if self.xmax == 0 {
            return false; // passthrough gate: nothing is "already seeded"
        }
        if xid < self.xmin {
            return true;
        }
        if xid >= self.xmax {
            return false;
        }
        !self.xip.contains(&xid)
    }

    /// Should a replicated change (commit LSN + optional xid) be skipped because the backfill
    /// snapshot already reflects it?
    pub fn should_skip(&self, commit_lsn: u64, xid: Option<u64>) -> bool {
        match xid {
            Some(x) => self.visible(x),
            None => commit_lsn != 0 && self.lsn != 0 && commit_lsn < self.lsn,
        }
    }
}

/// Read the table's current rows in a single repeatable-read snapshot, plus the snapshot LSN. The
/// engine seeds a shape/family from `rows` and skips replication changes whose COMMIT LSN is strictly
/// `< seed_lsn` (see `engine::process_envelope`; the comparison is against the transaction commit LSN
/// stamped by the ingestor, not the per-change record LSN).
/// Uses an explicit transaction over `&Client` (so it needs a dedicated connection, not a shared one).
///
/// `filter`, when given, is the shape's predicate: backfill reads only the matching rows
/// (`… WHERE <predicate>`) instead of the whole table, so a selective shape never scans/transfers the
/// rest. `None` reads the whole table (used while a family still seeds a full-table trace).
pub async fn backfill(client: &Client, ts: &TableSchema, filter: Option<&CompiledPredicate>) -> Result<Backfill> {
    client
        .batch_execute("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await
        .context("begin backfill snapshot")?;
    let result = backfill_in_txn(client, ts, filter).await;
    client.batch_execute("COMMIT").await.ok();
    result
}

async fn backfill_in_txn(client: &Client, ts: &TableSchema, filter: Option<&CompiledPredicate>) -> Result<Backfill> {
    // Push the shape's predicate into the SELECT so only matching rows are read. Text literals are
    // bound parameters; numeric/bool/null are inlined (see `crate::sql`). The engine still applies
    // `matches()` afterward, so the SQL only needs to be a sound superset filter.
    let where_sql = filter.and_then(|p| crate::sql::predicate_to_sql(p, ts));
    backfill_where_in_txn(client, ts, where_sql).await
}

/// Like [`backfill`], but with a **prebuilt** `WHERE` fragment + params (from the JSON SQL emitter) —
/// used for subquery shapes/nodes, whose `IN (SELECT …)` SQL the compiled-predicate emitter can't
/// reconstruct. `where_sql = None` reads the whole table.
pub async fn backfill_where(
    client: &Client,
    ts: &TableSchema,
    where_sql: Option<(String, Vec<String>)>,
) -> Result<Backfill> {
    client
        .batch_execute("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await
        .context("begin backfill snapshot")?;
    let result = backfill_where_in_txn(client, ts, where_sql).await;
    client.batch_execute("COMMIT").await.ok();
    result
}

async fn backfill_where_in_txn(
    client: &Client,
    ts: &TableSchema,
    where_sql: Option<(String, Vec<String>)>,
) -> Result<Backfill> {
    // One statement establishes the snapshot AND captures both fences (LSN + xid snapshot)
    // atomically with it.
    let fence = client
        .query_one("select pg_current_wal_lsn()::text, pg_current_snapshot()::text", &[])
        .await?;
    let seed_lsn: String = fence.get(0);
    let snap: String = fence.get(1);
    let gate = SnapshotGate::parse(&snap, &seed_lsn);
    let (where_clause, params) = match where_sql {
        Some((w, ps)) => (format!(" where {w}"), ps),
        None => (String::new(), Vec::new()),
    };
    let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
        params.iter().map(|s| s as &(dyn tokio_postgres::types::ToSql + Sync)).collect();
    let q = format!("select {} from {} t{}", row_json_expr(ts), ts.table.quote_qualified(), where_clause);
    let rows =
        client.query(&q, &param_refs).await.with_context(|| format!("backfill select {}", ts.table))?;
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        let j: serde_json::Value = r.get(0);
        let obj = j.as_object().context("backfill row expr did not return an object")?;
        out.push(ts.row_from_json(obj)?);
    }
    Ok(Backfill { rows: out, seed_lsn, gate })
}

/// Group-count seed for a counts pipeline: `SELECT <group cols>, count(*) … GROUP BY` under a
/// `REPEATABLE READ` snapshot — O(distinct groups) rather than O(rows) — with the same
/// visibility fences as a row backfill. Returned rows are full-width with only the group
/// columns populated (the counts pipeline projects exactly those positions); text-mapped
/// columns are cast `::text` for live-path byte identity.
pub async fn backfill_group_counts(
    client: &Client,
    ts: &TableSchema,
    group_cols: &[usize],
) -> Result<(Vec<(Row, i64)>, SnapshotGate)> {
    client
        .batch_execute("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await
        .context("begin counts seed snapshot")?;
    let result = group_counts_in_txn(client, ts, group_cols).await;
    client.batch_execute("COMMIT").await.ok();
    result
}

async fn group_counts_in_txn(
    client: &Client,
    ts: &TableSchema,
    group_cols: &[usize],
) -> Result<(Vec<(Row, i64)>, SnapshotGate)> {
    let fence = client
        .query_one("select pg_current_wal_lsn()::text, pg_current_snapshot()::text", &[])
        .await?;
    let seed_lsn: String = fence.get(0);
    let snap: String = fence.get(1);
    let gate = SnapshotGate::parse(&snap, &seed_lsn);
    let mut args = Vec::new();
    let mut by = Vec::new();
    for &i in group_cols {
        let (name, ty) = ts.columns.get(i).with_context(|| format!("group col {i} out of range"))?;
        let lit = format!("'{}'", name.replace('\'', "''"));
        let qi = quote_ident(name);
        match ty {
            ColumnType::Text => args.push(format!("{lit}, t.{qi}::text")),
            _ => args.push(format!("{lit}, to_jsonb(t.{qi})")),
        }
        by.push(format!("t.{qi}"));
    }
    let q = format!(
        "select jsonb_build_object({}), count(*)::bigint from {} t group by {}",
        args.join(", "),
        ts.table.quote_qualified(),
        by.join(", ")
    );
    let rows = client.query(&q, &[]).await.with_context(|| format!("counts seed select {}", ts.table))?;
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        let j: serde_json::Value = r.get(0);
        let obj = j.as_object().context("counts seed expr did not return an object")?;
        // Missing (non-group) columns default to Null — the pipeline projects only group cols.
        out.push((ts.row_from_json(obj)?, r.get::<_, i64>(1)));
    }
    Ok((out, gate))
}

/// The per-row JSON projection used by backfill and subset query-backs. Text-mapped columns are cast
/// with `::text` so the value is Postgres's *text output* — the same representation `test_decoding`
/// prints on the live path. `to_jsonb(t)` would instead give e.g. ISO-8601 `T`-form timestamps and
/// raw jsonb objects, so the same cell would compare unequal between a backfilled row and its first
/// replicated update (breaking retractions, equality routing, and MIN/MAX multisets). Int/Float/Bool
/// columns keep `to_jsonb` (native JSON scalars, matching the live parser). Chunked into multiple
/// `jsonb_build_object` calls `||`-concatenated to stay under the 100-argument limit.
fn row_json_expr(ts: &TableSchema) -> String {
    let mut objs: Vec<String> = Vec::new();
    for chunk in ts.columns.chunks(40) {
        let args: Vec<String> = chunk
            .iter()
            .map(|(name, ty)| {
                let lit = format!("'{}'", name.replace('\'', "''"));
                let qi = quote_ident(name);
                match ty {
                    ColumnType::Text => format!("{lit}, t.{qi}::text"),
                    _ => format!("{lit}, to_jsonb(t.{qi})"),
                }
            })
            .collect();
        objs.push(format!("jsonb_build_object({})", args.join(", ")));
    }
    objs.join(" || ")
}

/// Result of a one-shot subset query: the page rows + the snapshot LSN they were read at.
pub struct SubsetQuery {
    pub rows: Vec<Row>,
    pub lsn: String,
}

/// Run a **non-materialized** subset query: a single `SELECT … WHERE … ORDER BY … LIMIT … OFFSET …`
/// against Postgres in a `REPEATABLE READ` snapshot, returning the page rows and the snapshot LSN.
/// Unlike [`backfill`], this creates no shape and no durable stream — it is the ephemeral query-back a
/// subset/pagination view uses (the live tail is followed separately). `order` is `(column index,
/// descending?)`; the pk is appended as a tiebreaker so the window is total/stable.
pub async fn query_subset(
    client: &Client,
    ts: &TableSchema,
    filter: Option<&CompiledPredicate>,
    order: Option<(usize, bool)>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<SubsetQuery> {
    query_subset_where(client, ts, filter.and_then(|p| crate::sql::predicate_to_sql(p, ts)), order, limit, offset).await
}

/// Like [`query_subset`], but with a **prebuilt** `WHERE` fragment + params — used when the predicate
/// contains an `IN (SELECT …)` subquery (the JSON SQL emitter builds it; Postgres evaluates it natively,
/// so paginated subquery lists work without engine-side subquery state).
pub async fn query_subset_where(
    client: &Client,
    ts: &TableSchema,
    where_sql: Option<(String, Vec<String>)>,
    order: Option<(usize, bool)>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<SubsetQuery> {
    client
        .batch_execute("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await
        .context("begin subset snapshot")?;
    let result = query_subset_in_txn(client, ts, where_sql, order, limit, offset).await;
    client.batch_execute("COMMIT").await.ok();
    result
}

async fn query_subset_in_txn(
    client: &Client,
    ts: &TableSchema,
    where_sql: Option<(String, Vec<String>)>,
    order: Option<(usize, bool)>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<SubsetQuery> {
    let lsn: String = client.query_one("select pg_current_wal_lsn()::text", &[]).await?.get(0);
    let (where_clause, params) = match where_sql {
        Some((w, ps)) => (format!(" where {w}"), ps),
        None => (String::new(), Vec::new()),
    };
    let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
        params.iter().map(|s| s as &(dyn tokio_postgres::types::ToSql + Sync)).collect();
    // ORDER BY <col> <dir>, <pk> <dir> for a total order; a limit/offset without an explicit order
    // falls back to pk order so the page is deterministic. Idents are quoted; limit/offset are
    // non-negative integer literals — no injection surface.
    let order_sql = match order {
        Some((col, desc)) => {
            let d = if desc { "desc" } else { "asc" };
            format!(" order by {} {d}, {} {d}", quote_ident(&ts.columns[col].0), quote_ident(&ts.pk_name))
        }
        None if limit.is_some() || offset.is_some() => format!(" order by {} asc", quote_ident(&ts.pk_name)),
        None => String::new(),
    };
    let limit_sql = limit.map(|n| format!(" limit {}", n.max(0))).unwrap_or_default();
    let offset_sql = offset.map(|n| format!(" offset {}", n.max(0))).unwrap_or_default();
    let q = format!(
        "select {} from {} t{}{}{}{}",
        row_json_expr(ts),
        ts.table.quote_qualified(),
        where_clause,
        order_sql,
        limit_sql,
        offset_sql
    );
    let rows = client.query(&q, &param_refs).await.with_context(|| format!("subset select {}", ts.table))?;
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        let j: serde_json::Value = r.get(0);
        let obj = j.as_object().context("subset row expr did not return an object")?;
        out.push(ts.row_from_json(obj)?);
    }
    Ok(SubsetQuery { rows: out, lsn })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The snapshot gate must skip exactly the transactions the backfill snapshot could see:
    /// committed-before (`xid < xmin`) → skip; in-progress at snapshot (`xip`) → process; started
    /// after (`xid >= xmax`) → process — regardless of WAL position. This is the fence that closes
    /// the "commit record written but not yet visible" window an LSN comparison cannot express.
    #[test]
    fn snapshot_gate_visibility() {
        // snapshot: xmin 100, xmax 110, in-progress {103, 107}; snapshot LSN 0/100.
        let g = SnapshotGate::parse("100:110:103,107", "0/100");
        // committed before the snapshot -> already in the backfill -> skip
        assert!(g.should_skip(0, Some(99)));
        // between xmin and xmax, not in-progress -> visible -> skip (even if its commit LSN were AT
        // or ABOVE the snapshot LSN, the boundary-duplicate case)
        assert!(g.should_skip(0x200, Some(105)));
        // in-progress at snapshot time -> INVISIBLE to the backfill -> must be processed, even though
        // its commit LSN may be below the snapshot LSN (the dropped-row race the LSN rule had)
        assert!(!g.should_skip(0x50, Some(103)));
        assert!(!g.should_skip(0x50, Some(107)));
        // started after the snapshot -> process
        assert!(!g.should_skip(0x200, Some(110)));
        assert!(!g.should_skip(0x200, Some(200)));
        // no xid -> LSN fallback (strict <)
        assert!(g.should_skip(0x50, None));
        assert!(!g.should_skip(0x100, None));
        // passthrough gate never skips
        let p = SnapshotGate::passthrough();
        assert!(!p.should_skip(0x50, Some(99)));
        assert!(!p.should_skip(0x50, None));
    }

    #[test]
    fn lsn_parse_roundtrip() {
        assert_eq!(lsn_to_u64("0/1A2B3C"), 0x1A2B3C);
        assert_eq!(lsn_to_u64("2/10"), (2u64 << 32) | 0x10);
        assert_eq!(lsn_to_u64("garbage"), 0);
    }
}

/// Parse a Postgres LSN ("X/Y", hex) into a comparable u64. Returns 0 on parse failure.
pub fn lsn_to_u64(lsn: &str) -> u64 {
    match lsn.split_once('/') {
        Some((hi, lo)) => {
            let hi = u64::from_str_radix(hi.trim(), 16).unwrap_or(0);
            let lo = u64::from_str_radix(lo.trim(), 16).unwrap_or(0);
            (hi << 32) | lo
        }
        None => 0,
    }
}
