//! WebSocket fan-out for live executor events.

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast::error::RecvError;

use super::state::AppState;

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = state.events.subscribe();

    // Forwarder: pulls executor events out of the broadcast channel and pushes
    // them to this client as JSON text frames. If the client falls behind by
    // more than the channel capacity, lagged events are skipped silently —
    // the next event the client receives still has the latest job state, and
    // the UI re-fetches /api/job/status on connect anyway.
    let send_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    let json = match serde_json::to_string(&ev) {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    if sender.send(Message::Text(json)).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    });

    // Drain inbound frames so close/ping handling works. The admin panel never
    // sends application messages — this loop just keeps the socket alive.
    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(_) => break,
        }
    }

    send_task.abort();
}
