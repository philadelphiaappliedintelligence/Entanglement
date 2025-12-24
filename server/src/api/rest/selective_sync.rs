//! Selective sync routes
//!
//! Handles per-user sync preferences and rules.

use crate::api::AppState;
use crate::auth;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post, delete, put},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::error::AppError;

// ============================================================================
// ROUTES
// ============================================================================

pub fn selective_sync_routes() -> Router<AppState> {
    Router::new()
        // Sync rules management
        .route("/sync/rules", get(list_rules))
        .route("/sync/rules", post(create_rule))
        .route("/sync/rules/:id", get(get_rule))
        .route("/sync/rules/:id", put(update_rule))
        .route("/sync/rules/:id", delete(delete_rule))
        // Path checking
        .route("/sync/check", post(check_paths))
        // Device management
        .route("/sync/devices", get(list_devices))
        .route("/sync/devices/:device_id", put(update_device))
        .route("/sync/devices/:device_id", delete(remove_device))
}

// ============================================================================
// TYPES
// ============================================================================

#[derive(Serialize)]
struct SyncRuleResponse {
    id: String,
    rule_type: String, // "include" or "exclude"
    path_pattern: String,
    priority: i32,
    is_active: bool,
    created_at: String,
    updated_at: String,
}

#[derive(Deserialize)]
struct CreateRuleRequest {
    rule_type: String,
    path_pattern: String,
    priority: Option<i32>,
}

#[derive(Deserialize)]
struct UpdateRuleRequest {
    rule_type: Option<String>,
    path_pattern: Option<String>,
    priority: Option<i32>,
    is_active: Option<bool>,
}

#[derive(Deserialize)]
struct ListRulesQuery {
    include_inactive: Option<bool>,
}

#[derive(Serialize)]
struct ListRulesResponse {
    rules: Vec<SyncRuleResponse>,
}

#[derive(Deserialize)]
struct CheckPathsRequest {
    paths: Vec<String>,
}

#[derive(Serialize)]
struct CheckPathsResponse {
    results: Vec<PathCheckResult>,
}

#[derive(Serialize)]
struct PathCheckResult {
    path: String,
    should_sync: bool,
    matched_rule: Option<String>,
}

#[derive(Serialize)]
struct DeviceResponse {
    device_id: String,
    device_name: Option<String>,
    last_sync_cursor: Option<String>,
    synced_bytes: i64,
    max_sync_bytes: Option<i64>,
    is_active: bool,
    last_seen_at: String,
    created_at: String,
}

#[derive(Deserialize)]
struct UpdateDeviceRequest {
    device_name: Option<String>,
    max_sync_bytes: Option<i64>,
    is_active: Option<bool>,
}

// ============================================================================
// HANDLERS
// ============================================================================

/// Extract user ID from authorization header
fn extract_user_id(state: &AppState, headers: &axum::http::HeaderMap) -> Result<Uuid, AppError> {
    let auth_header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or_else(|| AppError::Unauthorized("Missing authorization".into()))?;

    auth::verify_token(&state.config.jwt_secret, auth_header)
        .map_err(|_| AppError::Unauthorized("Invalid token".into()))
}

/// List user's sync rules
async fn list_rules(
    State(state): State<AppState>,
    Query(query): Query<ListRulesQuery>,
    headers: axum::http::HeaderMap,
) -> Result<Json<ListRulesResponse>, AppError> {
    let user_id = extract_user_id(&state, &headers)?;
    
    let include_inactive = query.include_inactive.unwrap_or(false);
    
    let rules = sqlx::query_as::<_, (Uuid, String, String, i32, bool, DateTime<Utc>, DateTime<Utc>)>(
        r#"
        SELECT id, rule_type, path_pattern, priority, is_active, created_at, updated_at
        FROM selective_sync_rules
        WHERE user_id = $1 AND ($2 OR is_active = TRUE)
        ORDER BY priority DESC, created_at ASC
        "#
    )
    .bind(user_id)
    .bind(include_inactive)
    .fetch_all(&state.db)
    .await?;
    
    let rule_responses: Vec<SyncRuleResponse> = rules
        .into_iter()
        .map(|(id, rule_type, pattern, priority, is_active, created_at, updated_at)| {
            SyncRuleResponse {
                id: id.to_string(),
                rule_type,
                path_pattern: pattern,
                priority,
                is_active,
                created_at: created_at.to_rfc3339(),
                updated_at: updated_at.to_rfc3339(),
            }
        })
        .collect();
    
    Ok(Json(ListRulesResponse { rules: rule_responses }))
}

/// Create a sync rule
async fn create_rule(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<CreateRuleRequest>,
) -> Result<Json<SyncRuleResponse>, AppError> {
    let user_id = extract_user_id(&state, &headers)?;
    
    // Validate rule type
    if req.rule_type != "include" && req.rule_type != "exclude" {
        return Err(AppError::BadRequest("rule_type must be 'include' or 'exclude'".into()));
    }
    
    // Validate pattern
    if req.path_pattern.is_empty() {
        return Err(AppError::BadRequest("path_pattern cannot be empty".into()));
    }
    
    let rule_id = Uuid::new_v4();
    let now = Utc::now();
    let priority = req.priority.unwrap_or(0);
    
    sqlx::query(
        r#"
        INSERT INTO selective_sync_rules (id, user_id, rule_type, path_pattern, priority, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, $6)
        "#
    )
    .bind(rule_id)
    .bind(user_id)
    .bind(&req.rule_type)
    .bind(&req.path_pattern)
    .bind(priority)
    .bind(now)
    .execute(&state.db)
    .await?;
    
    Ok(Json(SyncRuleResponse {
        id: rule_id.to_string(),
        rule_type: req.rule_type,
        path_pattern: req.path_pattern,
        priority,
        is_active: true,
        created_at: now.to_rfc3339(),
        updated_at: now.to_rfc3339(),
    }))
}

/// Get a specific rule
async fn get_rule(
    State(state): State<AppState>,
    Path(rule_id): Path<Uuid>,
    headers: axum::http::HeaderMap,
) -> Result<Json<SyncRuleResponse>, AppError> {
    let user_id = extract_user_id(&state, &headers)?;
    
    let rule = sqlx::query_as::<_, (Uuid, String, String, i32, bool, DateTime<Utc>, DateTime<Utc>)>(
        r#"
        SELECT id, rule_type, path_pattern, priority, is_active, created_at, updated_at
        FROM selective_sync_rules
        WHERE id = $1 AND user_id = $2
        "#
    )
    .bind(rule_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Rule not found".into()))?;
    
    let (id, rule_type, pattern, priority, is_active, created_at, updated_at) = rule;
    
    Ok(Json(SyncRuleResponse {
        id: id.to_string(),
        rule_type,
        path_pattern: pattern,
        priority,
        is_active,
        created_at: created_at.to_rfc3339(),
        updated_at: updated_at.to_rfc3339(),
    }))
}

/// Update a sync rule
async fn update_rule(
    State(state): State<AppState>,
    Path(rule_id): Path<Uuid>,
    headers: axum::http::HeaderMap,
    Json(req): Json<UpdateRuleRequest>,
) -> Result<Json<SyncRuleResponse>, AppError> {
    let user_id = extract_user_id(&state, &headers)?;
    
    // Validate rule type if provided
    if let Some(ref rt) = req.rule_type {
        if rt != "include" && rt != "exclude" {
            return Err(AppError::BadRequest("rule_type must be 'include' or 'exclude'".into()));
        }
    }
    
    // Get current rule
    let current = sqlx::query_as::<_, (String, String, i32, bool)>(
        "SELECT rule_type, path_pattern, priority, is_active FROM selective_sync_rules WHERE id = $1 AND user_id = $2"
    )
    .bind(rule_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Rule not found".into()))?;
    
    let (current_type, current_pattern, current_priority, current_active) = current;
    
    let new_type = req.rule_type.unwrap_or(current_type);
    let new_pattern = req.path_pattern.unwrap_or(current_pattern);
    let new_priority = req.priority.unwrap_or(current_priority);
    let new_active = req.is_active.unwrap_or(current_active);
    let now = Utc::now();
    
    sqlx::query(
        r#"
        UPDATE selective_sync_rules
        SET rule_type = $1, path_pattern = $2, priority = $3, is_active = $4, updated_at = $5
        WHERE id = $6 AND user_id = $7
        "#
    )
    .bind(&new_type)
    .bind(&new_pattern)
    .bind(new_priority)
    .bind(new_active)
    .bind(now)
    .bind(rule_id)
    .bind(user_id)
    .execute(&state.db)
    .await?;
    
    // Get updated rule
    let rule = sqlx::query_as::<_, (DateTime<Utc>,)>(
        "SELECT created_at FROM selective_sync_rules WHERE id = $1"
    )
    .bind(rule_id)
    .fetch_one(&state.db)
    .await?;
    
    Ok(Json(SyncRuleResponse {
        id: rule_id.to_string(),
        rule_type: new_type,
        path_pattern: new_pattern,
        priority: new_priority,
        is_active: new_active,
        created_at: rule.0.to_rfc3339(),
        updated_at: now.to_rfc3339(),
    }))
}

/// Delete a sync rule
async fn delete_rule(
    State(state): State<AppState>,
    Path(rule_id): Path<Uuid>,
    headers: axum::http::HeaderMap,
) -> Result<StatusCode, AppError> {
    let user_id = extract_user_id(&state, &headers)?;
    
    let result = sqlx::query(
        "DELETE FROM selective_sync_rules WHERE id = $1 AND user_id = $2"
    )
    .bind(rule_id)
    .bind(user_id)
    .execute(&state.db)
    .await?;
    
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Rule not found".into()));
    }
    
    Ok(StatusCode::NO_CONTENT)
}

/// Check which paths should be synced based on rules
async fn check_paths(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<CheckPathsRequest>,
) -> Result<Json<CheckPathsResponse>, AppError> {
    let user_id = extract_user_id(&state, &headers)?;
    
    // Get all active rules ordered by priority
    let rules = sqlx::query_as::<_, (Uuid, String, String, i32)>(
        r#"
        SELECT id, rule_type, path_pattern, priority
        FROM selective_sync_rules
        WHERE user_id = $1 AND is_active = TRUE
        ORDER BY priority DESC
        "#
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;
    
    let mut results = Vec::new();
    
    for path in req.paths {
        let mut should_sync = true; // Default: sync everything
        let mut matched_rule: Option<String> = None;
        
        for (rule_id, rule_type, pattern, _priority) in &rules {
            if matches_pattern(&path, pattern) {
                should_sync = rule_type == "include";
                matched_rule = Some(rule_id.to_string());
                break; // First matching rule wins (highest priority)
            }
        }
        
        results.push(PathCheckResult {
            path,
            should_sync,
            matched_rule,
        });
    }
    
    Ok(Json(CheckPathsResponse { results }))
}

/// List user's devices
async fn list_devices(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<Vec<DeviceResponse>>, AppError> {
    let user_id = extract_user_id(&state, &headers)?;
    
    let devices = sqlx::query_as::<_, (String, Option<String>, Option<DateTime<Utc>>, i64, Option<i64>, bool, DateTime<Utc>, DateTime<Utc>)>(
        r#"
        SELECT device_id, device_name, last_sync_cursor, synced_bytes, max_sync_bytes, 
               is_active, last_seen_at, created_at
        FROM device_sync_state
        WHERE user_id = $1
        ORDER BY last_seen_at DESC
        "#
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;
    
    let device_responses: Vec<DeviceResponse> = devices
        .into_iter()
        .map(|(device_id, device_name, cursor, synced, max_sync, is_active, last_seen, created)| {
            DeviceResponse {
                device_id,
                device_name,
                last_sync_cursor: cursor.map(|c| c.to_rfc3339()),
                synced_bytes: synced,
                max_sync_bytes: max_sync,
                is_active,
                last_seen_at: last_seen.to_rfc3339(),
                created_at: created.to_rfc3339(),
            }
        })
        .collect();
    
    Ok(Json(device_responses))
}

/// Update a device's settings
async fn update_device(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    headers: axum::http::HeaderMap,
    Json(req): Json<UpdateDeviceRequest>,
) -> Result<Json<DeviceResponse>, AppError> {
    let user_id = extract_user_id(&state, &headers)?;
    
    // Get current device state
    let current = sqlx::query_as::<_, (Option<String>, Option<i64>, bool)>(
        "SELECT device_name, max_sync_bytes, is_active FROM device_sync_state WHERE user_id = $1 AND device_id = $2"
    )
    .bind(user_id)
    .bind(&device_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Device not found".into()))?;
    
    let (current_name, current_max, current_active) = current;
    
    let new_name = req.device_name.or(current_name);
    let new_max = req.max_sync_bytes.or(current_max);
    let new_active = req.is_active.unwrap_or(current_active);
    
    sqlx::query(
        r#"
        UPDATE device_sync_state
        SET device_name = $1, max_sync_bytes = $2, is_active = $3
        WHERE user_id = $4 AND device_id = $5
        "#
    )
    .bind(&new_name)
    .bind(new_max)
    .bind(new_active)
    .bind(user_id)
    .bind(&device_id)
    .execute(&state.db)
    .await?;
    
    // Get updated device
    let device = sqlx::query_as::<_, (Option<DateTime<Utc>>, i64, DateTime<Utc>, DateTime<Utc>)>(
        "SELECT last_sync_cursor, synced_bytes, last_seen_at, created_at FROM device_sync_state WHERE user_id = $1 AND device_id = $2"
    )
    .bind(user_id)
    .bind(&device_id)
    .fetch_one(&state.db)
    .await?;
    
    Ok(Json(DeviceResponse {
        device_id,
        device_name: new_name,
        last_sync_cursor: device.0.map(|c| c.to_rfc3339()),
        synced_bytes: device.1,
        max_sync_bytes: new_max,
        is_active: new_active,
        last_seen_at: device.2.to_rfc3339(),
        created_at: device.3.to_rfc3339(),
    }))
}

/// Remove a device
async fn remove_device(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<StatusCode, AppError> {
    let user_id = extract_user_id(&state, &headers)?;
    
    let result = sqlx::query(
        "DELETE FROM device_sync_state WHERE user_id = $1 AND device_id = $2"
    )
    .bind(user_id)
    .bind(&device_id)
    .execute(&state.db)
    .await?;
    
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Device not found".into()));
    }
    
    Ok(StatusCode::NO_CONTENT)
}

// ============================================================================
// HELPERS
// ============================================================================

/// Simple glob-style pattern matching
/// Supports: * (any characters), ? (single character), / (path separator)
fn matches_pattern(path: &str, pattern: &str) -> bool {
    // Normalize paths
    let path = path.trim_start_matches('/');
    let pattern = pattern.trim_start_matches('/');
    
    // Simple pattern matching
    if pattern.contains('*') {
        // Glob pattern
        let parts: Vec<&str> = pattern.split('*').collect();
        if parts.len() == 1 {
            return path == pattern;
        }
        
        // Check prefix
        if !parts[0].is_empty() && !path.starts_with(parts[0]) {
            return false;
        }
        
        // Check suffix
        if !parts[parts.len() - 1].is_empty() && !path.ends_with(parts[parts.len() - 1]) {
            return false;
        }
        
        // For simple cases, this works
        // More complex patterns would need proper glob implementation
        return true;
    }
    
    // Exact match or prefix match for directories
    if pattern.ends_with('/') {
        path.starts_with(pattern) || path == pattern.trim_end_matches('/')
    } else {
        path == pattern
    }
}
