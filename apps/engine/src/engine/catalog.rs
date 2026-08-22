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
    /// The shape went dormant: routing state dropped, stream + record retained. `resume` is the
    /// change-log position its stream is complete up to — a `(segment, offset)` pair since
    /// ADR-0006, because the offset alone is meaningless once the log has rotated; `gate` is its
    /// original backfill-snapshot fence. Restores as dormant (an improvement over the
    /// in-memory-only lifecycle: a restart no longer forgets dormant shapes).
    Dormant { id: String, resume: LogPosition, gate: crate::pg::SnapshotGate },
    /// A dormant shape was reactivated (replayed + re-registered).
    Reactivated { id: String },
    Dropped { id: String },
    /// The sequencer's checkpoint: the change-log position a restart replays from, plus the
    /// `(lsn, seq)` de-duplication highwater it had applied up to at that position (ADR-0003).
    ///
    /// The highwater has to travel WITH the position. A commit too large for one request body is
    /// appended in several chunks, so a crash can leave a prefix of a transaction applied and
    /// checkpointed while the rest is re-delivered; without the restored highwater that prefix would
    /// be applied twice, and aggregate/subquery contributor weights are not idempotent under
    /// duplicates. `None` is a checkpoint taken before anything was applied (a fresh boot).
    Offset {
        pos: LogPosition,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        highwater: Option<(u64, u64)>,
    },
    /// The change log rotated (ADR-0006): `segment` became the CURRENT segment at `at` (unix
    /// seconds). Written on the first creation of segment 0 too, so every segment's start time is
    /// in the log — segment `n` was closed when `n+1` began, which is what the retain window that
    /// governs deletion measures against. The fold's last one is the current segment.
    ChangesRotated { segment: u32, at: u64 },
    /// The retention sweeper deleted a rotated-out change-log segment (ADR-0006). Without it the
    /// fold's segment set would keep every segment the log ever had, so every restart would re-plan
    /// retiring streams that are long gone (harmless — a delete of an absent stream is a no-op — but
    /// it would also make the `changes_segments_retained` gauge wrong until the first sweep).
    ChangesSegmentDeleted { segment: u32 },
    /// **Audit only**: a table's schema drifted and was re-introspected (ADR-0005). The restore
    /// ignores it — every dependent shape of the table was retired by the same handler, so it is
    /// already `Dropped` in the log. It is written so the durable record explains *why* a swathe of
    /// shapes disappeared at a given point.
    SchemaChanged { table: TableRef, fingerprint: crate::schema::SchemaFingerprint },
    /// The engine created (or first adopted) its replication slot: the **epoch** every shape after
    /// this point in the log belongs to (ADR-0004). The LAST one wins — a reset appends a new one
    /// after the `Dropped` records of the epoch it ended, so a fold reads "these shapes, in this
    /// epoch" straight off the log.
    SlotBound(crate::engine::epoch::SlotBinding),
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
    /// The last `Offset` checkpoint that has actually **landed in storage** — what a restart would
    /// resume from, as opposed to what this process has processed in memory.
    ///
    /// It is the floor for change-log segment deletion (ADR-0006), and it has to be: the sequencer
    /// publishes its in-memory position the instant it crosses a segment boundary, but the
    /// checkpoint is an async append. Deleting on the in-memory position would let a sweep delete
    /// segment `n` while the durable checkpoint still says `(n, X)` — and a crash in that window
    /// leaves a boot resuming inside a stream that no longer exists.
    durable: Arc<std::sync::Mutex<Option<LogPosition>>>,
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

    /// The last change-log position whose `Offset` checkpoint is durable (see [`Self::durable`]).
    pub(crate) fn durable_offset(&self) -> Option<LogPosition> {
        self.durable.lock().unwrap().clone()
    }

    /// Adopt the checkpoint the boot restored from the catalog: it is by definition already durable,
    /// and without it nothing could be deleted until this process wrote its first checkpoint.
    pub(crate) fn seed_durable_offset(&self, pos: LogPosition) {
        let mut g = self.durable.lock().unwrap();
        if g.as_ref().is_none_or(|cur| *cur < pos) {
            *g = Some(pos);
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
    let durable: Arc<std::sync::Mutex<Option<LogPosition>>> = Arc::new(std::sync::Mutex::new(None));
    let landed = durable.clone();
    tokio::spawn(async move {
        let mut ensured = false;
        while let Some(ev) = rx.recv().await {
            if !ensured {
                ensured = self::ensure_catalog(&ds).await;
            }
            // Published only on a SUCCESSFUL append: an `Offset` whose write failed is not a
            // position a restart would resume from, and treating it as one would license deleting
            // the segment underneath it.
            let checkpoint = match &ev {
                CatalogEvent::Offset { pos, .. } => Some(pos.clone()),
                _ => None,
            };
            match serde_json::to_value(&ev) {
                Ok(json) => match ds.append_json(CATALOG_STREAM, &[json]).await {
                    Ok(()) => {
                        if let Some(pos) = checkpoint {
                            let mut g = landed.lock().unwrap();
                            if g.as_ref().is_none_or(|cur| *cur < pos) {
                                *g = Some(pos);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("catalog append failed (event lost; restart may under-restore): {e:#}")
                    }
                },
                Err(e) => tracing::error!("catalog event could not be serialized (event lost): {e:#}"),
            }
            counter.fetch_sub(1, Ordering::SeqCst);
        }
    });
    CatalogWriter { tx, in_flight, durable }
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

/// The durable catalog holds a change-log position written **before** ADR-0006 (a bare offset
/// string, from when the log was one un-segmented `changes` stream).
///
/// Fatal for the same reason [`CatalogPredatesQualification`] is: the value cannot be repaired, and
/// guessing would be worse than not booting. A bare offset is a byte position in a stream that no
/// longer exists under that name; adopting it as "segment 0" would resume the sequencer — or a
/// dormant shape — at an unrelated point in a different segment's byte space, silently replaying or
/// silently skipping an arbitrary span of changes. Greenfield: no such catalog can exist except
/// from a pre-cutover build, and recovery is a deliberate human act (reset the storage).
#[derive(Debug)]
pub struct CatalogPredatesSegmentation {
    detail: String,
}

impl std::fmt::Display for CatalogPredatesSegmentation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "durable catalog predates ADR-0006 ({}); reset the durable-streams data directory",
            self.detail
        )
    }
}

impl std::error::Error for CatalogPredatesSegmentation {}

/// Is this raw catalog event a pre-ADR-0006 change-log position? `Some(detail)` names it.
///
/// Deliberately positive-checking the OLD spelling rather than trusting the strict deserializer to
/// fail: an unparseable event is otherwise silently skipped by the fold, which for an `Offset` would
/// silently restart the sequencer from the beginning of the log.
fn predates_segmentation(ev: &serde_json::Value) -> Option<String> {
    match ev.get("t").and_then(serde_json::Value::as_str) {
        Some("offset") if ev.get("offset").is_some_and(serde_json::Value::is_string) => {
            Some(format!("bare change-log offset {}", ev["offset"]))
        }
        Some("dormant") if ev.get("resume_offset").is_some_and(serde_json::Value::is_string) => Some(format!(
            "bare dormant resume offset {} for shape {}",
            ev["resume_offset"],
            ev.get("id").and_then(serde_json::Value::as_str).unwrap_or("<unknown>")
        )),
        _ => None,
    }
}

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


/// One shape as the fold reconstructed it: (record, sharing signature, refcount, dormant resume
/// state). The last `Dormant`/`Reactivated` event wins.
type Restored = (ShapeRecord, Option<String>, usize, Option<(LogPosition, crate::pg::SnapshotGate)>);

/// The durable catalog, folded — everything a boot needs before it decides what to *do* with it.
///
/// Reading and deciding are separate because the epoch check has to happen between them (ADR-0004):
/// the binding to verify against is in the log, and no shape may be resumed before the verdict is
/// in.
pub(crate) struct CatalogFold {
    recs: HashMap<String, Restored>,
    /// The sequencer's change-log replay start (the last `Offset` checkpoint).
    start_pos: LogPosition,
    /// The `(lsn, seq)` de-duplication highwater recorded with that checkpoint (ADR-0003).
    start_highwater: Option<(u64, u64)>,
    /// The change log's current segment: the last `ChangesRotated`, else 0 (ADR-0006). A lower
    /// bound — a process can die between closing a segment and recording the rotation — so the
    /// boot walks forward from here (`changelog::resolve_current`).
    pub(crate) current_segment: u32,
    /// Every segment's start time (unix seconds), for the retain window that governs deletion.
    pub(crate) segment_starts: std::collections::BTreeMap<u32, u64>,
    /// The last `SlotBound`: the epoch these shapes belong to. `None` = nothing ever claimed one,
    /// which is a genuine first boot.
    pub(crate) binding: Option<crate::engine::epoch::SlotBinding>,
}

impl Default for CatalogFold {
    fn default() -> Self {
        CatalogFold {
            recs: HashMap::new(),
            start_pos: LogPosition::start(),
            start_highwater: None,
            binding: None,
            current_segment: 0,
            segment_starts: std::collections::BTreeMap::new(),
        }
    }
}

impl CatalogFold {
    /// Fold one event in. Pure (no engine, no IO), so the log's semantics — last-writer-wins for the
    /// epoch and the offset, remove-on-drop for the shapes — are unit-testable.
    fn apply(&mut self, ev: CatalogEvent) {
        match ev {
            CatalogEvent::Created { rec, sig } => {
                self.recs.insert(rec.id.clone(), (rec, sig, 1, None));
            }
            CatalogEvent::Joined { id } => {
                if let Some(e) = self.recs.get_mut(&id) {
                    e.2 += 1;
                }
            }
            CatalogEvent::Left { id } => {
                if let Some(e) = self.recs.get_mut(&id) {
                    e.2 = e.2.saturating_sub(1);
                }
            }
            CatalogEvent::Dormant { id, resume, gate } => {
                if let Some(e) = self.recs.get_mut(&id) {
                    e.3 = Some((resume, gate));
                }
            }
            CatalogEvent::Reactivated { id } => {
                if let Some(e) = self.recs.get_mut(&id) {
                    e.3 = None;
                }
            }
            CatalogEvent::Dropped { id } => {
                self.recs.remove(&id);
            }
            CatalogEvent::Offset { pos, highwater } => {
                self.start_pos = pos;
                self.start_highwater = highwater;
            }
            // The LAST rotation is the current segment; every one of them is kept, because the
            // retain window needs to know when each segment began (see the variant).
            CatalogEvent::ChangesRotated { segment, at } => {
                self.current_segment = self.current_segment.max(segment);
                self.segment_starts.insert(segment, at);
            }
            // A deleted segment leaves the set but never moves `current_segment`: the current one
            // is never deleted, so the last rotation still names it.
            CatalogEvent::ChangesSegmentDeleted { segment } => {
                self.segment_starts.remove(&segment);
            }
            // Audit only (see the variant): the shapes it explains are already `Dropped`,
            // and the boot's own introspection is the authority on the schema.
            CatalogEvent::SchemaChanged { .. } => {}
            // The LAST binding is the epoch in force. A reset appends its new one after the
            // `Dropped` records of the epoch it ended, so the two always agree.
            CatalogEvent::SlotBound(binding) => self.binding = Some(binding),
        }
    }

    /// The sequencer's restored change-log position — the segment the boot must vouch for before
    /// anything resumes (see `Engine::init_change_log`).
    pub(crate) fn start_pos(&self) -> LogPosition {
        self.start_pos.clone()
    }

    /// Nothing was ever written (or everything was dropped and nothing checkpointed).
    fn is_empty(&self) -> bool {
        self.recs.is_empty() && self.start_pos == LogPosition::start()
    }
}

/// How much of a folded catalog a boot actually installs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RestoreMode {
    /// The ordinary boot: restore the records and re-register them with the sequencer.
    Resume,
    /// The epoch broke (ADR-0004). Restore the shape RECORDS and nothing else — no resume, no
    /// sequencer registration, no retention lifecycle, and above all no teardown of its own.
    ///
    /// The records are what a reset needs in order to retire the shapes *properly* (close the
    /// stream, then delete it, and write `Dropped`), whether that reset happens immediately (the
    /// auto policy) or when an operator posts `/epoch/reset`. Nothing is resumed, so no old-epoch
    /// shape is ever maintained for an instant; nothing is destroyed either, so a refusing engine
    /// has not thrown anything away before the human said so.
    Park,
}

impl Engine {
    /// Read the durable shape catalog and fold it. No engine state is touched — see
    /// [`Self::apply_catalog`] for that half.
    pub(crate) async fn fold_catalog(&self) -> Result<CatalogFold> {
        let mut fold = CatalogFold::default();
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
                // Same stance for a pre-segmentation change-log position (ADR-0006): refuse the
                // boot naming it, rather than let the strict deserializer turn it into a silently
                // skipped event.
                if let Some(detail) = predates_segmentation(&ev) {
                    return Err(anyhow::Error::new(CatalogPredatesSegmentation { detail }));
                }
                let Ok(ev) = serde_json::from_value::<CatalogEvent>(ev) else { continue };
                fold.apply(ev);
            }
            match next {
                Some(n) if !up_to_date && n != off => off = n,
                _ => break,
            }
        }
        Ok(fold)
    }

    /// Install a folded catalog: re-register every restorable shape with the (not yet spawned)
    /// sequencer — see [`CATALOG_STREAM`] for the restore semantics per shape kind — or, in
    /// [`RestoreMode::Park`], record them and stop there.
    pub(crate) async fn apply_catalog(
        &self,
        fold: CatalogFold,
        compiled: &HashMap<TableRef, TableSchema>,
        mode: RestoreMode,
    ) -> Result<()> {
        if fold.is_empty() {
            return Ok(());
        }
        let CatalogFold { recs, start_pos, start_highwater, .. } = fold;
        tracing::info!("catalog restore: {} shape(s), change-log replay from {start_pos}", recs.len());
        // The restored checkpoint IS durable (it was read back out of the log), so it is the
        // segment-deletion floor from boot rather than from this process's first checkpoint.
        self.catalog_tx.seed_durable_offset(start_pos.clone());
        *self.seq_start.lock().unwrap() = start_pos;
        // The de-duplication highwater is restored with the position (ADR-0003), so a prefix of a
        // chunked commit that was applied and checkpointed before a crash is not applied twice when
        // Postgres re-delivers the transaction.
        *self.seq_highwater.lock().unwrap() = start_highwater;

        if mode == RestoreMode::Park {
            // The epoch broke: hold the records, touch nothing else (see `RestoreMode::Park`).
            let mut st = self.state.lock().await;
            for (id, (rec, _, _, _)) in recs {
                if let Ok(num) = id.trim_start_matches('s').parse::<u64>() {
                    st.next_shape_id = st.next_shape_id.max(num + 1);
                }
                st.shapes.insert(id, rec);
            }
            tracing::warn!(
                "catalog restore: {} shape(s) parked over a broken epoch — none is resumed or \
                 maintained; they exist only to be retired by the reset",
                st.shapes.len()
            );
            return Ok(());
        }

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
                    Some((resume_at, gate)) => {
                        self.lives.lock().unwrap().insert(
                            id.clone(),
                            ShapeLife {
                                last_read: std::time::Instant::now(),
                                state: LifeState::Dormant {
                                    since: std::time::Instant::now(),
                                    resume: resume_at,
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

    /// Re-register one restored shape with the sequencer (the resume half of `apply_catalog`).
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
        let (kind, changes_only, aggregate) = match &rec.aggregate {
            Some(a) => {
                let col = a.col.as_deref().map(|c| ts.column_index(c)).transpose()?;
                (CreateKind::Aggregate { func: a.func, col }, false, Some((a.func, col)))
            }
            None => (CreateKind::Plain, true, None),
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
            aggregate,
            &self.shutdown_token(),
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

    fn bound(slot: &str, sysid: &str, at: &str) -> CatalogEvent {
        CatalogEvent::SlotBound(crate::engine::epoch::SlotBinding {
            system_identifier: sysid.to_string(),
            timeline_id: 1,
            slot: slot.to_string(),
            bound_at: at.to_string(),
        })
    }

    fn fold_of(events: Vec<CatalogEvent>) -> CatalogFold {
        let mut fold = CatalogFold::default();
        for ev in events {
            fold.apply(ev);
        }
        fold
    }

    fn pos(segment: u32, offset: &str) -> LogPosition {
        LogPosition { segment, offset: offset.to_string() }
    }

    /// A catalog with no `SlotBound` anywhere is a genuine first boot — the state that licenses the
    /// engine to create a slot from nothing (ADR-0004).
    #[test]
    fn a_catalog_without_a_binding_folds_to_no_epoch() {
        let fold = fold_of(vec![CatalogEvent::Offset { pos: pos(0, "42"), highwater: None }]);
        assert!(fold.binding.is_none());
        assert_eq!(fold.start_pos, pos(0, "42"));
        // Nothing at all folds to the empty catalog, which the restore skips outright.
        assert!(CatalogFold::default().is_empty());
    }

    /// The de-duplication highwater is checkpointed WITH the position (ADR-0003), and the fold's
    /// last `Offset` wins for both together.
    ///
    /// It has to travel with the position because a commit too large for one request body is
    /// appended in several chunks: a crash can leave a prefix of a transaction applied and
    /// checkpointed while the rest is still to be re-delivered. Restoring the position without the
    /// highwater would re-apply that prefix — and aggregate/subquery contributor weights are not
    /// idempotent under duplicates.
    #[test]
    fn the_dedup_highwater_is_restored_with_the_checkpoint_it_was_taken_at() {
        let fold = fold_of(vec![
            CatalogEvent::Offset { pos: pos(0, "10"), highwater: Some((0x10, 3)) },
            CatalogEvent::Offset { pos: pos(0, "20"), highwater: Some((0x20, 7)) },
        ]);
        assert_eq!(fold.start_pos, pos(0, "20"));
        assert_eq!(fold.start_highwater, Some((0x20, 7)), "the last checkpoint's highwater, not the first");

        // A checkpoint taken before anything was applied carries none, and CLEARS an older one:
        // last-writer-wins for the pair, never a mix of a new position and a stale highwater.
        let fold = fold_of(vec![
            CatalogEvent::Offset { pos: pos(0, "20"), highwater: Some((0x20, 7)) },
            CatalogEvent::Offset { pos: pos(1, "0"), highwater: None },
        ]);
        assert_eq!(fold.start_pos, pos(1, "0"));
        assert_eq!(fold.start_highwater, None);

        // Nothing at all: no highwater, so a first boot de-duplicates from scratch.
        assert_eq!(CatalogFold::default().start_highwater, None);
    }

    /// The wire form: the highwater is omitted when absent (so a checkpoint that has applied
    /// nothing is the same bytes it always was) and round-trips when present.
    #[test]
    fn the_checkpoint_wire_form_carries_the_highwater_only_when_there_is_one() {
        let bare = serde_json::to_value(CatalogEvent::Offset { pos: pos(2, "9"), highwater: None }).unwrap();
        assert_eq!(bare.get("highwater"), None, "{bare}");
        let with = serde_json::to_value(CatalogEvent::Offset { pos: pos(2, "9"), highwater: Some((5, 6)) }).unwrap();
        assert_eq!(with["highwater"], serde_json::json!([5, 6]));
        match serde_json::from_value::<CatalogEvent>(with).unwrap() {
            CatalogEvent::Offset { pos: p, highwater } => {
                assert_eq!(p, pos(2, "9"));
                assert_eq!(highwater, Some((5, 6)));
            }
            other => panic!("expected Offset, got {other:?}"),
        }
    }

    /// The change log's segmentation folds out of the same log (ADR-0006): the last rotation is the
    /// current segment, and EVERY rotation is kept — segment `n` was closed when `n+1` began, which
    /// is the clock the retain window that governs deletion runs on.
    #[test]
    fn rotations_fold_to_the_current_segment_and_every_segments_start() {
        let fold = fold_of(vec![
            CatalogEvent::ChangesRotated { segment: 0, at: 100 },
            CatalogEvent::Offset { pos: pos(0, "9"), highwater: None },
            CatalogEvent::ChangesRotated { segment: 1, at: 200 },
            CatalogEvent::ChangesRotated { segment: 2, at: 300 },
        ]);
        assert_eq!(fold.current_segment, 2);
        assert_eq!(fold.segment_starts.get(&0), Some(&100));
        assert_eq!(fold.segment_starts.get(&1), Some(&200));
        assert_eq!(fold.segment_starts.get(&2), Some(&300));
        // A catalog that has only ever rotated has nothing to restore.
        assert_eq!(fold.start_pos, pos(0, "9"));

        // Nothing rotated = segment 0, which is also what a first boot creates.
        assert_eq!(CatalogFold::default().current_segment, 0);
    }

    /// A deleted segment leaves the folded set, so a restart neither re-plans retiring streams that
    /// are long gone nor reports a `changes_segments_retained` gauge counting them. It never moves
    /// the current segment — the current one is never deleted.
    #[test]
    fn deleted_segments_leave_the_fold() {
        let fold = fold_of(vec![
            CatalogEvent::ChangesRotated { segment: 0, at: 100 },
            CatalogEvent::ChangesRotated { segment: 1, at: 200 },
            CatalogEvent::ChangesRotated { segment: 2, at: 300 },
            CatalogEvent::ChangesSegmentDeleted { segment: 0 },
        ]);
        assert_eq!(fold.current_segment, 2);
        assert_eq!(fold.segment_starts.keys().copied().collect::<Vec<_>>(), vec![1, 2]);

        let json = serde_json::to_value(CatalogEvent::ChangesSegmentDeleted { segment: 7 }).unwrap();
        assert_eq!(json["t"], "changesSegmentDeleted");
        assert_eq!(json["segment"], 7);
    }

    /// The sequencer's checkpoint and a dormant shape's resume state carry the SEGMENT, so a
    /// restart after a rotation resumes in the right stream rather than at a byte position in a
    /// stream that no longer holds those bytes.
    #[test]
    fn checkpoints_and_dormant_resumes_carry_their_segment() {
        let created = serde_json::from_value::<CatalogEvent>(created_event("public.users")).unwrap();
        let gate = crate::pg::SnapshotGate::passthrough();
        let fold = fold_of(vec![
            created,
            CatalogEvent::Dormant { id: "s1".to_string(), resume: pos(3, "77"), gate },
            CatalogEvent::Offset { pos: pos(4, "12"), highwater: None },
        ]);
        assert_eq!(fold.start_pos, pos(4, "12"));
        assert_eq!(fold.recs.get("s1").and_then(|r| r.3.as_ref()).map(|(p, _)| p.clone()), Some(pos(3, "77")));

        // Wire form: an object, never a bare string.
        let json = serde_json::to_value(CatalogEvent::Offset { pos: pos(4, "12"), highwater: None }).unwrap();
        assert_eq!(json["t"], "offset");
        assert_eq!(json["pos"]["segment"], 4);
        assert_eq!(json["pos"]["offset"], "12");
    }

    /// A catalog written before ADR-0006 carries bare change-log offsets, which cannot be repaired:
    /// the byte position belongs to a stream that no longer exists under that name. It is named and
    /// refused at boot, never coerced to "segment 0" (which would resume at an unrelated point) and
    /// never silently skipped by the strict deserializer (which for an `Offset` would restart the
    /// sequencer from the beginning of the log).
    #[test]
    fn a_pre_segmentation_catalog_is_refused_by_name() {
        let bare_offset = serde_json::json!({ "t": "offset", "offset": "0000000000000000_0000000000000042" });
        let detail = predates_segmentation(&bare_offset).expect("a bare offset is recognised");
        assert!(detail.contains("bare change-log offset"), "{detail}");
        assert!(
            CatalogPredatesSegmentation { detail }.to_string().contains("predates ADR-0006"),
            "the operator is told which cutover the catalog is on the wrong side of"
        );

        let bare_dormant = serde_json::json!({
            "t": "dormant",
            "id": "s7",
            "resume_offset": "0000000000000000_0000000000000042",
            "gate": {},
        });
        let detail = predates_segmentation(&bare_dormant).expect("a bare dormant resume is recognised");
        assert!(detail.contains("s7"), "the refusal names the shape: {detail}");

        // The current spellings are not mistaken for the old ones.
        let now = serde_json::to_value(CatalogEvent::Offset { pos: pos(2, "9"), highwater: None }).unwrap();
        assert!(predates_segmentation(&now).is_none());
        let gate = crate::pg::SnapshotGate::passthrough();
        let now = serde_json::to_value(CatalogEvent::Dormant {
            id: "s7".to_string(),
            resume: pos(2, "9"),
            gate,
        })
        .unwrap();
        assert!(predates_segmentation(&now).is_none());
        // ...and neither is an unrelated event.
        assert!(predates_segmentation(&created_event("public.users")).is_none());
    }

    /// The LAST binding is the epoch in force: a reset appends its new one after the `Dropped`
    /// records of the epoch it ended, so the fold must not stop at the first.
    #[test]
    fn the_last_slot_bound_wins() {
        let created = serde_json::from_value::<CatalogEvent>(created_event("public.users")).unwrap();
        let fold = fold_of(vec![
            bound("s", "7300000000000000001", "2026-08-20T09:00:00.000Z"),
            created,
            CatalogEvent::Dropped { id: "s1".to_string() },
            bound("s", "7300000000000000001", "2026-08-21T11:30:00.000Z"),
        ]);
        let b = fold.binding.expect("the epoch is folded out of the log");
        assert_eq!(b.bound_at, "2026-08-21T11:30:00.000Z", "the newest binding is the epoch in force");
        assert!(fold.recs.is_empty(), "the epoch it ended took its shapes with it");
    }

    /// The binding survives everything else in the log — dropping every shape does not drop the
    /// epoch, and the epoch does not disturb the shapes.
    #[test]
    fn shapes_and_the_binding_fold_independently() {
        let created = serde_json::from_value::<CatalogEvent>(created_event("public.users")).unwrap();
        let fold = fold_of(vec![
            bound("s", "7300000000000000001", "2026-08-21T11:30:00.000Z"),
            created,
            CatalogEvent::Joined { id: "s1".to_string() },
        ]);
        assert_eq!(fold.binding.map(|b| b.slot), Some("s".to_string()));
        assert_eq!(fold.recs.get("s1").map(|r| r.2), Some(2), "create + join = refcount 2");
    }

    /// The event is stored (and restored) as its own `t` case, alongside the shape lifecycle — a
    /// catalog is one log, and its epoch is part of it.
    #[test]
    fn slot_bound_round_trips_on_the_wire() {
        let json = serde_json::to_value(bound("electric_circuits", "73", "2026-08-21T11:30:00.000Z")).unwrap();
        assert_eq!(json["t"], "slotBound");
        assert_eq!(json["slot"], "electric_circuits");
        assert_eq!(json["system_identifier"], "73");
        assert_eq!(json["timeline_id"], 1);
        assert_eq!(json["bound_at"], "2026-08-21T11:30:00.000Z");
        match serde_json::from_value::<CatalogEvent>(json).unwrap() {
            CatalogEvent::SlotBound(b) => assert_eq!(b.system_identifier, "73"),
            other => panic!("expected SlotBound, got {other:?}"),
        }
    }
}
