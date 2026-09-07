//! Multi-source discovery, reconciliation, and source-scoped HTTP serving.
//!
//! The control database/file is the sole desired-state authority. This module deliberately keeps
//! no durable source registry: the desired rows are fetched again on refresh and the poll only reads
//! the version row when its revision has not changed.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Path as AxumPath, Request, State};
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, oneshot};
use tower::ServiceExt;

use crate::config::{Config, SourcesConfig, SourcesMode};
use crate::ds::DsClient;
use crate::engine::{Engine, PostgresSetup};
use crate::shutdown::ShutdownToken;
use crate::store_identity::{StoreBound, StreamScope};
use crate::table_ref::TableSelector;

/// One row from the control-plane sources relation or its JSON file equivalent.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SourceRow {
    pub source_id: String,
    pub plugin: String,
    pub database_secret: String,
    pub slot: String,
    pub publication: String,
    pub tables: Vec<String>,
    pub revision: i64,
    pub updated_at: String,
}

/// The subset of a running source needed by the pure reconciliation law.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningSourceState {
    pub revision: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanAction {
    Start(SourceRow),
    Stop(String),
    Restart(SourceRow),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub actions: Vec<PlanAction>,
}

/// Compute the idempotent source lifecycle plan for one fetched row set.
pub fn reconcile(running: &BTreeMap<String, RunningSourceState>, rows: &[SourceRow]) -> Plan {
    let desired: BTreeMap<&str, &SourceRow> = rows.iter().map(|row| (row.source_id.as_str(), row)).collect();
    let mut actions = Vec::new();
    for (source_id, state) in running {
        match desired.get(source_id.as_str()) {
            None => actions.push(PlanAction::Stop(source_id.clone())),
            Some(row) if row.revision != state.revision => actions.push(PlanAction::Restart((*row).clone())),
            Some(_) => {}
        }
    }
    for (source_id, row) in desired {
        if !running.contains_key(source_id) {
            actions.push(PlanAction::Start(row.clone()));
        }
    }
    Plan { actions }
}

#[derive(Clone)]
pub struct SourcesSupervisor {
    inner: Arc<SupervisorInner>,
}

struct SupervisorInner {
    config: Config,
    sources: SourcesConfig,
    state: Mutex<SupervisorState>,
    reconcile: Mutex<()>,
    shutdown: ShutdownToken,
}

struct SupervisorState {
    running: BTreeMap<String, SourceRuntime>,
    failed: BTreeMap<String, FailedSource>,
    last_revision: Option<i64>,
    fetched: bool,
}

struct SourceRuntime {
    row: SourceRow,
    engine: Engine,
    router: Router,
    worker: Option<std::thread::JoinHandle<()>>,
}

#[derive(Clone)]
struct FailedSource {
    row: SourceRow,
    error: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct SourceSummary {
    pub source_id: String,
    pub revision: i64,
    pub ready: bool,
    pub error: Option<String>,
}

struct SourceSnapshot {
    revision: i64,
    rows: Vec<SourceRow>,
}

impl SourcesSupervisor {
    pub fn new(config: Config) -> Result<Self> {
        let sources = config.sources.clone().context("sources supervisor requires SOURCES_MODE")?;
        if matches!(sources.mode, SourcesMode::Table) && sources.pg_url.is_none() {
            bail!("sources table mode requires a control database URL");
        }
        Ok(Self {
            inner: Arc::new(SupervisorInner {
                config,
                sources,
                state: Mutex::new(SupervisorState {
                    running: BTreeMap::new(),
                    failed: BTreeMap::new(),
                    last_revision: None,
                    fetched: false,
                }),
                reconcile: Mutex::new(()),
                shutdown: ShutdownToken::new(),
            }),
        })
    }

    /// Build the host router. The source router is forwarded with a rewritten URI rather than
    /// nested: Axum's nested path captures otherwise leak into the engine's own Path extractors.
    pub fn router(&self) -> Router {
        let state = self.clone();
        Router::new()
            .route("/health", axum::routing::get(health))
            .route("/ready", axum::routing::get(ready))
            .route("/sources", axum::routing::get(list_sources))
            .route("/sources/{source_id}/status", axum::routing::get(source_status))
            .route("/sources/{source_id}", axum::routing::any(proxy_root))
            .route("/sources/{source_id}/{*path}", axum::routing::any(proxy_path))
            .route("/admin/refresh", axum::routing::post(refresh))
            .with_state(state)
    }

    pub fn shutdown_token(&self) -> ShutdownToken {
        self.inner.shutdown.clone()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub async fn with_test_source(config: Config, row: SourceRow, engine: Engine) -> Result<Self> {
        let supervisor = Self::new(config)?;
        let router = crate::http::router_with_introspection(engine.clone(), false);
        {
            let mut state = supervisor.inner.state.lock().await;
            state.running.insert(row.source_id.clone(), SourceRuntime { row, engine, router, worker: None });
            state.last_revision = state.running.values().map(|runtime| runtime.row.revision).max();
            state.fetched = true;
        }
        Ok(supervisor)
    }

    pub async fn initial_fetch_until_ready(&self) -> Result<()> {
        let mut attempt = 0u32;
        loop {
            match self.refresh().await {
                Ok(_) => return Ok(()),
                Err(error) => {
                    attempt = attempt.saturating_add(1);
                    let delay =
                        crate::replication::backoff_base(attempt.saturating_sub(1)).min(Duration::from_secs(30));
                    tracing::warn!(attempt, delay = ?delay, error = %error, "sources discovery unavailable; retrying");
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = self.inner.shutdown.wait() => {
                            bail!("sources discovery interrupted by shutdown");
                        }
                    }
                }
            }
        }
    }

    /// Fetch and reconcile unconditionally. The lock covers the fetch and all lifecycle changes,
    /// so concurrent refreshes cannot interleave plans.
    pub async fn refresh(&self) -> Result<i64> {
        let _guard = self.inner.reconcile.lock().await;
        let snapshot = self.fetch_snapshot().await?;
        self.apply_snapshot(snapshot).await
    }

    pub fn spawn_poll(&self) {
        let supervisor = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(supervisor.inner.sources.poll);
            interval.tick().await;
            loop {
                tokio::select! {
                    _ = supervisor.inner.shutdown.wait() => break,
                    _ = interval.tick() => {
                        if let Err(error) = supervisor.poll_once().await {
                            tracing::warn!(error = %error, "sources poll failed");
                        }
                    }
                }
            }
        });
    }

    pub async fn shutdown_all(&self) {
        let _guard = self.inner.reconcile.lock().await;
        let runtimes = {
            let mut state = self.inner.state.lock().await;
            std::mem::take(&mut state.running).into_values().collect::<Vec<_>>()
        };
        for mut runtime in runtimes {
            runtime.engine.shutdown_token().begin();
            if let Some(worker) = runtime.worker.take() {
                if let Err(error) = join_worker(worker).await {
                    tracing::warn!(source_id = %runtime.row.source_id, error = %error, "source worker did not join cleanly");
                }
            }
        }
    }

    async fn poll_once(&self) -> Result<()> {
        match self.inner.sources.mode {
            SourcesMode::File => {
                self.refresh().await?;
            }
            SourcesMode::Table => {
                let revision = self.read_control_revision().await?;
                let unchanged = self.inner.state.lock().await.last_revision == Some(revision);
                if !unchanged {
                    self.refresh().await?;
                }
            }
        }
        Ok(())
    }

    async fn fetch_snapshot(&self) -> Result<SourceSnapshot> {
        match self.inner.sources.mode {
            SourcesMode::File => {
                let path = self.inner.sources.file.as_deref().context("file mode has no sources file")?;
                let body = tokio::fs::read_to_string(path).await.with_context(|| format!("read {}", path.display()))?;
                let rows: Vec<SourceRow> = serde_json::from_str(&body).context("parse sources JSON")?;
                validate_rows(&rows)?;
                let revision = rows.iter().map(|row| row.revision).max().unwrap_or(0);
                Ok(SourceSnapshot { revision, rows })
            }
            SourcesMode::Table => {
                let revision = self.read_control_revision().await?;
                let url = self.inner.sources.pg_url.as_deref().context("table mode has no control URL")?;
                let client = crate::pg::connect(url).await.context("connect sources control database")?;
                let query = format!(
                    "SELECT source_id, plugin, database_secret, slot, publication, tables, revision, updated_at::text \
                     FROM {} ORDER BY source_id",
                    quote_table_name(&self.inner.sources.table)
                );
                let rows = client.query(&query, &[]).await.context("read sources table")?;
                let rows = rows.into_iter().map(source_row_from_pg).collect::<Result<Vec<_>>>()?;
                validate_rows(&rows)?;
                Ok(SourceSnapshot { revision, rows })
            }
        }
    }

    async fn read_control_revision(&self) -> Result<i64> {
        let url = self.inner.sources.pg_url.as_deref().context("table mode has no control URL")?;
        let client = crate::pg::connect(url).await.context("connect sources version database")?;
        let query = format!("SELECT revision FROM {} LIMIT 1", quote_table_name(&self.inner.sources.version_table));
        let row = client.query_opt(&query, &[]).await.context("read sources version")?;
        row.map(|row| row.get(0)).context("sources version table has no revision row")
    }

    async fn apply_snapshot(&self, snapshot: SourceSnapshot) -> Result<i64> {
        let running = {
            let state = self.inner.state.lock().await;
            state
                .running
                .iter()
                .map(|(source_id, runtime)| (source_id.clone(), RunningSourceState { revision: runtime.row.revision }))
                .collect::<BTreeMap<_, _>>()
        };
        let plan = reconcile(&running, &snapshot.rows);
        for action in plan.actions {
            match action {
                PlanAction::Stop(source_id) => {
                    if let Some(mut runtime) = self.inner.state.lock().await.running.remove(&source_id) {
                        if let Err(error) = stop_runtime(&mut runtime).await {
                            tracing::warn!(source_id = %source_id, error = %error, "source stop failed");
                        }
                    }
                    self.inner.state.lock().await.failed.remove(&source_id);
                }
                PlanAction::Restart(row) => {
                    if let Some(mut runtime) = self.inner.state.lock().await.running.remove(&row.source_id) {
                        self.inner.state.lock().await.failed.insert(
                            row.source_id.clone(),
                            FailedSource { row: row.clone(), error: "source restarting".to_string() },
                        );
                        if let Err(error) = stop_runtime(&mut runtime).await {
                            tracing::warn!(source_id = %row.source_id, error = %error, "source restart stop failed");
                        }
                    }
                    self.start_or_record(row).await;
                }
                PlanAction::Start(row) => {
                    self.start_or_record(row).await;
                }
            }
        }
        let mut state = self.inner.state.lock().await;
        let desired_ids =
            snapshot.rows.iter().map(|row| row.source_id.as_str()).collect::<std::collections::BTreeSet<_>>();
        state.failed.retain(|source_id, _| desired_ids.contains(source_id.as_str()));
        state.last_revision = Some(snapshot.revision);
        state.fetched = true;
        Ok(snapshot.revision)
    }

    async fn start_or_record(&self, row: SourceRow) {
        match self.start_source(row.clone()).await {
            Ok(runtime) => {
                let mut state = self.inner.state.lock().await;
                state.failed.remove(&row.source_id);
                state.running.insert(row.source_id.clone(), runtime);
            }
            Err(error) => {
                tracing::warn!(
                    source_id = %row.source_id,
                    revision = row.revision,
                    error = %error,
                    "source is not ready"
                );
                self.inner
                    .state
                    .lock()
                    .await
                    .failed
                    .insert(row.source_id.clone(), FailedSource { row, error: format!("{error:#}") });
            }
        }
    }

    async fn start_source(&self, row: SourceRow) -> Result<SourceRuntime> {
        validate_source_row(&row)?;
        let config = self.inner.config.clone();
        let grace = config.shutdown_grace;
        let source_id = row.source_id.clone();
        let row_for_thread = row.clone();
        let (ready_tx, ready_rx) = oneshot::channel();
        let thread_name = format!("circuits-source-{}", source_id);
        let worker = std::thread::Builder::new()
            .name(thread_name.clone())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = ready_tx.send(Err(anyhow::anyhow!("build source runtime: {error}")));
                        return;
                    }
                };
                let booted = runtime.block_on(boot_source(config, row_for_thread));
                match booted {
                    Ok((engine, router)) => {
                        let shutdown = engine.shutdown_token();
                        if ready_tx.send(Ok((engine, router))).is_err() {
                            shutdown.begin();
                        }
                        runtime.block_on(async move {
                            shutdown.wait().await;
                            if !shutdown.wait_for_parties(grace).await {
                                tracing::warn!("source shutdown grace elapsed with work still in flight");
                            }
                        });
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                    }
                }
            })
            .with_context(|| format!("spawn {thread_name}"))?;
        let result = ready_rx.await.with_context(|| format!("{thread_name} exited before readiness"))?;
        match result {
            Ok((engine, router)) => Ok(SourceRuntime { row, engine, router, worker: Some(worker) }),
            Err(error) => {
                join_worker(worker).await?;
                Err(error)
            }
        }
    }
}

pub async fn run(config: Config) -> Result<()> {
    crate::config::set_globals(
        &config.instance_id,
        &config.stack_id,
        config.secret.as_deref(),
        config.control_secret.as_deref(),
    );
    crate::pg::set_pool_size(config.db_pool_size);
    crate::pg::set_backfill_config(config.backfill);
    if let Some(target) = &config.statsd {
        crate::statsd::init(target, &config.instance_id);
    }

    let supervisor = SourcesSupervisor::new(config.clone())?;
    supervisor.initial_fetch_until_ready().await?;
    let listener =
        tokio::net::TcpListener::bind(&config.bind).await.with_context(|| format!("binding {}", config.bind))?;
    let address = listener.local_addr()?;
    println!("ENGINE_BINDING http://{address}");
    println!("ENGINE_LISTENING http://{address}");
    let poll_supervisor = supervisor.clone();
    poll_supervisor.spawn_poll();
    let shutdown_supervisor = supervisor.clone();
    axum::serve(listener, supervisor.router())
        .with_graceful_shutdown(async move {
            let signal = crate::shutdown::first_signal().await;
            tracing::info!(signal, "sources host shutdown requested");
            shutdown_supervisor.shutdown_token().begin();
            shutdown_supervisor.shutdown_all().await;
        })
        .await
        .context("serve sources host")?;
    Ok(())
}

async fn join_worker(worker: std::thread::JoinHandle<()>) -> Result<()> {
    tokio::task::spawn_blocking(move || worker.join().map_err(|_| anyhow::anyhow!("source worker panicked")))
        .await
        .context("join source worker task")??;
    Ok(())
}

async fn stop_runtime(runtime: &mut SourceRuntime) -> Result<()> {
    runtime.engine.shutdown_token().begin();
    if let Some(worker) = runtime.worker.take() {
        join_worker(worker).await?;
    }
    Ok(())
}

async fn boot_source(config: Config, row: SourceRow) -> Result<(Engine, Router)> {
    let database_url = resolve_database_secret(&row.database_secret).await?;
    let child = source_engine_config(&config, &row, &database_url)?;
    child.txn.probe().context("probe source transaction spill directory")?;
    let (ds, scope) = source_ds(&config, &row.source_id).await?;
    let admission = Engine::admit_store(ds, StoreBound::coupled_v1(&scope), config.initialize_namespace).await?;
    let engine = Engine::new_pg_with_setup(admission, database_url, PostgresSetup::ExternallyManaged);
    engine.set_dbsp_config(child.dbsp.clone());
    engine.set_txn_config(child.txn.clone());
    engine
        .setup_postgres(&child.tables, &row.slot)
        .await
        .with_context(|| format!("activate source {}", row.source_id))?;
    if engine.readiness_status() != "active" {
        bail!("source {} did not become active", row.source_id);
    }
    Ok((engine.clone(), crate::http::router_with_introspection(engine, child.trace)))
}

fn source_engine_config(base: &Config, row: &SourceRow, database_url: &str) -> Result<Config> {
    let sources = base.sources.as_ref().context("source config missing")?;
    let source_root = sources.storage_dir.join(&row.source_id);
    let pg_url = database_url.to_string();
    let slot = row.slot.clone();
    let tables = row.tables.join(",");
    let mut child = Config::resolve(|name| match name {
        "ELECTRIC_CIRCUITS_SOURCES_MODE"
        | "ELECTRIC_CIRCUITS_SOURCES_PG_URL"
        | "ELECTRIC_CIRCUITS_SOURCES_TABLE"
        | "ELECTRIC_CIRCUITS_SOURCES_VERSION_TABLE"
        | "ELECTRIC_CIRCUITS_SOURCES_POLL_SECS"
        | "ELECTRIC_CIRCUITS_SOURCES_FILE"
        | "ELECTRIC_CIRCUITS_SOURCES_STORAGE_DIR" => None,
        "ELECTRIC_CIRCUITS_PG_URL" => Some(pg_url.clone()),
        "ELECTRIC_CIRCUITS_PG_SLOT" => Some(slot.clone()),
        "ELECTRIC_CIRCUITS_PG_TABLES" => Some(tables.clone()),
        "ELECTRIC_STORAGE_DIR" => Some(source_root.to_string_lossy().into_owned()),
        "ELECTRIC_CIRCUITS_DBSP_DIR" => Some(source_root.join("dbsp").to_string_lossy().into_owned()),
        "ELECTRIC_CIRCUITS_TXN_SPILL_DIR" => Some(source_root.join("txn-spill").to_string_lossy().into_owned()),
        "ELECTRIC_CIRCUITS_DS_URL" => base.ds_url.clone(),
        "ELECTRIC_CIRCUITS_DS_IN_PROCESS_TEST" => base.ds_in_process_test_url.as_ref().map(|_| "1".to_string()),
        "ELECTRIC_CIRCUITS_INITIALIZE_NAMESPACE" => base.initialize_namespace.then_some("1".to_string()),
        _ => std::env::var(name).ok(),
    })?;
    child.sources = None;
    child.dbsp = base.dbsp.clone();
    child.dbsp.dir = source_root.join("dbsp");
    child.txn = base.txn.clone();
    child.txn.spill_dir = source_root.join("txn-spill");
    child.backfill = base.backfill;
    child.db_pool_size = base.db_pool_size;
    child.trace = base.trace;
    child.secret = base.secret.clone();
    child.control_secret = base.control_secret.clone();
    child.shutdown_grace = base.shutdown_grace;
    child.shutdown_ready_drain = base.shutdown_ready_drain;
    child.storage_dir = Some(source_root.to_string_lossy().into_owned());
    child.stack_id = row.source_id.clone();
    Ok(child)
}

async fn source_ds(config: &Config, source_id: &str) -> Result<(DsClient, StreamScope)> {
    if let Some(connection) = config.ds_connection.as_ref() {
        let scope = source_scope(&connection.scope, source_id)?;
        let connection = crate::ds::DsConnectionConfig::new(
            connection.base_url.clone(),
            connection.ca_bundle_path.clone(),
            connection.client_certificate_path.clone(),
            connection.client_key_path.clone(),
            scope.clone(),
        )?;
        return Ok((DsClient::connect(connection).await?, scope));
    }
    if let Some(base_url) = config.ds_in_process_test_url.as_ref() {
        #[cfg(any(test, feature = "test-support"))]
        {
            let scope = source_scope(&StreamScope::in_process_test_scope(), source_id)?;
            return Ok((DsClient::new_for_in_process_test_with_scope(base_url.clone(), scope.clone()), scope));
        }
        #[cfg(not(any(test, feature = "test-support")))]
        {
            let _ = (base_url, source_id);
            bail!("sources mode with an in-process Durable Streams URL requires test-support");
        }
    }
    bail!("sources mode requires a Durable Streams configuration")
}

fn source_scope(base: &StreamScope, source_id: &str) -> Result<StreamScope> {
    use sha2::{Digest, Sha256};
    let digest = format!("{:x}", Sha256::digest(source_id.as_bytes()));
    StreamScope::new(format!("source-{}", &digest[..40]), base.store.clone(), format!("query-{}", &digest[..40]))
}

async fn resolve_database_secret(reference: &str) -> Result<String> {
    let (kind, name) =
        reference.split_once(':').with_context(|| format!("database_secret has no resolver prefix: {reference}"))?;
    if name.trim().is_empty() {
        bail!("database_secret resolver name is empty");
    }
    let value = match kind {
        "env" => return resolve_env_secret(reference, |key| std::env::var(key).ok()),
        "file" => tokio::fs::read_to_string(name).await.with_context(|| format!("read database secret file {name}"))?,
        "aws-sm" => return resolve_aws_secret(name).await,
        other => bail!("unknown database_secret resolver '{other}'"),
    };
    validate_database_secret_value(&value)
}

async fn resolve_aws_secret(name: &str) -> Result<String> {
    resolve_aws_secret_with(name, |name| async move {
        let shared = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        let client = aws_sdk_secretsmanager::Client::new(&shared);
        let display_name = name.clone();
        client
            .get_secret_value()
            .secret_id(name)
            .send()
            .await
            .with_context(|| format!("read AWS Secrets Manager secret {display_name}"))?
            .secret_string()
            .context("AWS Secrets Manager secret has no SecretString")
            .map(ToString::to_string)
    })
    .await
}

async fn resolve_aws_secret_with<F, Fut>(name: &str, fetch: F) -> Result<String>
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<String>>,
{
    let value = fetch(name.to_string()).await?;
    validate_database_secret_value(&value)
}

fn validate_database_secret_value(value: &str) -> Result<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        bail!("database secret resolved to an empty value");
    }
    validate_database_url(&value)?;
    Ok(value)
}

fn resolve_env_secret(reference: &str, get: impl Fn(&str) -> Option<String>) -> Result<String> {
    let (kind, name) =
        reference.split_once(':').with_context(|| format!("database_secret has no resolver prefix: {reference}"))?;
    if kind != "env" || name.trim().is_empty() {
        bail!("database_secret must use env:NAME for an environment lookup");
    }
    let value = get(name).with_context(|| format!("environment secret {name} is missing"))?;
    let value = value.trim().to_string();
    if value.is_empty() {
        bail!("environment secret {name} is empty");
    }
    validate_database_url(&value)?;
    Ok(value)
}

fn validate_database_url(value: &str) -> Result<()> {
    let ca_bundle = std::env::var("ELECTRIC_CIRCUITS_PG_TLS_CA_BUNDLE").ok();
    let server_name = std::env::var("ELECTRIC_CIRCUITS_PG_TLS_SERVER_NAME").ok();
    crate::pg::PgConnectionConfig::resolve(value, ca_bundle.as_deref(), server_name.as_deref())
        .context("resolved database secret is not a valid Postgres URL")?;
    Ok(())
}

fn validate_rows(rows: &[SourceRow]) -> Result<()> {
    let mut source_ids = std::collections::BTreeSet::new();
    for row in rows {
        if !source_ids.insert(row.source_id.as_str()) {
            bail!("sources discovery returned duplicate source_id '{}'", row.source_id);
        }
    }
    Ok(())
}

fn validate_source_row(row: &SourceRow) -> Result<()> {
    if row.source_id.trim().is_empty()
        || row.source_id.contains('/')
        || row.source_id.contains('?')
        || row.source_id.contains('#')
        || row.source_id.contains('%')
    {
        bail!("source_id '{}' cannot be represented safely in a source route", row.source_id);
    }
    if row.plugin != crate::pg::PGOUTPUT {
        bail!("source '{}' uses unsupported logical-replication plugin '{}'", row.source_id, row.plugin);
    }
    if row.slot.trim().is_empty() {
        bail!("source '{}' has an empty slot", row.source_id);
    }
    if row.publication != format!("{}_pub", row.slot) {
        bail!(
            "source '{}' publication '{}' must match the externally managed slot publication '{}_pub'",
            row.source_id,
            row.publication,
            row.slot
        );
    }
    if row.tables.is_empty() {
        bail!("source '{}' has no tables", row.source_id);
    }
    for table in &row.tables {
        if !table.contains('.') || table.contains('*') {
            bail!("source '{}' table '{}' must be schema-qualified", row.source_id, table);
        }
        TableSelector::parse(table).with_context(|| format!("source '{}' table '{}'", row.source_id, table))?;
    }
    Ok(())
}

fn source_row_from_pg(row: tokio_postgres::Row) -> Result<SourceRow> {
    Ok(SourceRow {
        source_id: row.try_get(0).context("sources source_id")?,
        plugin: row.try_get(1).context("sources plugin")?,
        database_secret: row.try_get(2).context("sources database_secret")?,
        slot: row.try_get(3).context("sources slot")?,
        publication: row.try_get(4).context("sources publication")?,
        tables: row.try_get(5).context("sources tables")?,
        revision: row.try_get(6).context("sources revision")?,
        updated_at: row.try_get(7).context("sources updated_at")?,
    })
}

fn quote_table_name(name: &str) -> String {
    name.split('.').map(|part| format!("\"{}\"", part.replace('\"', "\"\""))).collect::<Vec<_>>().join(".")
}

async fn health() -> Response {
    StatusCode::OK.into_response()
}

async fn ready(State(supervisor): State<SourcesSupervisor>) -> Response {
    let fetched = supervisor.inner.state.lock().await.fetched;
    let status = if fetched { "active" } else { "waiting" };
    let code = if fetched { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    (code, axum::Json(serde_json::json!({ "status": status }))).into_response()
}

async fn list_sources(State(supervisor): State<SourcesSupervisor>) -> axum::Json<Vec<SourceSummary>> {
    axum::Json(supervisor.summaries().await)
}

async fn source_status(State(supervisor): State<SourcesSupervisor>, AxumPath(source_id): AxumPath<String>) -> Response {
    let state = supervisor.inner.state.lock().await;
    if let Some(runtime) = state.running.get(&source_id) {
        return axum::Json(runtime_status(&runtime.row, &runtime.engine)).into_response();
    }
    if let Some(failed) = state.failed.get(&source_id) {
        return axum::Json(failed_status(&failed.row, &failed.error)).into_response();
    }
    (StatusCode::NOT_FOUND, "source not found").into_response()
}

async fn refresh(
    State(supervisor): State<SourcesSupervisor>,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> Result<axum::Json<serde_json::Value>, crate::http::AppError> {
    crate::http::require_private_admin_with_secret(&headers, crate::config::control_secret())?;
    if !body.is_empty() {
        return Err(crate::http::AppError {
            status: StatusCode::BAD_REQUEST,
            msg: "POST /admin/refresh does not accept a request body".to_string(),
            retry_after: false,
        });
    }
    let revision = supervisor.refresh().await?;
    Ok(axum::Json(serde_json::json!({ "revision": revision })))
}

async fn proxy_root(
    State(supervisor): State<SourcesSupervisor>,
    AxumPath(source_id): AxumPath<String>,
    request: Request,
) -> Response {
    proxy(supervisor, source_id, String::new(), request).await
}

async fn proxy_path(
    State(supervisor): State<SourcesSupervisor>,
    AxumPath((source_id, path)): AxumPath<(String, String)>,
    request: Request,
) -> Response {
    proxy(supervisor, source_id, path, request).await
}

async fn proxy(supervisor: SourcesSupervisor, source_id: String, path: String, request: Request) -> Response {
    let router = {
        let state = supervisor.inner.state.lock().await;
        state.running.get(&source_id).map(|runtime| runtime.router.clone())
    };
    let Some(router) = router else {
        return (StatusCode::NOT_FOUND, "source not found").into_response();
    };
    let query = request.uri().query().map(|query| format!("?{query}")).unwrap_or_default();
    let target = format!("/{}{}", path.trim_start_matches('/'), query);
    let Ok(uri) = target.parse::<Uri>() else {
        return (StatusCode::BAD_REQUEST, "invalid source request URI").into_response();
    };
    let (parts, body) = request.into_parts();
    let mut forwarded = axum::http::Request::new(Body::new(body));
    *forwarded.method_mut() = parts.method;
    *forwarded.uri_mut() = uri;
    *forwarded.version_mut() = parts.version;
    *forwarded.headers_mut() = parts.headers;
    router.oneshot(forwarded).await.unwrap_or_else(|never| match never {})
}

impl SourcesSupervisor {
    async fn summaries(&self) -> Vec<SourceSummary> {
        let state = self.inner.state.lock().await;
        let mut result = state
            .running
            .values()
            .map(|runtime| {
                let readiness = runtime.engine.readiness_status();
                SourceSummary {
                    source_id: runtime.row.source_id.clone(),
                    revision: runtime.row.revision,
                    ready: readiness == "active",
                    error: (readiness != "active").then(|| readiness.to_string()),
                }
            })
            .collect::<Vec<_>>();
        result.extend(state.failed.values().map(|failed| SourceSummary {
            source_id: failed.row.source_id.clone(),
            revision: failed.row.revision,
            ready: false,
            error: Some(failed.error.clone()),
        }));
        result.sort_by(|left, right| left.source_id.cmp(&right.source_id));
        result
    }
}

fn runtime_status(row: &SourceRow, engine: &Engine) -> serde_json::Value {
    let position = engine.changes_position();
    let segments = engine.changes_segments();
    let epoch = engine.epoch_json();
    let consumers = engine
        .consumers()
        .into_iter()
        .map(|(id, position, lag_segments)| {
            let pinned_segments =
                segments.keys().copied().filter(|segment| *segment >= position.segment).collect::<Vec<_>>();
            serde_json::json!({
                "id": id,
                "pinned_segments": pinned_segments,
                "position": position,
                "lagSegments": lag_segments,
            })
        })
        .collect::<Vec<_>>();
    let readiness = engine.readiness_status();
    serde_json::json!({
        "changes": {
            "route": format!("/sources/{}/changes", row.source_id),
            "epoch": engine.changes_generation(),
            "position": position,
            "segments": segments,
        },
        "epoch": epoch,
        "position": position,
        "segments": segments,
        "consumers": consumers,
        "readiness": readiness,
        "revision": row.revision,
        "ready": readiness == "active",
        "error": if readiness == "active" { serde_json::Value::Null } else { serde_json::Value::String(readiness.to_string()) },
    })
}

fn failed_status(row: &SourceRow, error: &str) -> serde_json::Value {
    serde_json::json!({
        "changes": { "route": format!("/sources/{}/changes", row.source_id) },
        "epoch": serde_json::Value::Null,
        "position": serde_json::Value::Null,
        "segments": {},
        "consumers": [],
        "readiness": "not_ready",
        "revision": row.revision,
        "ready": false,
        "error": error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(source_id: &str, revision: i64) -> SourceRow {
        SourceRow {
            source_id: source_id.into(),
            plugin: "pgoutput".into(),
            database_secret: format!("env:{}", source_id),
            slot: format!("{}_slot", source_id),
            publication: format!("{}_pub", source_id),
            tables: vec!["public.thread_messages".into()],
            revision,
            updated_at: "2026-09-07T00:00:00Z".into(),
        }
    }

    fn running(entries: &[(&str, i64)]) -> BTreeMap<String, RunningSourceState> {
        entries
            .iter()
            .map(|(source_id, revision)| (source_id.to_string(), RunningSourceState { revision: *revision }))
            .collect()
    }

    #[test]
    fn identical_rows_have_an_empty_plan() {
        let rows = vec![row("alpha", 1), row("beta", 4)];
        assert_eq!(reconcile(&running(&[("alpha", 1), ("beta", 4)]), &rows), Plan::default());
    }

    #[test]
    fn reconcile_plans_start_stop_restart_and_mixed_changes() {
        let rows = vec![row("alpha", 2), row("new", 1)];
        let plan = reconcile(&running(&[("alpha", 1), ("gone", 7)]), &rows);
        assert_eq!(
            plan.actions,
            vec![
                PlanAction::Restart(row("alpha", 2)),
                PlanAction::Stop("gone".into()),
                PlanAction::Start(row("new", 1)),
            ]
        );
    }

    #[test]
    fn applying_a_plan_then_reconciling_is_empty() {
        let rows = vec![row("alpha", 2), row("new", 1)];
        let plan = reconcile(&running(&[("alpha", 1), ("gone", 7)]), &rows);
        let mut after = running(&[("alpha", 1), ("gone", 7)]);
        for action in &plan.actions {
            match action {
                PlanAction::Start(row) | PlanAction::Restart(row) => {
                    after.insert(row.source_id.clone(), RunningSourceState { revision: row.revision });
                }
                PlanAction::Stop(source_id) => {
                    after.remove(source_id);
                }
            }
        }
        assert_eq!(reconcile(&after, &rows), Plan::default());
    }

    #[test]
    fn environment_secret_resolves_and_missing_or_malformed_values_fail() {
        let url = resolve_env_secret("env:SOURCE_URL", |name| {
            (name == "SOURCE_URL").then(|| " postgres://postgres@127.0.0.1:5432/source ".to_string())
        })
        .expect("environment secret");
        assert_eq!(url, "postgres://postgres@127.0.0.1:5432/source");
        assert!(resolve_env_secret("env:MISSING", |_| None).is_err());
        assert!(resolve_env_secret("env:BROKEN", |_| Some("not a postgres URL".into())).is_err());
        assert!(
            resolve_env_secret("vault:SOURCE_URL", |_| { Some("postgres://postgres@127.0.0.1:5432/source".into()) })
                .is_err()
        );
    }

    #[tokio::test]
    async fn file_secret_is_trimmed_and_invalid_or_missing_files_fail() {
        let path = std::env::temp_dir().join(format!("circuits-source-secret-{}", uuid::Uuid::new_v4()));
        std::fs::write(&path, "\npostgres://postgres@127.0.0.1:5432/source\n").unwrap();
        let resolved = resolve_database_secret(&format!("file:{}", path.display())).await.unwrap();
        assert_eq!(resolved, "postgres://postgres@127.0.0.1:5432/source");
        std::fs::write(&path, "not a postgres URL").unwrap();
        assert!(resolve_database_secret(&format!("file:{}", path.display())).await.is_err());
        std::fs::remove_file(&path).unwrap();
        assert!(resolve_database_secret(&format!("file:{}", path.display())).await.is_err());
    }

    #[tokio::test]
    async fn aws_secret_resolution_uses_a_stub_without_contacting_aws() {
        let value = resolve_aws_secret_with("unit-test-secret", |name| async move {
            assert_eq!(name, "unit-test-secret");
            Ok(" postgres://postgres@127.0.0.1:5432/source ".into())
        })
        .await
        .unwrap();
        assert_eq!(value, "postgres://postgres@127.0.0.1:5432/source");
        assert!(resolve_aws_secret_with("bad", |_| async { Ok("not-a-url".into()) }).await.is_err());
        assert!(
            resolve_aws_secret_with("missing", |_| async { Err(anyhow::anyhow!("stubbed lookup failed")) })
                .await
                .is_err()
        );
    }
}
