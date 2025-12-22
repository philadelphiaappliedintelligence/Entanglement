//! WebSocket sync notifications
//!
//! Provides real-time push notifications to connected clients when files change.
//! Clients can subscribe to receive updates and trigger FileProvider reimport.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

use crate::api::AppState;
use crate::auth;

/// Message broadcast to connected clients when files change
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncNotification {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub path: String,
    pub action: String,
}

impl SyncNotification {
    pub fn file_changed(path: &str, action: &str) -> Self {
        Self {
            msg_type: "file_changed".to_string(),
            path: path.to_string(),
            action: action.to_string(),
        }
    }
}

/// Hub for broadcasting sync notifications to all connected clients
#[derive(Clone)]
pub struct SyncHub {
    /// Broadcast channel sender
    tx: broadcast::Sender<SyncNotification>,
}

impl SyncHub {
    /// Create a new SyncHub with specified channel capacity
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Broadcast a notification to all connected clients
    pub fn broadcast(&self, notification: SyncNotification) {
        // It's OK if there are no receivers - just means no clients connected
        let _ = self.tx.send(notification);
    }

    /// Broadcast a file change event
    pub fn notify_file_changed(&self, path: &str, action: &str) {
        let notification = SyncNotification::file_changed(path, action);
        debug!("Broadcasting sync notification: {:?}", notification);
        self.broadcast(notification);
    }

    /// Subscribe to receive notifications
    pub fn subscribe(&self) -> broadcast::Receiver<SyncNotification> {
        self.tx.subscribe()
    }
}

impl Default for SyncHub {
    fn default() -> Self {
        Self::new(256) // Buffer up to 256 messages
    }
}

/// Query parameters for WebSocket connection
#[derive(Deserialize)]
pub struct WsQuery {
    /// Authentication token
    token: String,
}

/// WebSocket upgrade handler
///
/// GET /ws/sync?token=<jwt>
///
/// Upgrades the connection to WebSocket and subscribes to sync notifications.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(query): Query<WsQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    // Validate token
    // Validate token
    match auth::verify_token(&state.config.jwt_secret, &query.token) {
        Ok(user_id) => {
            info!("WebSocket connection authenticated for user: {}", user_id);
            ws.on_upgrade(move |socket| handle_socket(socket, state))
        }
        Err(e) => {
            warn!("WebSocket auth failed: {}", e);
            // Return 401 by not upgrading
            // Axum will handle this gracefully
            ws.on_upgrade(|socket| async move {
                // Close immediately with error
                let _ = socket;
            })
        }
    }
}

/// Handle an individual WebSocket connection
async fn handle_socket(mut socket: WebSocket, state: AppState) {
    info!("WebSocket client connected");

    // Subscribe to sync notifications
    let mut rx = state.sync_hub.subscribe();

    // Send/receive loop
    loop {
        tokio::select! {
            // Forward broadcast notifications to client
            result = rx.recv() => {
                match result {
                    Ok(notification) => {
                        let json = serde_json::to_string(&notification).unwrap_or_default();
                        if socket.send(Message::Text(json)).await.is_err() {
                            debug!("WebSocket send failed, client disconnected");
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("WebSocket client lagged, missed {} messages", n);
                        // Continue anyway - client will get next message
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        debug!("Broadcast channel closed");
                        break;
                    }
                }
            }

            // Handle incoming messages from client (ping/pong, close)
            result = socket.recv() => {
                match result {
                    Some(Ok(Message::Text(text))) => {
                        debug!("Received message from client: {}", text);
                        // Could handle client commands here (e.g., subscribe to specific paths)
                    }
                    Some(Ok(Message::Ping(data))) => {
                        if socket.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        debug!("WebSocket client disconnected");
                        break;
                    }
                    Some(Err(e)) => {
                        error!("WebSocket error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    info!("WebSocket client disconnected");
}
