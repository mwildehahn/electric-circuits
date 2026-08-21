//! Durable shape catalog: the append-only `meta/catalog` event stream, its writer
//! task, and boot-time restore/resume of shapes.

use super::*;

/// The engine's durable **shape catalog**: an append-only event stream replayed at boot so a
/// restart re-registers every shape itself instead of requiring a client re-registration storm.
/// Plain/routed shapes resume with passthrough gates (the change log replays everything after the
/// persisted offset; re-emission across the crash window is idempotent absolute upserts);
/// aggregates re-seed their fold from a fresh Postgres snapshot (their fresh gate then skips the
/// replayed history). Subquery shapes are NOT restorable without persisted inner-node state (a
/// fresh-seeded node cannot detect downtime flips, which would leave stale move-outs forever) —
/// they are dropped loudly at restore for clients to recreate.
pub(crate) const CATALOG_STREAM: &str = "meta/catalog";

/// One catalog event. `Offset` checkpoints the sequencer's processed change-log position (the
/// replay start after a restart), appended at most every ~2s.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
pub(crate) enum CatalogEvent {
    Created { rec: ShapeRecord, sig: Option<String> },
    /// A subscriber joined a shared feed (refcount +1).
    Joined { id: String },
    /// A subscriber left a shared feed (refcount −1). With retention, reaching refcount 0 keeps
    /// the shape (it goes dormant later), so `Left` never implies teardown.
    Left { id: String },
    /// The shape went dormant: routing state dropped, stream + record retained. `resume_offset`
    /// is the change-log position its stream is complete up to; `gate` is its original
    /// backfill-snapshot fence. Restores as dormant (an improvement over the in-memory-only
    /// lifecycle: a restart no longer forgets dormant shapes).
    Dormant { id: String, resume_offset: String, gate: crate::pg::SnapshotGate },
    /// A dormant shape was reactivated (replayed + re-registered).
    Reactivated { id: String },
    Dropped { id: String },
    Offset { offset: String },
    /// **Audit only**: a table's schema drifted and was re-introspected (ADR-0005). The restore
    /// ignores it — every dependent shape of the table was retired by the same handler, so it is
    /// already `Dropped` in the log. It is written so the durable record explains *why* a swathe of
    /// shapes disappeared at a given point.
    SchemaChanged { table: TableRef, fingerprint: crate::schema::SchemaFingerprint },
}

/// The catalog writer's ordered channel, plus the count of events sent but not yet appended.
///
/// The counter exists for one caller: the circuit-tier drift path exits the process, and it must
/// not do so with its own `Dropped`/`SchemaChanged` events still in the queue — a restart would
/// then restore shapes whose streams it had just deleted (see [`CatalogWriter::drain`]).
#[derive(Clone)]
pub(crate) struct CatalogWriter {
    tx: mpsc::UnboundedSender<CatalogEvent>,
    in_flight: Arc<std::sync::atomic::AtomicI64>,
}

impl CatalogWriter {
    /// Enqueue an event. Infallible by design: a dead writer means the process is going away, and
    /// no caller has a better answer than continuing (the previous code spelled this `let _ =`).
    pub(crate) fn send(&self, ev: CatalogEvent) {
        self.in_flight.fetch_add(1, Ordering::SeqCst);
        if self.tx.send(ev).is_err() {
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Wait until every event sent so far has been appended (or `timeout` elapses — reported, never
    /// hung on). Returns whether the queue actually drained.
    pub(crate) async fn drain(&self, timeout: std::time::Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        while self.in_flight.load(Ordering::SeqCst) > 0 {
            if std::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        true
    }
}

/// Spawn the single catalog writer: events are appended strictly in send order (senders enqueue
/// while holding the engine-state lock, so the log order matches the state-mutation order).
pub(crate) fn spawn_catalog_writer(ds: DsClient) -> CatalogWriter {
    let (tx, mut rx) = mpsc::unbounded_channel::<CatalogEvent>();
    let in_flight = Arc::new(std::sync::atomic::AtomicI64::new(0));
    let counter = in_flight.clone();
    tokio::spawn(async move {
        let mut ensured = false;
        while let Some(ev) = rx.recv().await {
            if !ensured {
                ensured = self::ensure_catalog(&ds).await;
            }
            if let Ok(json) = serde_json::to_value(&ev)
                && let Err(e) = ds.append_json(CATALOG_STREAM, &[json]).await
            {
                tracing::error!("catalog append failed (event lost; restart may under-restore): {e:#}");
            }
            counter.fetch_sub(1, Ordering::SeqCst);
        }
    });
    CatalogWriter { tx, in_flight }
}

/// The durable catalog holds a record written **before** ADR-0002 (a bare `rec.table`).
///
/// A typed error because the boot path treats it differently from every other restore failure: an
/// ordinary failure is logged and the engine continues with an empty registry (the clients recreate
/// their shapes), but this one **refuses to boot**. Half-restoring a pre-qualification catalog is
/// the one outcome worse than not booting: the record's `sig` still carries the bare spelling, so an
/// identical post-cutover create would not share with it and the engine would quietly maintain two
/// streams for one table, forever. Recovery is a deliberate human act (reset the storage), not
/// something the engine gets to paper over.
#[derive(Debug)]
pub struct CatalogPredatesQualification {
    detail: String,
}

impl std::fmt::Display for CatalogPredatesQualification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "durable catalog predates ADR-0002 ({}); reset the durable-streams data directory",
            self.detail
        )
    }
}

impl std::error::Error for CatalogPredatesQualification {}

/// Did the shape's table move while the engine was down? `Some(description)` if so.
///
/// Only Postgres-mode tables can answer: a library-mode table has no fingerprint on either side and
/// is restored as before. A record with no fingerprint of its own, over a table that HAS one, cannot
/// be vouched for and is retired — greenfield, so that is a catalog written before the field
/// existed, not a format to keep compatibility with.
fn schema_moved_while_down(rec: &ShapeRecord, compiled: &HashMap<TableRef, TableSchema>) -> Option<String> {
    let now = compiled.get(&rec.table)?.fingerprint.as_ref()?;
    match &rec.fingerprint {
        Some(then) if then.still_serves(now) => None,
        Some(then) => Some(crate::schema::describe_drift(then, now).join("; ")),
        None => Some("the record predates schema fingerprinting".to_string()),
    }
}

pub(crate) async fn ensure_catalog(ds: &DsClient) -> bool {
    match ds.ensure_stream(CATALOG_STREAM).await {
        Ok(()) => true,
        Err(e) => {
            tracing::error!("catalog stream create failed: {e:#}");
            false
        }
    }
}


impl Engine {
    /// Replay the durable shape catalog and re-register every restorable shape with the (not yet
    /// spawned) sequencer — see [`CATALOG_STREAM`] for the restore semantics per shape kind.
    pub(crate) async fn restore_catalog(&self, compiled: &HashMap<TableRef, TableSchema>) -> Result<()> {
        // 1. Fold the event log.
        // (rec, sig, refcount, dormant resume state). The last Dormant/Reactivated event wins.
        type Restored = (ShapeRecord, Option<String>, usize, Option<(String, crate::pg::SnapshotGate)>);
        let mut recs: HashMap<String, Restored> = HashMap::new();
        let mut start_offset = "-1".to_string();
        let mut off = "-1".to_string();
        loop {
            let (events, next, up_to_date) = self.ds.read_json(CATALOG_STREAM, &off).await?;
            for ev in events {
                // The catalog is engine-written, so `rec.table` is ALWAYS the canonical
                // `schema.name` (`ShapeRecord`'s strict deserializer enforces it). A bare one can
                // only be a catalog written before ADR-0002: refuse the boot naming the record,
                // rather than let the strict deserializer turn it into a silently skipped event.
                if let Some(raw) = ev.pointer("/rec/table").and_then(serde_json::Value::as_str)
                    && crate::table_ref::TableRef::parse(raw).is_ok_and(|t| t.as_str() != raw)
                {
                    let id = ev.pointer("/rec/id").and_then(serde_json::Value::as_str).unwrap_or("<unknown>");
                    return Err(anyhow::Error::new(CatalogPredatesQualification {
                        detail: format!("bare table name '{raw}' in shape {id}"),
                    }));
                }
                let Ok(ev) = serde_json::from_value::<CatalogEvent>(ev) else { continue };
                match ev {
                    CatalogEvent::Created { rec, sig } => {
                        recs.insert(rec.id.clone(), (rec, sig, 1, None));
                    }
                    CatalogEvent::Joined { id } => {
                        if let Some(e) = recs.get_mut(&id) {
                            e.2 += 1;
                        }
                    }
                    CatalogEvent::Left { id } => {
                        if let Some(e) = recs.get_mut(&id) {
                            e.2 = e.2.saturating_sub(1);
                        }
                    }
                    CatalogEvent::Dormant { id, resume_offset, gate } => {
                        if let Some(e) = recs.get_mut(&id) {
                            e.3 = Some((resume_offset, gate));
                        }
                    }
                    CatalogEvent::Reactivated { id } => {
                        if let Some(e) = recs.get_mut(&id) {
                            e.3 = None;
                        }
                    }
                    CatalogEvent::Dropped { id } => {
                        recs.remove(&id);
                    }
                    CatalogEvent::Offset { offset } => start_offset = offset,
                    // Audit only (see the variant): the shapes it explains are already `Dropped`,
                    // and the boot's own introspection is the authority on the schema.
                    CatalogEvent::SchemaChanged { .. } => {}
                }
            }
            match next {
                Some(n) if !up_to_date && n != off => off = n,
                _ => break,
            }
        }
        if recs.is_empty() && start_offset == "-1" {
            return Ok(());
        }
        tracing::info!("catalog restore: {} shape(s), change-log replay from {start_offset}", recs.len());
        *self.seq_start.lock().unwrap() = start_offset;

        // 2. Restore records + shares; subquery shapes are dropped (see CATALOG_STREAM docs).
        let mut resume: Vec<ShapeRecord> = Vec::new();
        let mut dead_streams: Vec<String> = Vec::new();
        {
            let mut st = self.state.lock().await;
            for (id, (rec, sig, refcount, dormant)) in recs {
                if let Ok(num) = id.trim_start_matches('s').parse::<u64>() {
                    st.next_shape_id = st.next_shape_id.max(num + 1);
                }
                // DDL while the engine was DOWN is seen by nothing on the live path: no `Relation`
                // message, no reconciler tick. The record's own fingerprint is the only witness —
                // if it no longer matches what boot introspected, the retained stream holds rows
                // shaped by the old schema and can never be brought up to date (ADR-0005).
                if let Some(what) = schema_moved_while_down(&rec, compiled) {
                    tracing::warn!(
                        "restore: retiring shape {id} on {} — its schema changed while the engine was \
                         down ({what}); subscribers observe the closed stream and recreate",
                        rec.table
                    );
                    self.catalog_tx.send(CatalogEvent::Dropped { id: id.clone() });
                    dead_streams.push(rec.stream_path.clone());
                    continue;
                }
                if rec.is_subquery {
                    // Subquery shapes are registry-served and their inner-node contributor
                    // state is not persisted: a fresh-seeded node cannot detect flips that
                    // happened during downtime (stale move-outs would persist forever), so
                    // they are dropped loudly and clients recreate them.
                    tracing::warn!(
                        "restore: dropping subquery shape {id} (inner-node state is not persisted); subscribers observe the deleted stream and recreate"
                    );
                    self.catalog_tx.send(CatalogEvent::Dropped { id: id.clone() });
                    dead_streams.push(rec.stream_path.clone());
                    continue;
                }
                st.shapes.insert(id.clone(), rec.clone());
                if let Some(sig) = sig {
                    // Restored feeds are live immediately (their streams already hold data).
                    let (ready_tx, ready_rx) = tokio::sync::watch::channel(ShareOutcome::Ready);
                    drop(ready_tx); // receivers keep observing `Ready`
                    st.feed_by_sig.insert(sig.clone(), id.clone());
                    st.feed_shares.insert(id.clone(), FeedShare { sig, refcount, ready: ready_rx });
                }
                match dormant {
                    // A dormant shape restores AS dormant: record + stream retained, no routing,
                    // no replay at boot — the first touch reactivates it from its own resume
                    // offset. (Dormancy age restarts at boot; the TTL clock is conservative.)
                    Some((resume_offset, gate)) => {
                        self.lives.lock().unwrap().insert(
                            id.clone(),
                            ShapeLife {
                                last_read: std::time::Instant::now(),
                                state: LifeState::Dormant {
                                    since: std::time::Instant::now(),
                                    resume_offset,
                                    gate,
                                },
                            },
                        );
                    }
                    None => {
                        self.lives.lock().unwrap().insert(id.clone(), ShapeLife::active());
                        resume.push(rec);
                    }
                }
            }
            self.ensure_sequencer(&mut st);
        }
        // Restored dormant shapes still need the TTL/eviction layers running.
        self.ensure_retention_sweeper();
        // Retirement: clients may still be tailing these streams from before the restart, so close
        // before deleting — their long-poll is released at once with `stream-closed`.
        for path in dead_streams {
            let _ = self.ds.retire_stream(&path).await;
        }

        // 3. Re-register with the sequencer. Plain/routed shapes resume without a backfill and
        // with a passthrough gate (`changes_only = true` path): everything after the restored
        // offset replays, and re-emission across the crash window is idempotent. Aggregates
        // re-seed their fold from a fresh snapshot (fresh gate skips the replayed history).
        let cmd_tx = {
            let st = self.state.lock().await;
            st.sequencer.as_ref().expect("sequencer spawned above").cmd_tx.clone()
        };
        for rec in resume {
            let outcome = self.resume_shape(&cmd_tx, &rec, compiled).await;
            if let Err(e) = outcome {
                tracing::error!("restore: shape {} failed to resume ({e:#}); dropping it", rec.id);
                let mut st = self.state.lock().await;
                st.shapes.remove(&rec.id);
                self.catalog_tx.send(CatalogEvent::Dropped { id: rec.id.clone() });
                if let Some(share) = st.feed_shares.remove(&rec.id) {
                    st.feed_by_sig.remove(&share.sig);
                }
                drop(st);
                // Engine-initiated removal, so retire the stream (close, then delete): the shape is
                // gone, and a client still tailing it from before the restart must learn that rather
                // than sit on a stream nothing will ever append to again. Logged, never fatal — a
                // storage hiccup must not abort the restore of the other shapes.
                if let Err(e) = self.ds.retire_stream(&rec.stream_path).await {
                    tracing::warn!("restore: failed to retire stream {} for dropped shape {}: {e:#}", rec.stream_path, rec.id);
                }
            }
        }
        Ok(())
    }

    /// Re-register one restored shape with the sequencer (the resume half of `restore_catalog`).
    pub(crate) async fn resume_shape(
        &self,
        cmd_tx: &mpsc::UnboundedSender<SequencerCmd>,
        rec: &ShapeRecord,
        compiled: &HashMap<TableRef, TableSchema>,
    ) -> Result<()> {
        let ts = compiled
            .get(&rec.table)
            .with_context(|| format!("table '{}' no longer exists", rec.table))?;
        let out_cols: Option<Arc<Vec<usize>>> = match &rec.columns {
            Some(names) => {
                let idx: Result<Vec<usize>> = names.iter().map(|n| ts.column_index(n)).collect();
                Some(Arc::new(idx?))
            }
            None => None,
        };
        let num_id: u64 = rec.id.trim_start_matches('s').parse().unwrap_or(0);
        // Circuit-served restore: re-register with the sequencer, seed=false for plain shapes
        // (the stream is already complete up to the resume offset; dynamic groups re-derive
        // from the router snapshot, which the catch-up replay has brought to the same point).
        // Aggregates re-seed from the counts snapshot (their fold is not persisted) — same
        // fresh-value semantics as the legacy aggregate resume.
        if let Some(arr) = self.arrangements.lock().unwrap().clone() {
            match &rec.aggregate {
                Some(a) if matches!(a.func, AggFn::Count) && a.col.is_none() => {
                    if let Some(gcols) = arr.counts_group_cols(&rec.table).map(|g| g.to_vec()) {
                        if let Some(constraints) = plan_circuit_agg(rec.where_json.as_ref(), ts, &gcols) {
                            let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
                            cmd_tx
                                .send(SequencerCmd::CreateCircuitAgg {
                                    table: rec.table.clone(),
                                    shape_id: rec.id.clone(),
                                    stream_path: rec.stream_path.clone(),
                                    constraints,
                                    ready: ready_tx,
                                })
                                .map_err(|_| anyhow::anyhow!("sequencer is gone"))?;
                            ready_rx
                                .await
                                .unwrap_or_else(|_| Err("sequencer dropped".to_string()))
                                .map_err(|e| anyhow::anyhow!(e))?;
                            self.state.lock().await.circuit_placement.insert(
                                rec.id.clone(),
                                CircuitPlacement { label: "counts".into(), col: None, counts: true },
                            );
                            return Ok(());
                        }
                    }
                }
                _ => {}
            }
        }
        // Compiled lazily, after the circuit branch: a circuit-served subquery record never
        // needs (and could not build) a registry-free compiled predicate.
        let pred = Arc::new(CompiledPredicate::compile_opt(rec.where_json.as_ref(), ts)?);
        let (kind, changes_only, is_aggregate) = match &rec.aggregate {
            Some(a) => {
                let col = a.col.as_deref().map(|c| ts.column_index(c)).transpose()?;
                (CreateKind::Aggregate { func: a.func, col }, false, true)
            }
            None => (CreateKind::Plain, true, false),
        };
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
        cmd_tx
            .send(SequencerCmd::BeginShape {
                table: rec.table.clone(),
                shape_id: rec.id.clone(),
                num_id,
                stream_path: rec.stream_path.clone(),
                pred: pred.clone(),
                out_cols: out_cols.clone(),
                kind,
                ack: ack_tx,
            })
            .map_err(|_| anyhow::anyhow!("sequencer is gone"))?;
        backfill_and_activate(
            &self.ds,
            &self.pg_url,
            cmd_tx,
            ts,
            &rec.table,
            &rec.id,
            &rec.stream_path,
            &pred,
            out_cols.as_ref(),
            changes_only,
            is_aggregate,
            ack_rx,
        )
        .await
        .map_err(|e| anyhow::anyhow!(e))
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    fn created_event(table: &str) -> serde_json::Value {
        serde_json::json!({
            "t": "created",
            "rec": {
                "id": "s1",
                "table": table,
                "stream_path": "shape/s1",
                "changes_only": false,
                "where_json": null,
                "columns": null,
                "family_key": null,
                "is_subquery": false,
                "aggregate": null,
                "fingerprint": null,
            },
            "sig": null,
        })
    }

    /// The catalog is strict where API ingress is lenient: a record must already carry the
    /// canonical `schema.name`. A bare one can only be a catalog written before ADR-0002, and
    /// resolving it would leave its (bare-spelled) sharing signature unable to match an identical
    /// post-cutover create — two maintained streams for one table.
    #[test]
    fn catalog_records_must_be_canonically_spelled() {
        let ok = serde_json::from_value::<CatalogEvent>(created_event("public.users"))
            .expect("a canonical record restores");
        match ok {
            CatalogEvent::Created { rec, .. } => {
                assert_eq!(rec.table.to_string(), "public.users");
                assert_eq!(rec.table.schema(), "public");
            }
            other => panic!("expected Created, got {other:?}"),
        }

        let err = serde_json::from_value::<CatalogEvent>(created_event("users"))
            .expect_err("a bare record is refused, not resolved to public.users");
        assert!(err.to_string().contains("canonical"), "{err}");

        // The same rule applies to a non-public bare-impossible spelling: `a.b.c` is not a table.
        assert!(serde_json::from_value::<CatalogEvent>(created_event("a.b.c")).is_err());
        // ...and an explicitly qualified non-public table restores untouched.
        let ok = serde_json::from_value::<CatalogEvent>(created_event("other.users")).unwrap();
        match ok {
            CatalogEvent::Created { rec, .. } => assert_eq!(rec.table.to_string(), "other.users"),
            other => panic!("expected Created, got {other:?}"),
        }
    }
}
