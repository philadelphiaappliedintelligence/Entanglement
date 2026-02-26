//! Admin routes
//!
//! Server info, statistics, and health check endpoints.

use crate::api::AppState;
use crate::db::users;
use axum::{
    extract::State,
    http::StatusCode,
    routing::get,
    Json, Router,
};
use serde::Serialize;

use super::error::{self, AppError};

// ============================================================================
// ROUTES
// ============================================================================

pub fn admin_routes() -> Router<AppState> {
    Router::new()
        .route("/admin/stats", get(get_stats))
        .route("/server/info", get(get_server_info))
        // Health check endpoints for container orchestration
        .route("/health", get(health_check))
        .route("/health/ready", get(readiness_check))
        .route("/health/live", get(liveness_check))
}

// ============================================================================
// TYPES
// ============================================================================

#[derive(Serialize)]
struct ServerInfo {
    name: String,
    version: String,
}

#[derive(Serialize)]
struct StatsResponse {
    total_users: i64,
    total_files: i64,
    total_versions: i64,
    total_blob_bytes: i64,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
    db: &'static str,
}

// ============================================================================
// HANDLERS
// ============================================================================

async fn get_server_info(State(state): State<AppState>) -> Json<ServerInfo> {
    Json(ServerInfo {
        name: state.config.server_name.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

async fn get_stats(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<StatsResponse>, AppError> {
    // SECURITY: Require admin authentication
    let user_id = error::extract_user_id(&state, &headers)?;
    let user = users::get_user_by_id(&state.db, user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".into()))?;
    if !user.is_admin {
        return Err(AppError::Unauthorized("Admin access required".into()));
    }

    let stats = crate::db::get_stats(&state.db).await?;
    Ok(Json(StatsResponse {
        total_users: stats.total_users,
        total_files: stats.total_files,
        total_versions: stats.total_versions,
        total_blob_bytes: stats.total_blob_bytes,
    }))
}

/// Combined health check - verifies database connectivity
async fn health_check(State(state): State<AppState>) -> (StatusCode, Json<HealthResponse>) {
    // Check database connectivity with a simple query
    let db_status = match sqlx::query("SELECT 1").execute(&state.db).await {
        Ok(_) => "connected",
        Err(e) => {
            tracing::warn!("Health check: database error: {}", e);
            "disconnected"
        }
    };
    
    let status = if db_status == "connected" { "healthy" } else { "unhealthy" };
    let code = if db_status == "connected" { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    
    (code, Json(HealthResponse {
        status,
        version: env!("CARGO_PKG_VERSION"),
        db: db_status,
    }))
}

/// Readiness probe - returns 200 if server can accept traffic
async fn readiness_check(State(state): State<AppState>) -> StatusCode {
    match sqlx::query("SELECT 1").execute(&state.db).await {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

/// Liveness probe - returns 200 if process is running
async fn liveness_check() -> StatusCode {
    StatusCode::OK
}
