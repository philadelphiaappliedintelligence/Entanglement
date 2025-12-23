//! Admin routes
//!
//! Server info and statistics endpoints.

use crate::api::AppState;
use axum::{
    extract::State,
    routing::get,
    Json, Router,
};
use serde::Serialize;

use super::error::AppError;

// ============================================================================
// ROUTES
// ============================================================================

pub fn admin_routes() -> Router<AppState> {
    Router::new()
        .route("/admin/stats", get(get_stats))
        .route("/server/info", get(get_server_info))
}

// ============================================================================
// TYPES
// ============================================================================

#[derive(Serialize)]
struct ServerInfo {
    name: String,
    version: String,
    grpc_port: u16,
}

#[derive(Serialize)]
struct StatsResponse {
    total_users: i64,
    total_files: i64,
    total_versions: i64,
    total_blob_bytes: i64,
}

// ============================================================================
// HANDLERS
// ============================================================================

async fn get_server_info(State(state): State<AppState>) -> Json<ServerInfo> {
    Json(ServerInfo {
        name: state.config.server_name.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        grpc_port: state.config.grpc_port,
    })
}

async fn get_stats(State(state): State<AppState>) -> Result<Json<StatsResponse>, AppError> {
    let stats = crate::db::get_stats(&state.db).await?;
    Ok(Json(StatsResponse {
        total_users: stats.total_users,
        total_files: stats.total_files,
        total_versions: stats.total_versions,
        total_blob_bytes: stats.total_blob_bytes,
    }))
}
