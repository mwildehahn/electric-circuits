//! Minimal durable-streams HTTP client: PUT-create, POST-append (JSON array), and
//! offset-resumable reads (catch-up + long-poll live). Offsets are opaque tokens; we just
//! persist and replay `Stream-Next-Offset`.

use std::collections::HashMap;

use anyhow::{Context, Result, bail};

use crate::heap_size::HeapSize;
use serde::{Deserialize, Serialize};

/// A State-Protocol change event, the JSON item on every table/shape stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    #[serde(rename = "type")]
    pub type_: String,
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    /// The full prior row, carried by replication on UPDATE/DELETE (`REPLICA IDENTITY FULL`). Lets
    /// the engine compute the input delta without an in-memory `table_state`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old: Option<serde_json::Value>,
    pub headers: EnvelopeHeaders,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvelopeHeaders {
    pub operation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub txid: Option<String>,
    // The server stamps an `offset` onto each item; accept it on read, never send it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<String>,
    /// Postgres commit LSN of the change (set by the replication ingestor). Used to skip changes a
    /// shape/family already reflects from its backfill snapshot (`lsn <= seed_lsn`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lsn: Option<String>,
    /// Position of this change within its transaction (set by the ingestor). `(lsn, seq)` uniquely
    /// identifies a change, letting the tailer skip duplicates when the ingestor re-appends a batch
    /// after a partial failure or a crash between append and slot-advance (at-least-once delivery).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    /// **Transaction-end marker**: `Some(true)` on the LAST envelope of a transaction, and only
    /// there (ADR-0003).
    ///
    /// It is what keeps per-transaction atomic emission true on the wire now that a commit too
    /// large for one request body is appended in several chunks. Each append is exposed atomically
    /// by durable-streams, so without the marker the sequencer would see chunk 1 as a complete
    /// `(txid, lsn)` run, fan it out and flush it to shape streams, then do the same for chunks
    /// 2..N — a subscriber would observe a fraction of a transaction. With it, the sequencer HOLDS a
    /// trailing run whose last envelope is unmarked and processes the transaction only once the
    /// marker arrives.
    ///
    /// Every producer sets it: the ingestor on the last envelope of the last chunk (single-chunk
    /// commits included, so the rule is uniform), and library-mode writers on every envelope
    /// (one-envelope transactions). An envelope WITHOUT it that is not followed by one is an
    /// incomplete transaction, by definition — never a transaction that opted out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<bool>,
}

impl crate::heap_size::HeapSize for EnvelopeHeaders {
    fn heap_bytes(&self) -> usize {
        self.operation.heap_bytes()
            + self.txid.heap_bytes()
            + self.offset.heap_bytes()
            + self.lsn.heap_bytes()
    }
}

impl crate::heap_size::HeapSize for Envelope {
    fn heap_bytes(&self) -> usize {
        self.type_.heap_bytes() + self.key.heap_bytes() + self.value.heap_bytes() + self.old.heap_bytes()
            + self.headers.heap_bytes()
    }
}

/// What one envelope costs to hold in memory: its inline representation plus the heap it owns.
///
/// This is the quantity `ELECTRIC_CIRCUITS_TXN_MEMORY_BYTES` is measured in (ADR-0003), so the knob
/// counts what is actually held rather than what the same data would serialize to. It is a lower
/// bound in the same sense as every other [`crate::heap_size::HeapSize`] estimate (allocator
/// overhead and `serde_json::Map` bucket overhead are not modelled).
pub fn envelope_memory_bytes(env: &Envelope) -> u64 {
    (std::mem::size_of::<Envelope>() + env.heap_bytes()) as u64
}

pub struct ReadResult {
    pub envelopes: Vec<Envelope>,
    pub next_offset: Option<String>,
    pub up_to_date: bool,
    /// The server reported `stream-closed`: the stream is **terminal** — it will never grow again.
    /// For a shape stream that means the engine retired the shape (close-then-delete, see
    /// [`DsClient::retire_stream`]) and readers must stop, not re-poll: a closed stream answers a
    /// long-poll instantly, so looping on "empty page, same offset" would spin on the server. For a
    /// `changes/<n>` segment it means the log ROTATED (ADR-0006) and the reader follows the batch's
    /// rotation pointer onto the next segment.
    pub closed: bool,
}

/// What a `HEAD` found: the stream's tail offset and whether it is closed. `None` from
/// [`DsClient::head`] means the stream is not there (404) or soft-deleted (410).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamHead {
    pub next_offset: Option<String>,
    pub closed: bool,
}

/// A read hit a stream that is not there: deleted (404) or soft-deleted (410).
///
/// A typed error because callers make very different decisions about it than about a transient read
/// failure. On the change log (ADR-0006) it is never expected — a segment is deleted only once
/// nothing can resume inside it — so the sequencer treats it as an error to log loudly and back off
/// from, the boot treats it as fatal for the position it is about to resume, and a dormant shape's
/// replay treats it as "this shape can never be brought up to date", which evicts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamGone {
    pub path: String,
    pub status: u16,
}

impl std::fmt::Display for StreamGone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "stream '{}' is gone ({})", self.path, self.status)
    }
}

impl std::error::Error for StreamGone {}

/// Does this error (or anything it was contextualised from) mean "the stream is not there"?
pub fn is_stream_gone(e: &anyhow::Error) -> bool {
    e.chain().any(|c| c.downcast_ref::<StreamGone>().is_some())
}

/// The outcome of an append that treats retirement as an answer rather than an error (see
/// [`DsClient::append_checked`]).
pub enum Appended {
    /// Landed; `next_offset` is the stream's tail afterwards (`stream-next-offset`).
    Ok { next_offset: Option<String> },
    /// The stream is retired — deleted (404), soft-deleted (410) or closed (409 + `stream-closed`).
    Retired(u16),
}

/// Why an append failed: the stream is retired — deleted (404), soft-deleted (410) or closed
/// (409 + `stream-closed`), all terminal, discard — or a transient/other error (retry or surface).
/// `Gone` carries the status so the log/error names which of the three it was.
enum AppendError {
    Gone(u16),
    Other(anyhow::Error),
}

#[derive(Clone)]
pub struct DsClient {
    base: String,
    http: reqwest::Client,
    /// Bytes appended per stream path since this process started (serialized request bodies).
    /// The durable-streams server exposes no per-stream sizes, so this engine-side accounting is
    /// what the retention disk-budget layer works from. It undercounts streams that already
    /// existed before the process started (restart persistence is the catalog work, GH #8).
    appended: std::sync::Arc<std::sync::Mutex<HashMap<String, u64>>>,
}

impl DsClient {
    pub fn new(base: impl Into<String>) -> Self {
        DsClient {
            base: base.into(),
            http: reqwest::Client::new(),
            appended: std::sync::Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }

    /// Tracked bytes appended to `path` since process start (0 if never appended).
    pub fn appended_bytes(&self, path: &str) -> u64 {
        self.appended.lock().unwrap().get(path).copied().unwrap_or(0)
    }

    /// Snapshot of tracked appended bytes for every stream path with the given prefix
    /// (e.g. `"shape/"` for the retention disk budget).
    pub fn appended_bytes_with_prefix(&self, prefix: &str) -> HashMap<String, u64> {
        self.appended
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| p.starts_with(prefix))
            .map(|(p, b)| (p.clone(), *b))
            .collect()
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn stream_url(&self, path: &str) -> String {
        format!("{}/{}", self.base.trim_end_matches('/'), path.trim_start_matches('/'))
    }

    /// Idempotently create a JSON stream (PUT). Existing stream with same config -> 200.
    pub async fn ensure_stream(&self, path: &str) -> Result<()> {
        let res = self
            .http
            .put(self.stream_url(path))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .send()
            .await
            .with_context(|| format!("PUT {path}"))?;
        let status = res.status();
        // Drain the body so the connection returns to reqwest's pool. Skipping this leaks one socket
        // per call, which exhausts ephemeral ports when creating many shape streams.
        let body = res.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(())
        } else {
            bail!("PUT {path} -> {status}: {body}")
        }
    }

    /// Append envelopes as a JSON array (the server flattens one array level into N messages).
    /// Append raw JSON events (non-envelope streams, e.g. the shape catalog).
    pub async fn append_json(&self, path: &str, events: &[serde_json::Value]) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }
        let res = self
            .http
            .post(self.stream_url(path))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(events)
            .send()
            .await
            .with_context(|| format!("POST {path}"))?;
        let status = res.status();
        if !status.is_success() {
            let body = res.text().await.unwrap_or_default();
            bail!("POST {path} -> {status}: {body}");
        }
        Ok(())
    }

    /// Read raw JSON events (non-envelope streams). Returns `(events, next_offset, up_to_date)`.
    pub async fn read_json(
        &self,
        path: &str,
        offset: &str,
    ) -> Result<(Vec<serde_json::Value>, Option<String>, bool)> {
        let url = format!("{}?offset={}", self.stream_url(path), offset);
        let res = self.http.get(url).send().await.with_context(|| format!("GET {path}"))?;
        let status = res.status();
        let next_offset = header(&res, "stream-next-offset");
        let up_to_date = res.headers().get("stream-up-to-date").is_some();
        if status.as_u16() == 204 || status.as_u16() == 404 {
            return Ok((Vec::new(), next_offset, true));
        }
        if !status.is_success() {
            bail!("GET {path} -> {status}");
        }
        let body = res.text().await?;
        let events: Vec<serde_json::Value> = if body.trim().is_empty() {
            Vec::new()
        } else {
            serde_json::from_str(&body).with_context(|| format!("parsing stream body: {body}"))?
        };
        Ok((events, next_offset, up_to_date))
    }

    /// Append envelopes. A retired stream (404/410/closed) is an error here; the live path uses
    /// [`Self::append_reliable`], which discards instead, and the change log uses
    /// [`Self::append_checked`], which routes around it.
    pub async fn append(&self, path: &str, envelopes: &[Envelope]) -> Result<()> {
        match self.append_once(path, envelopes).await {
            Ok(_) => Ok(()),
            Err(AppendError::Gone(status)) => bail!("POST {path} -> {status} (stream retired)"),
            Err(AppendError::Other(e)) => Err(e),
        }
    }

    /// Append, reporting a retired stream as an outcome instead of an error, and handing back the
    /// stream's tail offset on success.
    ///
    /// The change log's writer needs both (ADR-0006): the tail offset IS the segment's size (the
    /// rotation decision), and a closed segment is a routing signal — walk forward to the successor
    /// — not a loss. Transient failures still surface as `Err` so the ingestor can tear its
    /// connection down unacknowledged rather than lose the commit.
    pub async fn append_checked(&self, path: &str, envelopes: &[Envelope]) -> Result<Appended> {
        match self.append_once(path, envelopes).await {
            Ok(next_offset) => Ok(Appended::Ok { next_offset }),
            Err(AppendError::Gone(status)) => Ok(Appended::Retired(status)),
            Err(AppendError::Other(e)) => Err(e),
        }
    }

    async fn append_once(
        &self,
        path: &str,
        envelopes: &[Envelope],
    ) -> std::result::Result<Option<String>, AppendError> {
        if envelopes.is_empty() {
            return Ok(None);
        }
        // Serialize once ourselves (instead of `.json(...)`) so the successful append's byte size
        // can be recorded for the retention disk-budget accounting.
        let payload = serde_json::to_vec(envelopes)
            .map_err(|e| AppendError::Other(anyhow::Error::new(e).context(format!("serializing POST {path}"))))?;
        let payload_len = payload.len() as u64;
        let res = self
            .http
            .post(self.stream_url(path))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(payload)
            .send()
            .await
            .map_err(|e| AppendError::Other(anyhow::Error::new(e).context(format!("POST {path}"))))?;
        let status = res.status();
        // A retired stream answers 404 (deleted), 410 (soft-deleted) or 409 + `stream-closed: true`
        // (closed, which retirement does before deleting). Read the header before the body consumes
        // the response; the body text ("stream is closed") is not the contract.
        let closed = header(&res, "stream-closed").is_some_and(|v| v.eq_ignore_ascii_case("true"));
        let next_offset = header(&res, "stream-next-offset");
        // Drain the body so the connection can be pooled and reused (avoids a socket leak per append).
        let body = res.text().await.unwrap_or_default();
        if status.is_success() {
            *self.appended.lock().unwrap().entry(path.to_string()).or_insert(0) += payload_len;
            Ok(next_offset)
        } else if status.as_u16() == 404 || status.as_u16() == 410 || (status.as_u16() == 409 && closed) {
            Err(AppendError::Gone(status.as_u16()))
        } else {
            Err(AppendError::Other(anyhow::anyhow!("POST {path} -> {status}: {body}")))
        }
    }

    /// Append with **no silent loss**: retry transient failures with capped backoff until the append
    /// lands. A dropped shape-stream append is a permanent divergence for every subscriber of that
    /// shape, so the only sound behaviors are (a) retry until success — the storage server being down
    /// simply backpressures the tailer, matching the ingestor's read-then-commit stance — or (b) stop
    /// because the stream was retired (the shape was dropped/evicted mid-flush), which is a clean
    /// no-op. Envelopes are absolute per-pk (`upsert`/`delete` by key), so an at-least-once retry
    /// that double-appends after an ambiguous network failure is idempotent for readers.
    /// Returns `false` iff the stream is retired (404, 410, or closed).
    ///
    /// Treating a **closed** stream as terminal is sound only for shape streams: their envelopes are
    /// absolute per-pk and the stream is about to be deleted, so the discarded batch has no reader
    /// left to diverge. The change log must keep using [`Self::append`] (which propagates): a closed
    /// `changes/*` segment is a routing signal, and silently dropping ingest there would lose data.
    pub async fn append_reliable(&self, path: &str, envelopes: &[Envelope]) -> bool {
        let mut attempt = 0u32;
        loop {
            match self.append_once(path, envelopes).await {
                Ok(_) => return true,
                Err(AppendError::Gone(status)) => {
                    tracing::debug!("append to {path}: stream retired ({status}); discarding {} envelopes", envelopes.len());
                    return false;
                }
                Err(AppendError::Other(e)) => {
                    attempt += 1;
                    let backoff = std::time::Duration::from_millis(100u64.saturating_mul(1 << attempt.min(5)).min(2000));
                    if attempt.is_multiple_of(10) {
                        tracing::error!("append to {path} still failing after {attempt} attempts: {e:#}");
                    } else {
                        tracing::warn!("append to {path} failed (attempt {attempt}), retrying in {backoff:?}: {e:#}");
                    }
                    tokio::time::sleep(backoff).await;
                }
            }
        }
    }

    /// Close a stream: `POST` with `Stream-Closed: true` and an empty body. Closing is terminal —
    /// appends are refused with `409` + `stream-closed` afterwards — and it releases every waiting
    /// long-poll reader immediately with `stream-closed: true` instead of leaving it blocked until
    /// the read times out. Idempotent (`204` again for an already-closed stream); an absent (`404`)
    /// or soft-deleted (`410`) stream is a success, there is nothing left to close.
    pub async fn close_stream(&self, path: &str) -> Result<()> {
        let res = self
            .http
            .post(self.stream_url(path))
            .header("stream-closed", "true")
            .send()
            .await
            .with_context(|| format!("POST {path} (close)"))?;
        let status = res.status();
        // Drain the body so the connection returns to reqwest's pool (see `ensure_stream`).
        let body = res.text().await.unwrap_or_default();
        if status.is_success() || status.as_u16() == 404 || status.as_u16() == 410 {
            Ok(())
        } else {
            bail!("POST {path} (close) -> {status}: {body}")
        }
    }

    /// Retire a stream: close it, THEN delete it (see `docs/adr/0007-retirement-closes-before-delete.md`).
    /// Every engine-initiated removal of a shape stream goes through here — eviction, purge,
    /// drop-at-restore, the degraded subquery reap, and the future schema-drift / epoch-reset paths —
    /// so a tailing client is released at once with `stream-closed` and "closed" unambiguously means
    /// "the engine retired this shape; re-subscribe". Closing is terminal, so the paths that are NOT
    /// retirement must not use it: deactivation parks a dormant shape whose stream stays appendable
    /// for reactivation, and creation rollback removes a stream no subscriber ever saw.
    /// A failed close is logged and does not block the delete: removing the stream is the must-have,
    /// the close is the courtesy signal. Returns the delete's result.
    pub async fn retire_stream(&self, path: &str) -> Result<()> {
        if let Err(e) = self.close_stream(path).await {
            tracing::warn!("retiring stream {path}: close failed ({e:#}); deleting anyway");
        }
        self.delete_stream(path).await
    }

    /// Delete a stream (DELETE). An already-gone stream — absent (404) or soft-deleted (410) — is a
    /// success: deletion is idempotent, and a retry loop (the degraded reap) must not spin forever
    /// on a stream storage has already retired.
    pub async fn delete_stream(&self, path: &str) -> Result<()> {
        let res = self.http.delete(self.stream_url(path)).send().await.with_context(|| format!("DELETE {path}"))?;
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        if status.is_success() || status.as_u16() == 404 || status.as_u16() == 410 {
            self.appended.lock().unwrap().remove(path);
            Ok(())
        } else {
            bail!("DELETE {path} -> {status}: {body}")
        }
    }

    /// `HEAD` a stream: its tail offset and whether it is closed, without reading a byte of it (and
    /// without resetting its TTL). `Ok(None)` = not there (404) or soft-deleted (410).
    ///
    /// The change log's boot walk uses this to step over segments a crashed predecessor closed
    /// (ADR-0006): a closed segment can be a gigabyte, and durable-streams offers no bounded tail
    /// read, so the successor is derived and *verified* rather than read back.
    pub async fn head(&self, path: &str) -> Result<Option<StreamHead>> {
        let res = self
            .http
            .head(self.stream_url(path))
            .send()
            .await
            .with_context(|| format!("HEAD {path}"))?;
        let status = res.status();
        if status.as_u16() == 404 || status.as_u16() == 410 {
            return Ok(None);
        }
        if !status.is_success() {
            bail!("HEAD {path} -> {status}");
        }
        Ok(Some(StreamHead {
            next_offset: header(&res, "stream-next-offset"),
            closed: header(&res, "stream-closed").is_some_and(|v| v.eq_ignore_ascii_case("true")),
        }))
    }

    /// Read from `offset` (use "-1" for the beginning). `live` enables long-poll tailing.
    pub async fn read(&self, path: &str, offset: &str, live: bool) -> Result<ReadResult> {
        let mut url = format!("{}?offset={}", self.stream_url(path), offset);
        if live {
            url.push_str("&live=long-poll");
        }
        let res = self.http.get(url).send().await.with_context(|| format!("GET {path}"))?;
        let status = res.status();
        let next_offset = header(&res, "stream-next-offset");
        let up_to_date = res.headers().get("stream-up-to-date").is_some();
        // The single place `stream-closed` is parsed off a read; every caller works from
        // `ReadResult::closed`.
        let closed = header(&res, "stream-closed").is_some_and(|v| v.eq_ignore_ascii_case("true"));

        // 204 = long-poll timeout / no new data / a close that woke this long-poll.
        if status.as_u16() == 204 {
            return Ok(ReadResult { envelopes: Vec::new(), next_offset, up_to_date, closed });
        }
        // A stream that is not there is a TYPED error (see `StreamGone`), never a generic read
        // failure: on the change log every caller has its own, very different answer to it.
        if status.as_u16() == 404 || status.as_u16() == 410 {
            return Err(anyhow::Error::new(StreamGone { path: path.to_string(), status: status.as_u16() }));
        }
        if !status.is_success() {
            bail!("GET {path} -> {status}");
        }
        let body = res.text().await?;
        let envelopes: Vec<Envelope> = if body.trim().is_empty() {
            Vec::new()
        } else {
            serde_json::from_str(&body).with_context(|| format!("parsing stream body: {body}"))?
        };
        Ok(ReadResult { envelopes, next_offset, up_to_date, closed })
    }
}

fn header(res: &reqwest::Response, name: &str) -> Option<String> {
    res.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_string)
}
