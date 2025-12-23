//! WebSocket sync notifications
//!
//! Provides real-time push notifications to connected clients when files change.
//! Clients can subscribe to receive updates and trigger FileProvider reimport.
//! Rate limiter is reserved for future per-user broadcast throttling.

#![allow(dead_code)]

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

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

/// Rate limiter for file change broadcasts per user
/// Prevents malicious users from flooding the broadcast channel
#[derive(Clone)]
pub struct BroadcastRateLimiter {
    /// Per-user token bucket: user_id -> (tokens, last_refill_time)
    buckets: Arc<RwLock<HashMap<Uuid, (u32, Instant)>>>,
    /// Maximum tokens (burst capacity)
    max_tokens: u32,
    /// Tokens refilled per second
    refill_rate: u32,
}

impl BroadcastRateLimiter {
    pub fn new(max_tokens: u32, refill_rate: u32) -> Self {
        Self {
            buckets: Arc::new(RwLock::new(HashMap::new())),
            max_tokens,
            refill_rate,
        }
    }

    /// Try to consume a token for the given user
    /// Returns true if allowed, false if rate limited
    pub async fn try_acquire(&self, user_id: Uuid) -> bool {
        let mut buckets = self.buckets.write().await;
        let now = Instant::now();

        let (tokens, last_refill) = buckets
            .entry(user_id)
            .or_insert((self.max_tokens, now));

        // Refill tokens based on elapsed time
        let elapsed = now.duration_since(*last_refill);
        let refill_amount = (elapsed.as_secs_f32() * self.refill_rate as f32) as u32;
        if refill_amount > 0 {
            *tokens = (*tokens + refill_amount).min(self.max_tokens);
            *last_refill = now;
        }

        // Try to consume a token
        if *tokens > 0 {
            *tokens -= 1;
            true
        } else {
            warn!("Rate limiting user {} for broadcast spam", user_id);
            false
        }
    }
}

impl Default for BroadcastRateLimiter {
    fn default() -> Self {
        // Allow burst of 50 file ops, refill 10 per second
        Self::new(50, 10)
    }
}

/// Hub for broadcasting sync notifications to all connected clients
#[derive(Clone)]
pub struct SyncHub {
    /// Broadcast channel sender
    tx: broadcast::Sender<SyncNotification>,
    /// Rate limiter for broadcasts
    rate_limiter: BroadcastRateLimiter,
}

impl SyncHub {
    /// Create a new SyncHub with specified channel capacity
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self {
            tx,
            rate_limiter: BroadcastRateLimiter::default(),
        }
    }

    /// Broadcast a notification to all connected clients
    pub fn broadcast(&self, notification: SyncNotification) {
        // It's OK if there are no receivers - just means no clients connected
        let _ = self.tx.send(notification);
    }

    /// Broadcast a file change event with rate limiting
    /// Returns false if rate limited
    pub async fn notify_file_changed_rate_limited(&self, path: &str, action: &str, user_id: Uuid) -> bool {
        if !self.rate_limiter.try_acquire(user_id).await {
            warn!("Dropping broadcast for user {} due to rate limiting", user_id);
            return false;
        }
        let notification = SyncNotification::file_changed(path, action);
        debug!("Broadcasting sync notification: {:?}", notification);
        self.broadcast(notification);
        true
    }

    /// Broadcast a file change event (no rate limiting - for internal use)
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
/// Returns 401 Unauthorized if authentication fails (does NOT upgrade connection).
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(query): Query<WsQuery>,
    State(state): State<AppState>,
) -> Response {
    // Validate token BEFORE upgrading connection
    // This prevents resource exhaustion from failed auth attempts
    match auth::verify_token(&state.config.jwt_secret, &query.token) {
        Ok(user_id) => {
            info!("WebSocket connection authenticated for user: {}", user_id);
            ws.on_upgrade(move |socket| handle_socket(socket, state)).into_response()
        }
        Err(e) => {
            warn!("WebSocket auth failed: {}", e);
            // Return 401 Unauthorized WITHOUT upgrading the connection
            // This prevents resource allocation for unauthenticated requests
            (StatusCode::UNAUTHORIZED, "Invalid or expired token").into_response()
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
