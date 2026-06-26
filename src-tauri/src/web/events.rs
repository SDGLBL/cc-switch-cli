use std::convert::Infallible;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::Stream;
use tokio::sync::broadcast::error::RecvError;

use super::state::WebState;

/// `GET /api/events` — Server-Sent Events stream bridging Tauri `listen()`.
///
/// Each broadcast message is a pre-serialized JSON string of the form
/// `{ "event": "...", "payload": ... }`. The browser shim opens one shared
/// `EventSource` and demultiplexes by the `event` field. `emit` is never used
/// by the frontend, so this channel is server -> client only.
pub async fn sse_handler(
    State(state): State<WebState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut rx = state.events.subscribe();
    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(msg) => yield Ok(Event::default().data(msg)),
                // Slow client fell behind; skip dropped messages and continue.
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::default())
}
