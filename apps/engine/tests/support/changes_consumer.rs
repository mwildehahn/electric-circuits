//! Reference implementation of the external change-log reader contract.
//!
//! It is deliberately test-only: consumers own their checkpointing and delivery. The engine
//! promises an at-least-once page stream, so a conforming reader drops control envelopes by type,
//! holds a trailing transaction until its `last` marker, de-duplicates complete changes by
//! `(lsn, seq)`, and crosses a rotation only after the closed segment was fully drained.

use axum::response::Response;
use electric_circuits_engine::changelog::{is_control, rotation_target_in};
use electric_circuits_engine::ds::Envelope;

pub struct ChangePage {
    pub envelopes: Vec<Envelope>,
    pub closed: bool,
}

impl ChangePage {
    pub async fn from_response(response: Response) -> Result<Self, String> {
        let closed = response.headers().get("stream-closed").is_some_and(|value| value == "true");
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024).await.map_err(|error| error.to_string())?;
        let envelopes = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
        Ok(Self { envelopes, closed })
    }
}

#[derive(Default)]
pub struct ReferenceConsumer {
    held: Vec<Envelope>,
    highwater: Option<(String, u64)>,
    transactions: Vec<Vec<Envelope>>,
}

impl ReferenceConsumer {
    /// Consume one positioned page and, only when the page is closed *and* its data was drained,
    /// return its rotation target. A control pointer has no transaction identity and is never
    /// delivered as application data.
    pub fn consume(&mut self, page: ChangePage) -> Option<u32> {
        let next = rotation_target_in(&page.envelopes);
        for envelope in page.envelopes.into_iter().filter(|envelope| !is_control(envelope)) {
            let Some(lsn) = envelope.headers.lsn.clone() else { continue };
            let Some(seq) = envelope.headers.seq else { continue };
            if self.highwater.as_ref().is_some_and(|highwater| (lsn.clone(), seq) <= *highwater) {
                continue;
            }
            // A replay can overlap the held, unmarked prefix before the crash/retry. It has not
            // advanced the durable highwater yet, so suppress the exact prefix locally too.
            if self
                .held
                .iter()
                .any(|held| held.headers.lsn.as_deref() == Some(lsn.as_str()) && held.headers.seq == Some(seq))
            {
                continue;
            }
            let last = envelope.headers.last == Some(true);
            self.held.push(envelope);
            if last {
                self.highwater = Some((lsn, seq));
                self.transactions.push(std::mem::take(&mut self.held));
            }
        }
        if page.closed && self.held.is_empty() { next } else { None }
    }

    pub fn transactions(&self) -> &[Vec<Envelope>] {
        &self.transactions
    }
}
