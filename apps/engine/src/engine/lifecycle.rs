//! Shape lifecycle: creation (all strategies), sharing, and the retention state machine
//! (release/touch/dormancy/eviction/sweep).

use super::*;


impl Engine {
    /// `share`: when true, an identical existing shape (same table, canonical predicate, and columns) is
    /// joined by ref-count instead of creating a second stream — so N app clients subscribing to the same
    /// reference shape (e.g. `project_members WHERE user_id = me`) share one maintained output. The
    /// Electric `/v1/shape` path passes `false`: it keys per-request live state by shape id, so each
    /// request needs its own handle.
    pub async fn create_shape(
        &self,
        table: &str,
        where_: Option<PredicateJson>,
        columns: Option<Vec<String>>,
        changes_only: bool,
        share: bool,
    ) -> Result<ShapeRecord> {
        // A degraded engine cannot say what belongs in a shape (see `Engine::ensure_not_degraded`),
        // so it refuses to make one rather than backfill from state it knows is wrong. This early
        // check is the cheap refusal and the only one the *join* path takes; a create that gets
        // past it checks again under the state lock before registering, and once more before
        // handing back a handle (see `ensure_create_not_degraded`).
        self.ensure_not_degraded()?;
        // Whole shape-creation timer (backfill + registration); emitted by the creator on success only
        // (joiners return early before this fires) as `create_snapshot_task.stop.duration`.
        let created_at = std::time::Instant::now();
        let mut st = self.state.lock().await;
        let ts = match st.tables.get(table) {
            Some(ts) => ts.clone(),
            None => bail!("unknown table '{table}'"),
        };
        let col_names = columns.clone();
        let out_cols = resolve_columns(&ts, columns)?;

        // Shape sharing: an identical shape (subset feed, materialized, OR subquery) that already exists
        // is joined (ref-count++), returning the same stream — no second stream, no per-subscriber append
        // fan-out. Subquery shapes share their inner-set nodes in the registry regardless; sharing the
        // *outer* shape here collapses identical subquery shapes fully.
        let feed_sig = if share { Some(shape_signature(table, &where_, &out_cols, changes_only)) } else { None };
        if let Some(sig) = &feed_sig {
            if let Some(existing_id) = st.feed_by_sig.get(sig).cloned() {
                if let Some(rec) = st.shapes.get(&existing_id).cloned() {
                    let share = st.feed_shares.get_mut(&existing_id).expect("share entry for live feed");
                    share.refcount += 1;
                let _ = self.catalog_tx.send(CatalogEvent::Joined { id: existing_id.clone() });
                    let ready = share.ready.clone();
                    // Release the lock, then wait for the creator's backfill to land: a joiner must not
                    // see a stream whose snapshot isn't readable yet, and must surface (not mask) a
                    // failed creation.
                    drop(st);
                    if let Err(e) = await_share_ready(ready, &existing_id).await {
                        // The failed creator already removed the share entries; undo nothing.
                        // A degraded outcome arrives typed, so this joiner answers 503 with the
                        // same reason the creator did.
                        return Err(e);
                    }
                    // The creator succeeded — but it may have succeeded a moment BEFORE the
                    // degradation mark, in which case the reaper is about to delete the very
                    // stream this handle points at. This is the joiner's equivalent of the
                    // creator's final `ensure_create_not_degraded`: check the same latch after
                    // the work is done, and give back the refcount taken above so a refused
                    // join does not pin the shape.
                    if let Err(e) = self.ensure_not_degraded() {
                        self.release_shape(&existing_id).await;
                        return Err(e);
                    }
                    // A rejoin is a touch: if the shape went dormant since the last subscriber
                    // left, reactivate it (change-log replay) before handing out the stream.
                    if let Err(e) = self.ensure_active(&existing_id).await {
                        // Roll the failed join back so the dead subscription doesn't pin the shape.
                        self.release_shape(&existing_id).await;
                        return Err(e);
                    }
                    return Ok(rec);
                }
            }
        }

        let num_id = st.next_shape_id;
        let id = format!("s{num_id}");
        st.next_shape_id += 1;
        let stream_path = format!("shape/{id}");
        // NOTE: the stream itself is created AFTER the state lock is released (per path below):
        // the PUT is a storage round-trip with a durability fsync, and holding the global lock
        // across it serializes concurrent shape creations.


        // Subquery shapes (`col IN (SELECT …)`) are maintained by the cross-table registry, not by a
        // tailer's local routing. Ensure a tailer exists for the outer table AND every referenced inner
        // table (so their deltas reach the registry), then register + backfill via the registry.
        if where_.as_ref().is_some_and(predicate_has_subquery) {
            let where_json = where_.expect("subquery predicate present");
            let mut tables = referenced_tables(&where_json);
            tables.push(table.to_string());
            for t in &tables {
                if !st.tables.contains_key(t) {
                    bail!("unknown table '{t}' referenced by subquery");
                }
            }
            // The sequencer feeds every table's deltas to the registry; just make sure it runs.
            self.ensure_sequencer(&mut st);
            let rec = ShapeRecord {
                id: id.clone(),
                table: table.to_string(),
                stream_path: stream_path.clone(),
                changes_only,
                where_json: Some(where_json.clone()),
                columns: col_names.clone(),
                family_key: None,
                is_subquery: true,
                aggregate: None,
            };
            // The load-bearing degrade check: taken under the state lock, in the same critical
            // section as the registration below. The degradation reaper snapshots every registered
            // subquery stream under this same lock, so with respect to that snapshot a create either
            // registered before it (and is refused by its final check — see
            // `ensure_create_not_degraded`) or observes `degraded` here and never registers at all.
            self.ensure_not_degraded()?;
            st.shapes.insert(id.clone(), rec.clone());
            let _ = self.catalog_tx.send(CatalogEvent::Created { rec: rec.clone(), sig: feed_sig.clone() });
            self.lives.lock().unwrap().insert(id.clone(), ShapeLife::active());
            self.ensure_retention_sweeper();
            // First subquery shape: from here on a lost flip is possible, so the stream reaper that
            // fires on degradation needs to exist.
            self.ensure_degrade_reaper();
            // Register this (first) subquery shape so later identical ones join it by ref-count.
            // Joiners wait on `ready_tx` — the shape isn't live until the registry has seeded its
            // nodes and backfilled the stream.
            let (ready_tx, ready_rx) = tokio::sync::watch::channel(ShareOutcome::Pending);
            if let Some(sig) = feed_sig {
                st.feed_by_sig.insert(sig.clone(), id.clone());
                st.feed_shares.insert(id.clone(), FeedShare { sig, refcount: 1, ready: ready_rx });
            }
            // Release the engine-state lock before the registry work. Creation is three-phase:
            // begin (brief registry lock: nodes/edges/pending buffer registered) → Postgres
            // seeding + backfill with NO lock held (concurrent creates parallelize on the
            // shared pool) → finish (brief lock: install seeds, gated replay of buffered
            // deltas, register the shape). Replay flips propagate through the worker pool.
            drop(st);
            let mut creating = CreateGuard::new(self, &id, table, &stream_path, Registration::Registry);
            let res = async {
                self.ds.ensure_stream(&stream_path).await?;
                self.create_subquery_three_phase(&id, table, &stream_path, &where_json, out_cols, changes_only)
                    .await
            }
            .await;
            match res {
                Ok(()) => {
                    // The create's work is done, but it may have overlapped a degradation — its stream is
                    // then already reaped and the handle would be dead on arrival. Refuse instead of
                    // answering success (see `ensure_create_not_degraded`).
                    if let Err(e) = self.ensure_create_not_degraded() {
                        let _ = ready_tx.send(ShareOutcome::Degraded);
                        creating.rollback().await;
                        return Err(e);
                    }
                    creating.complete();
                    let _ = ready_tx.send(ShareOutcome::Ready);
                    trace_lifecycle(
                        &self.trace_tx,
                        crate::trace::GraphLifecycle::ShapeAdded { shape: id, table: table.to_string() },
                    );
                    crate::statsd::create_snapshot_task(created_at.elapsed());
                    return Ok(rec);
                }
                Err(e) => {
                    // Registration failed: wake any joiners with the failure, then undo everything
                    // this create registered so later identical creates don't join a dead stream.
                    let _ = ready_tx.send(ShareOutcome::Failed);
                    creating.rollback().await;
                    return Err(e);
                }
            }
        }

        let pred = Arc::new(CompiledPredicate::compile_opt(where_.as_ref(), &ts)?);
        // Family placement (for graph introspection): an equality template routes by these key columns
        // via a shared family; otherwise it's a standalone filter.
        let family_key = pred
            .equality_template()
            .map(|pairs| pairs.iter().map(|(i, _)| ts.columns[*i].0.clone()).collect::<Vec<_>>());

        let cmd_tx = self.ensure_sequencer(&mut st).cmd_tx.clone();
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
        cmd_tx
            .send(SequencerCmd::BeginShape {
                table: table.to_string(),
                shape_id: id.clone(),
                num_id,
                stream_path: stream_path.clone(),
                pred: pred.clone(),
                out_cols: out_cols.clone(),
                kind: CreateKind::Plain,
                ack: ack_tx,
            })
            .map_err(|_| anyhow::anyhow!("sequencer is gone"))?;

        let rec = ShapeRecord {
            id: id.clone(),
            table: table.to_string(),
            stream_path,
            changes_only,
            where_json: where_.clone(),
            columns: col_names,
            family_key,
            is_subquery: false,
            aggregate: None,
        };
        // Under the state lock, immediately before registering — see `ensure_create_not_degraded`
        // for why this check and the one after the create's work are together sufficient.
        self.ensure_not_degraded()?;
        st.shapes.insert(id.clone(), rec.clone());
        let _ = self.catalog_tx.send(CatalogEvent::Created { rec: rec.clone(), sig: feed_sig.clone() });
        self.lives.lock().unwrap().insert(id.clone(), ShapeLife::active());
        self.ensure_retention_sweeper();
        // Register the (first) shared feed so later identical subset feeds join it. Joiners wait on
        // `share_tx` for the backfill outcome.
        let (share_tx, share_rx) = tokio::sync::watch::channel(ShareOutcome::Pending);
        if let Some(sig) = feed_sig {
            st.feed_by_sig.insert(sig.clone(), id.clone());
            st.feed_shares.insert(id.clone(), FeedShare { sig, refcount: 1, ready: share_rx });
        }
        // Release the engine-state lock, then run the two-phase backfill+activate so the shape's
        // snapshot is readable when we return (the Electric adapter folds the stream immediately).
        // The sequencer keeps processing all tables meanwhile, buffering this shape's deltas.
        drop(st);
        let mut creating = CreateGuard::new(self, &id, table, &rec.stream_path, Registration::Sequencer);
        let outcome = match self.ds.ensure_stream(&rec.stream_path).await {
            Err(e) => Err(format!("creating shape stream: {e:#}")),
            Ok(()) => backfill_and_activate(
            &self.ds, &self.pg_url, &cmd_tx, &ts, table, &id, &rec.stream_path, &pred,
            out_cols.as_ref(), changes_only, false, ack_rx,
        )
        .await,
        };
        match outcome {
            Ok(()) => {
                // The create's work is done, but it may have overlapped a degradation — its stream is
                // then already reaped and the handle would be dead on arrival. Refuse instead of
                // answering success (see `ensure_create_not_degraded`).
                if let Err(e) = self.ensure_create_not_degraded() {
                    let _ = share_tx.send(ShareOutcome::Degraded);
                    creating.rollback().await;
                    return Err(e);
                }
                creating.complete();
                let _ = share_tx.send(ShareOutcome::Ready);
                trace_lifecycle(
                    &self.trace_tx,
                    crate::trace::GraphLifecycle::ShapeAdded { shape: rec.id.clone(), table: rec.table.clone() },
                );
                crate::statsd::create_snapshot_task(created_at.elapsed());
                Ok(rec)
            }
            Err(e) => {
                // Backfill/registration failed: wake any joiners, then undo the whole registration
                // (no zombie shape a later identical create would join) and surface the error.
                let _ = share_tx.send(ShareOutcome::Failed);
                creating.rollback().await;
                bail!("shape '{id}' creation failed: {e}")
            }
        }
    }

    /// Create a scalar **aggregation** shape (COUNT/SUM/AVG/MIN/MAX over `where`), maintained
    /// incrementally. An electric-circuits extension — not part of the Electric-compatible API. Rejects
    /// subquery predicates (use a plain filter); SUM/AVG/MIN/MAX require a column.
    pub async fn create_aggregate(
        &self,
        table: &str,
        where_: Option<PredicateJson>,
        func: AggFn,
        col: Option<String>,
    ) -> Result<ShapeRecord> {
        // Same refusal as `create_shape`: a degraded engine does not get to answer for a fold over
        // rows whose membership it can no longer vouch for.
        self.ensure_not_degraded()?;
        let mut st = self.state.lock().await;
        let ts = st.tables.get(table).cloned().ok_or_else(|| anyhow::anyhow!("unknown table '{table}'"))?;
        if where_.as_ref().is_some_and(predicate_has_subquery) {
            bail!("aggregations over subquery predicates are not supported");
        }
        let col_idx = match &col {
            Some(c) => Some(ts.column_index(c)?),
            None => None,
        };
        if matches!(func, AggFn::Sum | AggFn::Avg | AggFn::Min | AggFn::Max) && col_idx.is_none() {
            bail!("aggregation {func:?} requires a column");
        }

        // Aggregate sharing: an identical aggregation (same table, predicate, function, column) is joined
        // by ref-count — one maintained fold feeds every subscriber (e.g. the same live COUNT opened by
        // many clients).
        let agg_sig = agg_signature(table, &where_, &func, col_idx);
        if let Some(existing_id) = st.feed_by_sig.get(&agg_sig).cloned() {
            if let Some(rec) = st.shapes.get(&existing_id).cloned() {
                let share = st.feed_shares.get_mut(&existing_id).expect("share entry for aggregate");
                share.refcount += 1;
                let _ = self.catalog_tx.send(CatalogEvent::Joined { id: existing_id.clone() });
                let ready = share.ready.clone();
                drop(st);
                await_share_ready(ready, &existing_id).await?;
                self.touch_shape(&existing_id); // aggregates never park, but the read is a touch
                return Ok(rec);
            }
        }

        let pred = Arc::new(CompiledPredicate::compile_opt(where_.as_ref(), &ts)?);

        let num_id = st.next_shape_id;
        let id = format!("s{num_id}");
        st.next_shape_id += 1;
        let stream_path = format!("shape/{id}");
        self.ds.ensure_stream(&stream_path).await?;

        // Circuit-served path: a bare COUNT whose predicate decomposes over the table's counts
        // pipeline is seeded by summing groups and updated from group deltas — no Postgres.
        if matches!(func, AggFn::Count) && col_idx.is_none() {
            let arr = self.arrangements.lock().unwrap().clone();
            if let Some(arr) = arr {
                if let Some(gcols) = arr.counts_group_cols(table).map(|g| g.to_vec()) {
                    if let Some(constraints) = plan_circuit_agg(where_.as_ref(), &ts, &gcols) {
                        let cmd_tx = self.ensure_sequencer(&mut st).cmd_tx.clone();
                        let (ready_tx2, ready_rx2) = tokio::sync::oneshot::channel();
                        cmd_tx
                            .send(SequencerCmd::CreateCircuitAgg {
                                table: table.to_string(),
                                shape_id: id.clone(),
                                stream_path: stream_path.clone(),
                                constraints,
                                ready: ready_tx2,
                            })
                            .map_err(|_| anyhow::anyhow!("sequencer is gone"))?;
                        let rec = ShapeRecord {
                            id: id.clone(),
                            table: table.to_string(),
                            stream_path: stream_path.clone(),
                            changes_only: false,
                            where_json: where_,
                            columns: None,
                            family_key: None,
                            is_subquery: false,
                            aggregate: Some(AggInfo { func, col }),
                        };
                        // Under the state lock, immediately before registering — see
                        // `ensure_create_not_degraded` for why this check and the one after the
                        // create's work are together sufficient.
                        self.ensure_not_degraded()?;
                        st.shapes.insert(id.clone(), rec.clone());
                        st.circuit_placement.insert(
                            id.clone(),
                            CircuitPlacement { label: "counts".into(), col: None, counts: true },
                        );
                        let _ = self
                            .catalog_tx
                            .send(CatalogEvent::Created { rec: rec.clone(), sig: Some(agg_sig.clone()) });
                        self.lives.lock().unwrap().insert(id.clone(), ShapeLife::active());
                        self.ensure_retention_sweeper();
                        let (share_tx, share_rx) = tokio::sync::watch::channel(ShareOutcome::Pending);
                        st.feed_by_sig.insert(agg_sig.clone(), id.clone());
                        st.feed_shares.insert(id.clone(), FeedShare { sig: agg_sig, refcount: 1, ready: share_rx });
                        drop(st);
                        let mut creating =
                            CreateGuard::new(self, &id, table, &rec.stream_path, Registration::Sequencer);
                        return match ready_rx2
                            .await
                            .unwrap_or_else(|_| Err("sequencer dropped the ready channel".to_string()))
                        {
                            Ok(()) => {
                                // The create's work is done, but it may have overlapped a degradation — its stream is
                                // then already reaped and the handle would be dead on arrival. Refuse instead of
                                // answering success (see `ensure_create_not_degraded`).
                                if let Err(e) = self.ensure_create_not_degraded() {
                                    let _ = share_tx.send(ShareOutcome::Degraded);
                                    creating.rollback().await;
                                    return Err(e);
                                }
                                creating.complete();
                                let _ = share_tx.send(ShareOutcome::Ready);
                                trace_lifecycle(
                                    &self.trace_tx,
                                    crate::trace::GraphLifecycle::ShapeAdded {
                                        shape: rec.id.clone(),
                                        table: rec.table.clone(),
                                    },
                                );
                                Ok(rec)
                            }
                            Err(e) => {
                                let _ = share_tx.send(ShareOutcome::Failed);
                                creating.rollback().await;
                                bail!("aggregate '{id}' creation failed: {e}")
                            }
                        };
                    }
                }
            }
        }

        let cmd_tx = self.ensure_sequencer(&mut st).cmd_tx.clone();
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
        cmd_tx
            .send(SequencerCmd::BeginShape {
                table: table.to_string(),
                shape_id: id.clone(),
                num_id,
                stream_path: stream_path.clone(),
                pred: pred.clone(),
                out_cols: None,
                kind: CreateKind::Aggregate { func, col: col_idx },
                ack: ack_tx,
            })
            .map_err(|_| anyhow::anyhow!("sequencer is gone"))?;

        let stream_path_c = stream_path.clone();
        let rec = ShapeRecord {
            id: id.clone(),
            table: table.to_string(),
            stream_path,
            changes_only: false,
            where_json: where_,
            columns: None,
            family_key: None,
            is_subquery: false,
            aggregate: Some(AggInfo { func, col }),
        };
        // Under the state lock, immediately before registering — see `ensure_create_not_degraded`
        // for why this check and the one after the create's work are together sufficient.
        self.ensure_not_degraded()?;
        st.shapes.insert(id.clone(), rec.clone());
        let _ = self.catalog_tx.send(CatalogEvent::Created { rec: rec.clone(), sig: Some(agg_sig.clone()) });
        self.lives.lock().unwrap().insert(id.clone(), ShapeLife::active());
        self.ensure_retention_sweeper();
        // Register this (first) aggregate so later identical ones join it by ref-count.
        let (share_tx, share_rx) = tokio::sync::watch::channel(ShareOutcome::Pending);
        st.feed_by_sig.insert(agg_sig.clone(), id.clone());
        st.feed_shares.insert(id.clone(), FeedShare { sig: agg_sig, refcount: 1, ready: share_rx });
        drop(st);
        let mut creating = CreateGuard::new(self, &id, table, &rec.stream_path, Registration::Sequencer);
        let outcome = backfill_and_activate(
            &self.ds, &self.pg_url, &cmd_tx, &ts, table, &id, &stream_path_c, &pred,
            None, false, true, ack_rx,
        )
        .await;
        match outcome {
            Ok(()) => {
                // The create's work is done, but it may have overlapped a degradation — its stream is
                // then already reaped and the handle would be dead on arrival. Refuse instead of
                // answering success (see `ensure_create_not_degraded`).
                if let Err(e) = self.ensure_create_not_degraded() {
                    let _ = share_tx.send(ShareOutcome::Degraded);
                    creating.rollback().await;
                    return Err(e);
                }
                creating.complete();
                let _ = share_tx.send(ShareOutcome::Ready);
                trace_lifecycle(
                    &self.trace_tx,
                    crate::trace::GraphLifecycle::ShapeAdded { shape: rec.id.clone(), table: rec.table.clone() },
                );
                Ok(rec)
            }
            Err(e) => {
                let _ = share_tx.send(ShareOutcome::Failed);
                creating.rollback().await;
                bail!("aggregate '{id}' creation failed: {e}")
            }
        }
    }

    /// Release one subscription on a shape (extended-API `DELETE /shapes/{id}`, `/v1/shape` handle
    /// eviction). Refcount-0 does **not** tear the shape down: it stays active (a brief reconnect
    /// rejoins it warm), goes dormant after the retention idle timeout, and is eventually evicted
    /// by the layered policy (see `crate::retention`). Releasing is also a touch, so the idle
    /// countdown starts at the disconnect. Infallible: it only adjusts in-memory counters.
    pub async fn release_shape(&self, id: &str) {
        let mut st = self.state.lock().await;
        if let Some(share) = st.feed_shares.get_mut(id) {
            share.refcount = share.refcount.saturating_sub(1);
            let _ = self.catalog_tx.send(CatalogEvent::Left { id: id.to_string() });
        }
        drop(st);
        self.touch_shape(id);
    }

    /// Force-drop a shape NOW, bypassing the retention lifecycle: full teardown (record, share
    /// entries, lifecycle entry, sequencer routing, subquery-registry entry, durable stream)
    /// regardless of refcount or lifecycle state. An admin/debug operation (`DELETE
    /// /shapes/{id}?purge=true`, the visualizer's trash button) — subscribed clients see their
    /// stream vanish and recreate via the normal 404 / must-refetch path. The sequencer command
    /// queue is FIFO, so a purge ordered after an in-flight resume removes whatever the resume
    /// registered.
    pub async fn purge_shape(&self, id: &str) -> Result<()> {
        let mut st = self.state.lock().await;
        self.lives.lock().unwrap().remove(id);
        if let Some(share) = st.feed_shares.remove(id) {
            st.feed_by_sig.remove(&share.sig);
        }
        let removed = st.shapes.remove(id);
        st.circuit_placement.remove(id);
        if removed.is_some() {
            let _ = self.catalog_tx.send(CatalogEvent::Dropped { id: id.to_string() });
        }
        if let Some(rec) = &removed {
            if let Some(seq) = st.sequencer.as_ref() {
                let _ = seq
                    .cmd_tx
                    .send(SequencerCmd::RemoveShape { table: rec.table.clone(), shape_id: id.to_string() });
            }
        }
        drop(st);
        // Subquery shapes live in the registry (a no-op for plain shapes).
        self.subqueries.lock().await.drop_subquery_shape(id).await;
        if let Some(rec) = removed {
            // Retirement: close (releasing any tailing long-poll with `stream-closed`) then delete.
            if let Err(e) = self.ds.retire_stream(&rec.stream_path).await {
                tracing::warn!("failed to delete stream {} for purged shape {id}: {e:#}", rec.stream_path);
            }
            trace_lifecycle(&self.trace_tx, crate::trace::GraphLifecycle::ShapeDropped { shape: id.to_string() });
            tracing::info!("purged shape {id} (forced)");
        }
        Ok(())
    }

    /// Record an engine-visible read of a shape (drives the retention idle timer + LRU order).
    pub(crate) fn touch_shape(&self, id: &str) {
        if let Some(life) = self.lives.lock().unwrap().get_mut(id) {
            life.last_read = std::time::Instant::now();
        }
    }

    /// The shape's retention lifecycle, for introspection (`GET /shapes/{id}`).
    pub async fn shape_lifecycle(&self, id: &str) -> Option<&'static str> {
        self.lives.lock().unwrap().get(id).map(|l| match l.state {
            LifeState::Active => "active",
            LifeState::Deactivating { .. } => "deactivating",
            LifeState::Dormant { .. } => "dormant",
            LifeState::Reactivating { .. } => "reactivating",
        })
    }

    /// Make sure a shape is active, reactivating it from dormancy if needed ("any touch
    /// reactivates"): replay the change log from the shape's resume offset through its predicate
    /// onto the retained stream — no Postgres backfill — then re-register it for live routing.
    /// Concurrent touches coalesce onto one replay; a touch during deactivation waits for the
    /// transition to settle first. Also refreshes `last_read`.
    pub async fn ensure_active(&self, id: &str) -> Result<()> {
        loop {
            enum Step {
                Done,
                WaitDeactivate(tokio::sync::watch::Receiver<bool>),
                WaitReactivate(tokio::sync::watch::Receiver<Option<bool>>),
            }
            let step = {
                let mut lives = self.lives.lock().unwrap();
                match lives.get_mut(id) {
                    // Unknown to retention (already evicted, or never tracked): nothing to do here —
                    // the caller's own record lookup decides between 404 and normal service.
                    None => Step::Done,
                    Some(life) => {
                        life.last_read = std::time::Instant::now();
                        match &life.state {
                            LifeState::Active => Step::Done,
                            LifeState::Deactivating { done } => Step::WaitDeactivate(done.clone()),
                            LifeState::Reactivating { done } => Step::WaitReactivate(done.clone()),
                            LifeState::Dormant { resume_offset, gate, .. } => {
                                // Kick off the replay in a DETACHED task: `ensure_active` futures
                                // are dropped when an HTTP client disconnects, and a cancelled
                                // in-place replay would strand the shape in `Reactivating`. The
                                // task always settles the lifecycle state and publishes the
                                // outcome; this caller then awaits THIS attempt's channel like any
                                // concurrent toucher.
                                let resume_offset = resume_offset.clone();
                                let gate = gate.clone();
                                let (tx, rx) = tokio::sync::watch::channel(None);
                                life.state = LifeState::Reactivating { done: rx.clone() };
                                let engine = self.clone();
                                let id = id.to_string();
                                tokio::spawn(async move {
                                    let res = engine.resume_dormant(&id, resume_offset.clone(), gate.clone()).await;
                                    let mut lives = engine.lives.lock().unwrap();
                                    match res {
                                        Ok(()) => {
                                            if let Some(life) = lives.get_mut(&id) {
                                                life.state = LifeState::Active;
                                                life.last_read = std::time::Instant::now();
                                            }
                                            let _ = tx.send(Some(true));
                                        }
                                        Err(e) => {
                                            tracing::warn!("reactivating shape {id} failed: {e:#}");
                                            // Restore the dormant resume state so a later touch retries.
                                            if let Some(life) = lives.get_mut(&id) {
                                                life.state = LifeState::Dormant {
                                                    since: std::time::Instant::now(),
                                                    resume_offset,
                                                    gate,
                                                };
                                            }
                                            let _ = tx.send(Some(false));
                                        }
                                    }
                                });
                                Step::WaitReactivate(rx)
                            }
                        }
                    }
                }
            };
            match step {
                Step::Done => return Ok(()),
                Step::WaitDeactivate(mut rx) => {
                    // Deactivation in flight: wait for it to settle, then loop (we'll see Dormant).
                    while !*rx.borrow_and_update() {
                        if rx.changed().await.is_err() {
                            break; // deactivator vanished; re-inspect the state
                        }
                    }
                }
                Step::WaitReactivate(mut rx) => loop {
                    let outcome = *rx.borrow_and_update();
                    match outcome {
                        Some(true) => return Ok(()),
                        Some(false) => bail!("shape '{id}' reactivation failed; retry the read"),
                        None => {
                            if rx.changed().await.is_err() {
                                bail!("shape '{id}' reactivator died; retry the read");
                            }
                        }
                    }
                },
            }
        }
    }

    /// The replay half of a reactivation: re-register the shape through the sequencer's two-phase
    /// pending-buffer handshake, but replay the change log from the dormant resume offset instead
    /// of taking a Postgres snapshot. Live deltas arriving during the replay buffer in the pending
    /// shape and drain through the same gate at activation; any overlap between the replay and the
    /// buffer double-applies only absolute per-pk upserts/deletes — idempotent for stream readers.
    /// Split from [`ensure_active`] so the lifecycle bookkeeping stays in one place.
    pub(crate) async fn resume_dormant(&self, id: &str, resume_offset: String, gate: crate::pg::SnapshotGate) -> Result<()> {
        let (rec, ts, pred, out_cols, num_id, cmd_tx) = {
            let mut st = self.state.lock().await;
            let rec =
                st.shapes.get(id).cloned().with_context(|| format!("shape '{id}' vanished during reactivation"))?;
            let ts =
                st.tables.get(&rec.table).cloned().with_context(|| format!("unknown table '{}'", rec.table))?;
            let pred = Arc::new(CompiledPredicate::compile_opt(rec.where_json.as_ref(), &ts)?);
            let out_cols = resolve_columns(&ts, rec.columns.clone())?;
            let num_id: u64 =
                id.strip_prefix('s').and_then(|n| n.parse().ok()).context("unparseable shape id")?;
            let cmd_tx = self.ensure_sequencer(&mut st).cmd_tx.clone();
            (rec, ts, pred, out_cols, num_id, cmd_tx)
        };
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
        cmd_tx
            .send(SequencerCmd::BeginShape {
                table: rec.table.clone(),
                shape_id: id.to_string(),
                num_id,
                stream_path: rec.stream_path.clone(),
                pred: pred.clone(),
                out_cols: out_cols.clone(),
                kind: CreateKind::Plain,
                ack: ack_tx,
            })
            .map_err(|_| anyhow::anyhow!("sequencer is gone"))?;
        ack_rx.await.map_err(|_| anyhow::anyhow!("sequencer dropped the begin-shape ack"))?;
        // Replay everything the retained stream is missing (buffering live deltas meanwhile).
        let emitted = match replay_changes_for_shape(
            &self.ds,
            &ts,
            &rec.table,
            &pred,
            out_cols.as_ref(),
            &gate,
            &rec.stream_path,
            &resume_offset,
        )
        .await
        {
            Ok(n) => n,
            Err(e) => {
                let _ = cmd_tx
                    .send(SequencerCmd::AbortShape { table: rec.table.clone(), shape_id: id.to_string() });
                return Err(e.context(format!("shape '{id}' reactivation replay failed")));
            }
        };
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        cmd_tx
            .send(SequencerCmd::ActivateShape {
                table: rec.table.clone(),
                shape_id: id.to_string(),
                gate,
                agg_seed: Vec::new(),
                emitted_seed: emitted,
                ready: ready_tx,
            })
            .map_err(|_| anyhow::anyhow!("sequencer is gone"))?;
        ready_rx
            .await
            .unwrap_or_else(|_| Err("sequencer dropped the ready channel".to_string()))
            .map_err(|e| anyhow::anyhow!("shape '{id}' reactivation failed: {e}"))?;
        let _ = self.catalog_tx.send(CatalogEvent::Reactivated { id: id.to_string() });
        metrics().shapes_reactivated.fetch_add(1, Ordering::Relaxed);
        trace_lifecycle(
            &self.trace_tx,
            crate::trace::GraphLifecycle::ShapeReactivated { shape: id.to_string(), table: rec.table.clone() },
        );
        tracing::info!("reactivated dormant shape {id} (table {})", rec.table);
        Ok(())
    }

    /// Move an idle refcount-0 shape from active to dormant: the sequencer unregisters its
    /// routing and hands back the resume state (fully-processed change-log offset + the shape's
    /// snapshot gate); the stream and record are retained. Rechecks eligibility under the locks —
    /// a touch or rejoin racing the sweep wins.
    ///
    /// Parking is NOT retirement: the retained stream is never closed, because reactivation appends
    /// the replayed changes to it (closing is terminal for appends).
    pub(crate) async fn deactivate_shape(&self, id: &str) -> Result<()> {
        let st = self.state.lock().await;
        let Some(rec) = st.shapes.get(id).cloned() else { return Ok(()) }; // already gone
        if rec.is_subquery || rec.aggregate.is_some() {
            return Ok(()); // never dormant (state not rebuildable from a bounded replay)
        }
        if st.feed_shares.get(id).is_some_and(|s| s.refcount > 0) {
            return Ok(()); // resubscribed since the sweep snapshot
        }
        let Some(cmd_tx) = st.sequencer.as_ref().map(|s| s.cmd_tx.clone()) else { return Ok(()) };
        let (done_tx, done_rx) = tokio::sync::watch::channel(false);
        {
            let mut lives = self.lives.lock().unwrap();
            let Some(life) = lives.get_mut(id) else { return Ok(()) };
            if !matches!(life.state, LifeState::Active)
                || life.last_read.elapsed() < self.retention.idle_timeout
            {
                return Ok(()); // touched or already transitioning since the sweep snapshot
            }
            life.state = LifeState::Deactivating { done: done_rx };
        }
        drop(st);

        let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
        let sent = cmd_tx
            .send(SequencerCmd::DeactivateShape { table: rec.table.clone(), shape_id: id.to_string(), resp: resp_tx })
            .is_ok();
        let resume = if sent { resp_rx.await.ok().flatten() } else { None };
        let mut lives = self.lives.lock().unwrap();
        let Some(life) = lives.get_mut(id) else { return Ok(()) };
        match resume {
            Some((resume_offset, gate)) => {
                life.state = LifeState::Dormant {
                    since: std::time::Instant::now(),
                    resume_offset: resume_offset.clone(),
                    gate: gate.clone(),
                };
                drop(lives);
                let _ = self.catalog_tx.send(CatalogEvent::Dormant { id: id.to_string(), resume_offset, gate });
                metrics().shapes_dormanted.fetch_add(1, Ordering::Relaxed);
                trace_lifecycle(&self.trace_tx, crate::trace::GraphLifecycle::ShapeDormant { shape: id.to_string() });
                tracing::debug!("shape {id} went dormant (idle)");
            }
            None => {
                // The sequencer didn't know the shape (or is gone): leave it active. Reset the
                // idle clock so the sweep backs off a full idle window instead of re-attempting
                // (and re-warning) every sweep.
                life.state = LifeState::Active;
                life.last_read = std::time::Instant::now();
                drop(lives);
                tracing::warn!("deactivating shape {id}: sequencer returned no resume state; left active");
            }
        }
        let _ = done_tx.send(true);
        Ok(())
    }

    /// Evict a shape: delete its record, share entries, lifecycle entry, and durable stream. A
    /// returning `/v1/shape` client gets `409 must-refetch`; an extended-API client gets `404` and
    /// recreates. Normally only **dormant** shapes are evicted; the exception is non-parkable
    /// shapes (subquery / aggregate — see [`crate::retention`]), which the TTL layer evicts
    /// straight from active with a full teardown. Rechecks eligibility under the locks — a
    /// reactivation or rejoin racing the sweep wins.
    pub(crate) async fn evict_shape(&self, id: &str, reason: EvictReason) -> Result<()> {
        let mut st = self.state.lock().await;
        let Some(rec) = st.shapes.get(id).cloned() else { return Ok(()) };
        let parkable = !rec.is_subquery && rec.aggregate.is_none();
        {
            let mut lives = self.lives.lock().unwrap();
            let evictable = match lives.get(id) {
                Some(life) if matches!(life.state, LifeState::Dormant { .. }) => true,
                // A non-parkable shape is evicted from active only if it is still idle past the
                // full grace window (a touch since the sweep snapshot wins).
                Some(life) if !parkable && matches!(life.state, LifeState::Active) => {
                    life.last_read.elapsed() >= self.retention.idle_timeout + self.retention.dormant_ttl
                }
                _ => false, // transitioning (or already evicted) since the sweep snapshot
            };
            if !evictable {
                return Ok(());
            }
            if st.feed_shares.get(id).is_some_and(|s| s.refcount > 0) {
                return Ok(());
            }
            lives.remove(id);
        }
        if let Some(share) = st.feed_shares.remove(id) {
            st.feed_by_sig.remove(&share.sig);
        }
        let removed = st.shapes.remove(id);
        st.circuit_placement.remove(id);
        if removed.is_some() {
            let _ = self.catalog_tx.send(CatalogEvent::Dropped { id: id.to_string() });
        }
        // A dormant shape is already unregistered from the sequencer; a non-parkable one is still
        // live and needs the full teardown (sequencer routing for aggregates, registry for subqueries).
        if !parkable {
            if let Some(seq) = st.sequencer.as_ref() {
                let _ = seq
                    .cmd_tx
                    .send(SequencerCmd::RemoveShape { table: rec.table.clone(), shape_id: id.to_string() });
            }
        }
        drop(st);
        if !parkable {
            self.subqueries.lock().await.drop_subquery_shape(id).await;
        }
        if let Some(rec) = removed {
            // Eviction is terminal (unlike deactivation), so the stream is retired: closed, then
            // deleted — a client still tailing it is released at once with `stream-closed`.
            if let Err(e) = self.ds.retire_stream(&rec.stream_path).await {
                tracing::warn!("failed to delete stream {} for evicted shape {id}: {e:#}", rec.stream_path);
            }
            metrics().shapes_evicted.fetch_add(1, Ordering::Relaxed);
            trace_lifecycle(&self.trace_tx, crate::trace::GraphLifecycle::ShapeDropped { shape: id.to_string() });
            tracing::info!("evicted shape {id} ({})", reason.as_str());
        }
        Ok(())
    }

    /// One retention sweep: snapshot every shape's status, run the pure layered policy
    /// ([`crate::retention::plan_sweep`]), then execute the plan. Public so a harness can force a
    /// sweep instead of waiting for the background interval.
    pub async fn retention_sweep(&self) {
        let cfg = self.retention.clone();
        let snapshot: Vec<SweepShape> = {
            let st = self.state.lock().await;
            let bytes = self.ds.appended_bytes_with_prefix("shape/");
            let lives = self.lives.lock().unwrap();
            st.shapes
                .values()
                .map(|rec| {
                    let life = lives.get(&rec.id);
                    let (idle, dormant_for, in_transition) = match life {
                        None => (std::time::Duration::ZERO, None, true), // mid-create; leave alone
                        Some(l) => match &l.state {
                            LifeState::Active => (l.last_read.elapsed(), None, false),
                            LifeState::Dormant { since, .. } => (l.last_read.elapsed(), Some(since.elapsed()), false),
                            LifeState::Deactivating { .. } | LifeState::Reactivating { .. } => {
                                (l.last_read.elapsed(), None, true)
                            }
                        },
                    };
                    SweepShape {
                        id: rec.id.clone(),
                        refcount: st.feed_shares.get(&rec.id).map(|s| s.refcount).unwrap_or(0),
                        idle,
                        dormant_for,
                        in_transition,
                        dormancy_eligible: !rec.is_subquery && rec.aggregate.is_none(),
                        stream_bytes: bytes.get(&rec.stream_path).copied().unwrap_or(0),
                    }
                })
                .collect()
        };
        let plan = crate::retention::plan_sweep(&cfg, &snapshot);
        if plan.over_capacity {
            metrics().retention_pressure.fetch_add(1, Ordering::Relaxed);
            tracing::error!(
                "retention: {} shapes exceed max_shapes={} but nothing dormant is left to evict — \
                 every shape is actively subscribed or recently read; raise ELECTRIC_CIRCUITS_MAX_SHAPES or lower the idle timeout",
                snapshot.len(),
                cfg.max_shapes
            );
        }
        if plan.over_budget {
            metrics().retention_pressure.fetch_add(1, Ordering::Relaxed);
            tracing::error!(
                "retention: shape streams exceed the disk budget ({} bytes) but nothing dormant is left to evict — \
                 raise ELECTRIC_CIRCUITS_SHAPE_DISK_BUDGET_MB or lower the idle timeout",
                cfg.disk_budget_bytes
            );
        }
        for id in &plan.deactivate {
            if let Err(e) = self.deactivate_shape(id).await {
                tracing::warn!("retention: deactivating shape {id} failed: {e:#}");
            }
        }
        for (id, reason) in &plan.evict {
            if let Err(e) = self.evict_shape(id, *reason).await {
                tracing::warn!("retention: evicting shape {id} failed: {e:#}");
            }
        }
    }

    /// Spawn (once) the background retention sweeper. Started lazily from the shape-create paths
    /// (and after a catalog restore) so library users that never create shapes never run it.
    pub(crate) fn ensure_retention_sweeper(&self) {
        if self.retention_started.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let engine = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(engine.retention.sweep_interval);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            tick.tick().await; // the first tick fires immediately; skip it
            loop {
                tick.tick().await;
                engine.retention_sweep().await;
            }
        });
    }

}

impl Engine {
    /// Orchestrate the registry's three-phase subquery-shape creation (see
    /// `SubqueryRegistry::begin_create`): the Postgres seeding queries and the outer backfill
    /// run WITHOUT the registry lock, so concurrent creates parallelize on the shared pool
    /// (`ELECTRIC_DB_POOL_SIZE`) instead of serializing behind one create's round-trips.
    /// A begin-conflict (sharing a node another create is still seeding) retries briefly.
    async fn create_subquery_three_phase(
        &self,
        id: &str,
        table: &str,
        stream_path: &str,
        where_json: &PredicateJson,
        out_cols: Option<Arc<Vec<usize>>>,
        changes_only: bool,
    ) -> Result<()> {
        // Phase A (brief lock), with conflict retry.
        let begin = {
            let mut attempt = 0u32;
            loop {
                let res = self.subqueries.lock().await.begin_create(
                    id, table, stream_path, where_json, out_cols.clone(), changes_only,
                );
                match res {
                    Ok(b) => break b,
                    Err(e) if e.to_string().contains("subquery create conflict") && attempt < 100 => {
                        attempt += 1;
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    }
                    Err(e) => return Err(e),
                }
            }
        };
        // Phase B (no registry lock): seed fresh nodes + backfill the shape, all from pooled PG.
        let phase_b = async {
            let mut node_seeds = Vec::with_capacity(begin.seeds.len());
            for (sig, inner_table, inner_where) in &begin.seeds {
                let ts = begin
                    .schemas
                    .get(inner_table)
                    .cloned()
                    .with_context(|| format!("seed: unknown inner table '{inner_table}'"))?;
                let wsql = inner_where
                    .as_ref()
                    .map(|w| crate::sql::predicate_json_to_sql(w, 1, &begin.schemas, inner_table));
                let client = crate::pg::pool_for(
                    self.pg_url.as_deref().context("subquery work requires postgres")?,
                )
                .get()
                .await?;
                let bf = crate::pg::backfill_where(&client, &ts, wsql).await?;
                node_seeds.push((sig.clone(), bf.rows, bf.gate));
            }
            let outer_ts = begin
                .schemas
                .get(table)
                .cloned()
                .with_context(|| format!("unknown outer table '{table}'"))?;
            let (outer_gate, seeded, seeded_pks) = if changes_only {
                (crate::pg::SnapshotGate::passthrough(), 0u64, HashSet::new())
            } else {
                let (wsql, params) =
                    crate::sql::predicate_json_to_sql(where_json, 1, &begin.schemas, table);
                let client = crate::pg::pool_for(
                    self.pg_url.as_deref().context("subquery work requires postgres")?,
                )
                .get()
                .await?;
                let bf = crate::pg::backfill_where(&client, &outer_ts, Some((wsql, params))).await?;
                let seeded_pks: HashSet<String> =
                    bf.rows.iter().map(|r| outer_ts.key_string(r).unwrap_or_default()).collect();
                let out: Vec<(Row, ZWeight)> = bf.rows.iter().map(|r| (r.clone(), 1)).collect();
                let mut seeded = 0u64;
                if !out.is_empty() {
                    let envs = translate_output(
                        &outer_ts,
                        out,
                        None,
                        None,
                        out_cols.as_deref().map(Vec::as_slice),
                    );
                    self.ds.append(stream_path, &envs).await?;
                    seeded = envs.len() as u64;
                }
                (bf.gate, seeded, seeded_pks)
            };
            Ok::<_, anyhow::Error>((node_seeds, outer_gate, seeded, seeded_pks))
        }
        .await;
        // Phase C (brief lock): install + gated replay. A failure in either phase unwinds through
        // the caller's create rollback (which reaches the registry via `abort_create`), so there is
        // exactly one undo path whether the create failed or was cancelled.
        let (node_seeds, outer_gate, seeded, seeded_pks) = phase_b?;
        let finished = self
            .subqueries
            .lock()
            .await
            .finish_create(id, node_seeds, outer_gate, seeded, seeded_pks)
            .await?;
        let crate::subquery::FinishedCreate { work, deferred, node_work } = finished;
        if !node_work.is_empty() {
            // Re-derivations a child node's flip aimed at one of THIS create's fresh nodes while it
            // was still seeding: reconciling then would have run against an empty set, and the seed
            // (from an older snapshot) would have been installed over the change. Handed off first
            // so the node's set is right before anything reads it — though the order against the
            // shape-deferred hand-off below is not load-bearing: emission is absolute per pk, and
            // this walk re-derives every dependent of the node anyway, so whichever of the two runs
            // last evaluates against the reconciled set.
            self.pending_flips.fetch_add(1, Ordering::SeqCst);
            if self.flip_tx.send(FlipWork::DeferredNode { work: node_work }).is_err() {
                self.pending_flips.fetch_sub(1, Ordering::SeqCst);
            }
        }
        if !work.is_empty() {
            // Replay flips propagate exactly like live ones (barrier-covered).
            self.pending_flips.fetch_add(1, Ordering::SeqCst);
            if self.flip_tx.send(FlipWork::Walk { work, txid: None, lsn: None }).is_err() {
                self.pending_flips.fetch_sub(1, Ordering::SeqCst);
            }
        }
        if !deferred.is_empty() {
            // Flips that reached this shape's edges while it was still pending — a shared inner
            // node's, since a fresh node's deltas buffer on the node itself. They carry membership
            // the phase-B snapshot could not contain, so they are barrier-covered like any other
            // effect: counted before the hand-off, released only once they land.
            self.pending_flips.fetch_add(1, Ordering::SeqCst);
            if self
                .flip_tx
                .send(FlipWork::Deferred { shape_id: id.to_string(), work: deferred })
                .is_err()
            {
                self.pending_flips.fetch_sub(1, Ordering::SeqCst);
            }
        }
        Ok(())
    }
}

/// Which per-kind registration a rolled-back create must undo alongside the engine-state entries.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Registration {
    /// Plain + aggregate shapes: the sequencer's entry for the shape, pending or live.
    Sequencer,
    /// Subquery shapes: the cross-table registry's entry, pending or installed.
    Registry,
}

/// Everything a create has registered before its first storage/registry await, given back if the
/// create never reaches its own end.
///
/// The create's future is awaited straight from the HTTP handler, so a client that disconnects
/// takes it away mid-flight — during the backfill, a stream append, the subquery conflict retry, or
/// phase C. Without this, the half-made shape stays in `feed_by_sig` forever: every later identical
/// create joins it and waits on a creator that no longer exists (and bumps its refcount, so
/// retention never evicts it either), while its sequencer/registry entry keeps buffering deltas for
/// a shape nobody can read.
struct CreateGuard {
    engine: Engine,
    shape_id: String,
    table: String,
    stream_path: String,
    registration: Registration,
    armed: bool,
}

impl CreateGuard {
    fn new(
        engine: &Engine,
        shape_id: &str,
        table: &str,
        stream_path: &str,
        registration: Registration,
    ) -> Self {
        Self {
            engine: engine.clone(),
            shape_id: shape_id.to_string(),
            table: table.to_string(),
            stream_path: stream_path.to_string(),
            registration,
            armed: true,
        }
    }

    /// The create reached its end: everything it registered stays.
    fn complete(&mut self) {
        self.armed = false;
    }

    /// The create failed and its caller is still there to be told: roll back in place, so the error
    /// the caller returns is already true of the engine's state.
    async fn rollback(&mut self) {
        self.armed = false;
        self.engine
            .rollback_create(&self.shape_id, &self.table, &self.stream_path, self.registration)
            .await;
    }
}

impl Drop for CreateGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // Cancelled. The rollback needs the async engine/registry locks, which `drop` cannot await,
        // so it runs DETACHED — the same shape as the reactivation path, and for the same reason.
        tracing::warn!("create of shape '{}' was cancelled; rolling back", self.shape_id);
        let (engine, shape_id, table, stream_path, registration) = (
            self.engine.clone(),
            self.shape_id.clone(),
            self.table.clone(),
            self.stream_path.clone(),
            self.registration,
        );
        tokio::spawn(async move {
            engine.rollback_create(&shape_id, &table, &stream_path, registration).await;
        });
    }
}

impl Engine {
    /// The second half of a create's degrade gate: taken after the create's work is done and before
    /// it hands back a handle. `Err(Degraded)` means the caller must roll the create back and answer
    /// 503 rather than return the record.
    ///
    /// **Why this check plus the one under the state lock is sufficient.** The degradation reaper
    /// snapshots every registered subquery stream under the state lock
    /// (`Engine::reap_subquery_streams`) and deletes exactly those. Every create checks `degraded`
    /// under that same lock in the same critical section that registers its record. So, against that
    /// snapshot, a create either
    ///
    /// * observed `degraded` under the lock and never registered — nothing to reap, nothing handed
    ///   back; or
    /// * registered BEFORE the snapshot, so its stream is IN the snapshot and will be deleted — and
    ///   this final check, which runs after the create's own work finished and therefore after the
    ///   mark that caused the snapshot, sees `degraded` and refuses.
    ///
    /// A successful create can therefore never return a handle whose stream the reaper has already
    /// deleted. The one window left is a mark landing after this check: indistinguishable from a
    /// degradation an instant after the response, which every client must handle anyway — and that
    /// create's stream is registered, so the reaper deletes it like any other.
    fn ensure_create_not_degraded(&self) -> Result<()> {
        self.ensure_not_degraded()
    }

    /// Undo a create: the one implementation of "this shape was never made", shared by the explicit
    /// error paths and by [`CreateGuard`]'s cancelled path.
    ///
    /// Engine state goes first: the share signature is what a retrying client contends on, and the
    /// per-kind entry left behind for the extra moment only buffers deltas for a shape nobody can
    /// reach. The sequencer command is `RemoveShape`, not `AbortShape`: a cancellation can land
    /// after `ActivateShape` has already turned the pending buffer into live routing, and only
    /// `RemoveShape` covers both states (it drops the pending buffer, the routed/standalone/
    /// aggregate registration, and the emit counter); commands are FIFO, so one sent while
    /// `BeginShape` is still queued still lands after it.
    async fn rollback_create(&self, id: &str, table: &str, stream_path: &str, registration: Registration) {
        let mut st = self.state.lock().await;
        let existed = st.shapes.remove(id).is_some();
        st.circuit_placement.remove(id);
        if let Some(share) = st.feed_shares.remove(id) {
            // Only if the signature still points HERE: a joiner woken by this create's failure may
            // already have registered its own replacement under the same signature.
            if st.feed_by_sig.get(&share.sig).is_some_and(|cur| cur == id) {
                st.feed_by_sig.remove(&share.sig);
            }
        }
        if registration == Registration::Sequencer
            && let Some(seq) = st.sequencer.as_ref()
        {
            let _ = seq
                .cmd_tx
                .send(SequencerCmd::RemoveShape { table: table.to_string(), shape_id: id.to_string() });
        }
        drop(st);
        self.lives.lock().unwrap().remove(id);
        if existed {
            let _ = self.catalog_tx.send(CatalogEvent::Dropped { id: id.to_string() });
        }
        if registration == Registration::Registry {
            self.subqueries.lock().await.abort_create(id).await;
        }
        // Deleted, not retired: the create never returned, so the stream was never handed to a
        // subscriber and there is no one to signal with a close.
        let _ = self.ds.delete_stream(stream_path).await;
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;

    /// A subquery-capable engine with no durable-streams server behind it: the rollback's stream
    /// DELETE fails and is ignored, which leaves exactly the in-memory state these tests assert on.
    async fn engine_with_subquery_tables() -> (Engine, PredicateJson) {
        let engine = Engine::new(DsClient::new("http://127.0.0.1:1"));
        let schema: Schema = serde_json::from_value(serde_json::json!({
            "tables": {
                "outer_t": { "columns": { "id": {"type":"int"}, "gid": {"type":"int"} }, "primaryKey": "id" },
                "inner_t": { "columns": { "id": {"type":"int"}, "gid": {"type":"int"} }, "primaryKey": "id" }
            }
        }))
        .unwrap();
        let compiled = compile_schema(&schema).unwrap();
        engine.subqueries.lock().await.set_schemas(Arc::new(compiled.clone()));
        *engine.tables_shared.write().unwrap() = compiled.clone();
        engine.state.lock().await.tables = compiled;
        let where_json = serde_json::from_value(serde_json::json!({
            "col": "gid", "in": { "table": "inner_t", "project": "gid" }
        }))
        .unwrap();
        (engine, where_json)
    }

    /// Everything `create_shape`'s subquery path registers before its first await, plus the guard
    /// it arms there.
    async fn register(engine: &Engine, id: &str, where_json: &PredicateJson) -> CreateGuard {
        let mut st = engine.state.lock().await;
        st.shapes.insert(id.to_string(), ShapeRecord {
            id: id.to_string(),
            table: "outer_t".into(),
            stream_path: format!("shape/{id}"),
            changes_only: false,
            where_json: Some(where_json.clone()),
            columns: None,
            family_key: None,
            is_subquery: true,
            aggregate: None,
        });
        let (_ready_tx, ready) = tokio::sync::watch::channel(ShareOutcome::Pending);
        st.feed_by_sig.insert("sig".into(), id.to_string());
        st.feed_shares.insert(id.to_string(), FeedShare { sig: "sig".into(), refcount: 1, ready });
        drop(st);
        engine.lives.lock().unwrap().insert(id.to_string(), ShapeLife::active());
        CreateGuard::new(engine, id, "outer_t", &format!("shape/{id}"), Registration::Registry)
    }

    /// The detached rollback must leave nothing a later identical create could join or conflict with.
    async fn assert_rolled_back(engine: &Engine, id: &str) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while engine.get_shape(id).await.is_some() || engine.subqueries.lock().await.touches("outer_t") {
            assert!(std::time::Instant::now() < deadline, "the cancelled create was never rolled back");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let st = engine.state.lock().await;
        assert!(st.feed_by_sig.is_empty(), "the share signature must not outlive the create");
        assert!(st.feed_shares.is_empty());
        assert!(engine.lives.lock().unwrap().is_empty(), "the retention entry must go too");
        assert!(engine.subquery_stats().await.is_empty(), "no registry node may survive");
    }

    /// Cancelled between `begin_create` and `finish_create`: the registry holds a pending shape
    /// buffering outer deltas and a fresh node buffering inner ones. Both must go, or the next
    /// identical create conflicts on that half-seeded node until its retry budget runs out.
    #[tokio::test(flavor = "multi_thread")]
    async fn cancelled_before_install_unwinds_the_pending_registry_state() {
        let (engine, where_json) = engine_with_subquery_tables().await;
        let guard = register(&engine, "s1", &where_json).await;
        let begin = engine
            .subqueries
            .lock()
            .await
            .begin_create("s1", "outer_t", "shape/s1", &where_json, None, false)
            .unwrap();
        assert_eq!(begin.seeds.len(), 1, "one fresh node to seed");

        drop(guard);
        assert_rolled_back(&engine, "s1").await;
    }

    /// Cancelled after `finish_create` installed the shape (its phase-C replay awaits, and the HTTP
    /// response is still one await away): the rollback must go through the ordinary drop path so the
    /// installed shape, its index entry and its feed go with the engine-side state.
    #[tokio::test(flavor = "multi_thread")]
    async fn cancelled_after_install_drops_the_registered_shape() {
        let (engine, where_json) = engine_with_subquery_tables().await;
        let guard = register(&engine, "s1", &where_json).await;
        let begin = engine
            .subqueries
            .lock()
            .await
            .begin_create("s1", "outer_t", "shape/s1", &where_json, None, false)
            .unwrap();
        let seeds = begin
            .seeds
            .iter()
            .map(|(sig, _, _)| (sig.clone(), Vec::new(), crate::pg::SnapshotGate::passthrough()))
            .collect();
        engine
            .subqueries
            .lock()
            .await
            .finish_create("s1", seeds, crate::pg::SnapshotGate::passthrough(), 0, HashSet::new())
            .await
            .unwrap();
        assert!(engine.subqueries.lock().await.shapes.contains_key("s1"), "installed before the drop");

        drop(guard);
        assert_rolled_back(&engine, "s1").await;
        assert!(engine.graph().await.shapes.is_empty());
    }

    /// Cancelled in the MIDDLE of phase C: the fresh node's seed is already asserted into the
    /// membership circuit and the shape is not installed. The pending entry never left the registry,
    /// so the detached rollback finds it and unwinds the whole create — the partial seed's
    /// contributor tuples included. Without that, the leaked half-seeded node makes every later
    /// create sharing it conflict until the engine restarts.
    #[tokio::test(flavor = "multi_thread")]
    async fn cancelled_mid_phase_c_unwinds_the_partly_seeded_node() {
        let (engine, where_json) = engine_with_subquery_tables().await;
        let guard = register(&engine, "s1", &where_json).await;
        let node_id = {
            let mut reg = engine.subqueries.lock().await;
            let begin =
                reg.begin_create("s1", "outer_t", "shape/s1", &where_json, None, false).unwrap();
            let sig = begin.seeds[0].0.clone();
            reg.assert_seed_row_for_test(&sig, "1", Value::Int(7)).await;
            let node_id = reg.nodes[&sig].node_id;
            assert_eq!(reg.circuit_distinct(node_id), 1, "the partial seed really landed");
            node_id
        };

        drop(guard);
        assert_rolled_back(&engine, "s1").await;
        assert_eq!(
            engine.subqueries.lock().await.circuit_distinct(node_id),
            0,
            "the partial seed was retracted with the node it belonged to"
        );
    }

    // --- the sharing rendezvous carries the creator's REASON ------------------------------------

    /// Everything an in-flight shared create leaves behind for a joiner to find: the shape record,
    /// the signature a later identical create matches on, and the share entry whose outcome the
    /// joiner waits for. Returns the creator's end of that outcome channel.
    async fn pending_share(
        engine: &Engine,
        id: &str,
        where_json: &PredicateJson,
    ) -> tokio::sync::watch::Sender<ShareOutcome> {
        let sig = shape_signature("outer_t", &Some(where_json.clone()), &None, false);
        let (tx, rx) = tokio::sync::watch::channel(ShareOutcome::Pending);
        let mut st = engine.state.lock().await;
        st.shapes.insert(id.to_string(), ShapeRecord {
            id: id.to_string(),
            table: "outer_t".into(),
            stream_path: format!("shape/{id}"),
            changes_only: false,
            where_json: Some(where_json.clone()),
            columns: None,
            family_key: None,
            is_subquery: true,
            aggregate: None,
        });
        st.feed_by_sig.insert(sig.clone(), id.to_string());
        st.feed_shares.insert(id.to_string(), FeedShare { sig, refcount: 1, ready: rx });
        tx
    }

    async fn refcount(engine: &Engine, id: &str) -> usize {
        engine.state.lock().await.feed_shares.get(id).map(|s| s.refcount).unwrap_or(0)
    }

    /// Park until the joiner has actually joined (its refcount is taken) and is waiting on the
    /// creator's outcome, so the outcome below is published INTO that wait.
    async fn await_joined(engine: &Engine, id: &str) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while refcount(engine, id).await < 2 {
            assert!(std::time::Instant::now() < deadline, "the joiner never joined the share");
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    fn join(engine: &Engine, where_json: &PredicateJson) -> tokio::task::JoinHandle<Result<ShapeRecord>> {
        let engine = engine.clone();
        let where_json = where_json.clone();
        tokio::spawn(async move { engine.create_shape("outer_t", Some(where_json), None, false, true).await })
    }

    /// A creator that refuses because the engine degraded publishes that REASON, so its joiner
    /// answers with the same typed error (503) instead of the generic "failed to initialize"
    /// (500). Two identical requests from identical clients must not disagree about why the
    /// engine said no.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_joiner_of_a_degraded_create_gets_the_typed_refusal() {
        let (engine, where_json) = engine_with_subquery_tables().await;
        let ready_tx = pending_share(&engine, "s1", &where_json).await;
        let joining = join(&engine, &where_json);
        await_joined(&engine, "s1").await;

        let _ = ready_tx.send(ShareOutcome::Degraded);
        let err = joining.await.unwrap().expect_err("the joiner must be refused");
        assert!(
            err.downcast_ref::<Degraded>().is_some(),
            "the joiner must get the creator's typed refusal, not a generic failure: {err:#}"
        );
    }

    /// The other half: the creator finished a moment BEFORE the mark, so the outcome is `Ready` —
    /// but the reaper is about to delete the stream that handle points at. The joiner re-checks
    /// the same latch the creator's final check uses, and gives back the refcount it took (a
    /// refused join must not pin the shape).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_joiner_ready_after_the_mark_is_refused_and_gives_back_its_refcount() {
        let (engine, where_json) = engine_with_subquery_tables().await;
        let ready_tx = pending_share(&engine, "s1", &where_json).await;
        let joining = join(&engine, &where_json);
        await_joined(&engine, "s1").await;

        engine.force_degraded();
        let _ = ready_tx.send(ShareOutcome::Ready);
        let err = joining.await.unwrap().expect_err("the joiner must be refused");
        assert!(err.downcast_ref::<Degraded>().is_some(), "and typed, like the creator's: {err:#}");
        assert_eq!(refcount(&engine, "s1").await, 1, "the refused join released its subscription");
    }

    /// An ordinary creation failure stays an ordinary failure: only degradation is special-cased,
    /// so a retryable init failure must not start answering 503.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_joiner_of_a_failed_create_still_gets_the_generic_error() {
        let (engine, where_json) = engine_with_subquery_tables().await;
        let ready_tx = pending_share(&engine, "s1", &where_json).await;
        let joining = join(&engine, &where_json);
        await_joined(&engine, "s1").await;

        let _ = ready_tx.send(ShareOutcome::Failed);
        let err = joining.await.unwrap().expect_err("the joiner must be refused");
        assert!(err.downcast_ref::<Degraded>().is_none(), "not a degradation: {err:#}");
        assert!(format!("{err:#}").contains("failed to initialize"), "{err:#}");
    }
}
