//! Conflict detection and resolution routes
//!
//! Handles sync conflict detection, listing, and resolution.

use crate::api::AppState;
use crate::auth;
use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::error::AppError;

// ============================================================================
// ROUTES
// ============================================================================

pub fn conflict_routes() -> Router<AppState> {
    Router::new()
        .route("/conflicts", get(list_conflicts))
        .route("/conflicts/:id", get(get_conflict))
        .route("/conflicts/:id/resolve", post(resolve_conflict))
        .route("/conflicts/detect", post(detect_conflicts))
}

// ============================================================================
// TYPES
// ============================================================================

#[derive(Debug, Clone, sqlx::FromRow)]
#[allow(dead_code)]
pub struct SyncConflict {
    pub id: Uuid,
    pub file_id: Uuid,
    pub user_id: Uuid,
    pub local_version_id: Option<Uuid>,
    pub remote_version_id: Option<Uuid>,
    pub conflict_type: String,
    pub resolved_at: Option<DateTime<Utc>>,
    pub resolution: Option<String>,
    pub resolved_by: Option<Uuid>,
    pub detected_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct ConflictResponse {
    id: String,
    file_id: String,
    file_path: String,
    conflict_type: String,
    local_version: Option<VersionInfo>,
    remote_version: Option<VersionInfo>,
    detected_at: String,
    resolved_at: Option<String>,
    resolution: Option<String>,
}

#[derive(Serialize, Clone)]
struct VersionInfo {
    id: String,
    size_bytes: i64,
    blob_hash: String,
    created_at: String,
}

#[derive(Deserialize)]
struct ListConflictsQuery {
    include_resolved: Option<bool>,
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Serialize)]
struct ListConflictsResponse {
    conflicts: Vec<ConflictResponse>,
    total: i64,
}

#[derive(Deserialize)]
struct ResolveConflictRequest {
    resolution: String, // 'keep_local', 'keep_remote', 'keep_both'
}

#[derive(Serialize)]
struct ResolveConflictResponse {
    message: String,
    conflict_id: String,
    resolution: String,
}

#[derive(Deserialize)]
struct DetectConflictsRequest {
    /// List of files to check for conflicts
    files: Vec<FileCheckRequest>,
}

#[derive(Deserialize)]
struct FileCheckRequest {
    path: String,
    /// Local file's hash
    local_hash: String,
    /// Local modification timestamp
    local_modified_at: String,
}

#[derive(Serialize)]
struct DetectConflictsResponse {
    conflicts: Vec<DetectedConflict>,
}

#[derive(Serialize)]
struct DetectedConflict {
    path: String,
    conflict_type: String,
    local_hash: String,
    remote_hash: Option<String>,
    requires_action: bool,
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

/// List user's sync conflicts
async fn list_conflicts(
    State(state): State<AppState>,
    Query(query): Query<ListConflictsQuery>,
    headers: axum::http::HeaderMap,
) -> Result<Json<ListConflictsResponse>, AppError> {
    let user_id = extract_user_id(&state, &headers)?;
    
    let include_resolved = query.include_resolved.unwrap_or(false);
    let limit = query.limit.unwrap_or(50);
    let offset = query.offset.unwrap_or(0);
    
    // Query conflicts with file path
    let conflicts = if include_resolved {
        sqlx::query_as::<_, (Uuid, Uuid, String, Option<Uuid>, Option<Uuid>, String, Option<DateTime<Utc>>, Option<String>, DateTime<Utc>, String)>(
            r#"
            SELECT c.id, c.file_id, c.conflict_type, c.local_version_id, c.remote_version_id,
                   COALESCE(c.resolution, ''), c.resolved_at, COALESCE(c.resolution, ''), c.detected_at, f.path
            FROM sync_conflicts c
            JOIN files f ON c.file_id = f.id
            WHERE c.user_id = $1
            ORDER BY c.detected_at DESC
            LIMIT $2 OFFSET $3
            "#
        )
        .bind(user_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.db)
        .await?
    } else {
        sqlx::query_as::<_, (Uuid, Uuid, String, Option<Uuid>, Option<Uuid>, String, Option<DateTime<Utc>>, Option<String>, DateTime<Utc>, String)>(
            r#"
            SELECT c.id, c.file_id, c.conflict_type, c.local_version_id, c.remote_version_id,
                   COALESCE(c.resolution, ''), c.resolved_at, COALESCE(c.resolution, ''), c.detected_at, f.path
            FROM sync_conflicts c
            JOIN files f ON c.file_id = f.id
            WHERE c.user_id = $1 AND c.resolved_at IS NULL
            ORDER BY c.detected_at DESC
            LIMIT $2 OFFSET $3
            "#
        )
        .bind(user_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.db)
        .await?
    };
    
    // Get total count
    let total: (i64,) = if include_resolved {
        sqlx::query_as("SELECT COUNT(*) FROM sync_conflicts WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(&state.db)
            .await?
    } else {
        sqlx::query_as("SELECT COUNT(*) FROM sync_conflicts WHERE user_id = $1 AND resolved_at IS NULL")
            .bind(user_id)
            .fetch_one(&state.db)
            .await?
    };
    
    // Collect all version IDs for batch fetching
    let version_ids: Vec<Uuid> = conflicts
        .iter()
        .flat_map(|(_, _, _, local_v, remote_v, _, _, _, _, _)| {
            [local_v.clone(), remote_v.clone()].into_iter().flatten()
        })
        .collect();
    
    // Batch fetch version info
    let versions: std::collections::HashMap<Uuid, VersionInfo> = if !version_ids.is_empty() {
        sqlx::query_as::<_, (Uuid, i64, String, DateTime<Utc>)>(
            "SELECT id, size_bytes, blob_hash, created_at FROM versions WHERE id = ANY($1)"
        )
        .bind(&version_ids)
        .fetch_all(&state.db)
        .await?
        .into_iter()
        .map(|(id, size, hash, created)| {
            (id, VersionInfo {
                id: id.to_string(),
                size_bytes: size,
                blob_hash: hash,
                created_at: created.to_rfc3339(),
            })
        })
        .collect()
    } else {
        std::collections::HashMap::new()
    };
    
    let conflict_responses: Vec<ConflictResponse> = conflicts
        .into_iter()
        .map(|(id, file_id, conflict_type, local_v, remote_v, _res, resolved_at, resolution, detected_at, path)| {
            ConflictResponse {
                id: id.to_string(),
                file_id: file_id.to_string(),
                file_path: path,
                conflict_type,
                local_version: local_v.and_then(|vid| versions.get(&vid).cloned()),
                remote_version: remote_v.and_then(|vid| versions.get(&vid).cloned()),
                detected_at: detected_at.to_rfc3339(),
                resolved_at: resolved_at.map(|t| t.to_rfc3339()),
                resolution: if resolution.as_ref().map(|s| s.is_empty()).unwrap_or(true) { None } else { resolution },
            }
        })
        .collect();
    
    Ok(Json(ListConflictsResponse {
        conflicts: conflict_responses,
        total: total.0,
    }))
}

/// Get a specific conflict with full details
async fn get_conflict(
    State(state): State<AppState>,
    Path(conflict_id): Path<Uuid>,
    headers: axum::http::HeaderMap,
) -> Result<Json<ConflictResponse>, AppError> {
    let user_id = extract_user_id(&state, &headers)?;
    
    let conflict = sqlx::query_as::<_, (Uuid, Uuid, String, Option<Uuid>, Option<Uuid>, Option<DateTime<Utc>>, Option<String>, DateTime<Utc>, String)>(
        r#"
        SELECT c.id, c.file_id, c.conflict_type, c.local_version_id, c.remote_version_id,
               c.resolved_at, c.resolution, c.detected_at, f.path
        FROM sync_conflicts c
        JOIN files f ON c.file_id = f.id
        WHERE c.id = $1 AND c.user_id = $2
        "#
    )
    .bind(conflict_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Conflict not found".into()))?;
    
    let (id, file_id, conflict_type, local_version_id, remote_version_id, resolved_at, resolution, detected_at, path) = conflict;
    
    // Fetch version info if available
    let local_version = if let Some(version_id) = local_version_id {
        sqlx::query_as::<_, (Uuid, i64, String, DateTime<Utc>)>(
            "SELECT id, size_bytes, blob_hash, created_at FROM versions WHERE id = $1"
        )
        .bind(version_id)
        .fetch_optional(&state.db)
        .await?
        .map(|(id, size, hash, created)| VersionInfo {
            id: id.to_string(),
            size_bytes: size,
            blob_hash: hash,
            created_at: created.to_rfc3339(),
        })
    } else {
        None
    };
    
    let remote_version = if let Some(version_id) = remote_version_id {
        sqlx::query_as::<_, (Uuid, i64, String, DateTime<Utc>)>(
            "SELECT id, size_bytes, blob_hash, created_at FROM versions WHERE id = $1"
        )
        .bind(version_id)
        .fetch_optional(&state.db)
        .await?
        .map(|(id, size, hash, created)| VersionInfo {
            id: id.to_string(),
            size_bytes: size,
            blob_hash: hash,
            created_at: created.to_rfc3339(),
        })
    } else {
        None
    };
    
    Ok(Json(ConflictResponse {
        id: id.to_string(),
        file_id: file_id.to_string(),
        file_path: path,
        conflict_type,
        local_version,
        remote_version,
        detected_at: detected_at.to_rfc3339(),
        resolved_at: resolved_at.map(|t| t.to_rfc3339()),
        resolution,
    }))
}

/// Resolve a conflict
async fn resolve_conflict(
    State(state): State<AppState>,
    Path(conflict_id): Path<Uuid>,
    headers: axum::http::HeaderMap,
    Json(req): Json<ResolveConflictRequest>,
) -> Result<Json<ResolveConflictResponse>, AppError> {
    let user_id = extract_user_id(&state, &headers)?;
    
    // Validate resolution type
    let valid_resolutions = ["keep_local", "keep_remote", "keep_both"];
    if !valid_resolutions.contains(&req.resolution.as_str()) {
        return Err(AppError::BadRequest(format!(
            "Invalid resolution. Must be one of: {}",
            valid_resolutions.join(", ")
        )));
    }
    
    // Get conflict
    let conflict = sqlx::query_as::<_, (Uuid, Uuid, Option<Uuid>, Option<Uuid>)>(
        r#"
        SELECT id, file_id, local_version_id, remote_version_id
        FROM sync_conflicts
        WHERE id = $1 AND user_id = $2 AND resolved_at IS NULL
        "#
    )
    .bind(conflict_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Conflict not found or already resolved".into()))?;
    
    let (_, file_id, local_version_id, remote_version_id) = conflict;
    
    // Apply resolution
    match req.resolution.as_str() {
        "keep_local" => {
            // Set local version as current
            if let Some(version_id) = local_version_id {
                sqlx::query("UPDATE files SET current_version_id = $1, updated_at = NOW() WHERE id = $2")
                    .bind(version_id)
                    .bind(file_id)
                    .execute(&state.db)
                    .await?;
            }
        }
        "keep_remote" => {
            // Set remote version as current (already is, usually)
            if let Some(version_id) = remote_version_id {
                sqlx::query("UPDATE files SET current_version_id = $1, updated_at = NOW() WHERE id = $2")
                    .bind(version_id)
                    .bind(file_id)
                    .execute(&state.db)
                    .await?;
            }
        }
        "keep_both" => {
            // Create a conflict copy with local version
            if let Some(_local_version) = local_version_id {
                // Get original file path
                let (path,): (String,) = sqlx::query_as("SELECT path FROM files WHERE id = $1")
                    .bind(file_id)
                    .fetch_one(&state.db)
                    .await?;
                
                // Create conflict copy path
                let conflict_path = create_conflict_path(&path);
                
                // Create new file with local version
                sqlx::query(
                    r#"
                    INSERT INTO files (path, current_version_id, owner_id, created_at, updated_at)
                    SELECT $1, $2, owner_id, NOW(), NOW()
                    FROM files WHERE id = $3
                    "#
                )
                .bind(&conflict_path)
                .bind(local_version_id)
                .bind(file_id)
                .execute(&state.db)
                .await?;
            }
        }
        _ => {}
    }
    
    // Mark conflict as resolved
    sqlx::query(
        r#"
        UPDATE sync_conflicts
        SET resolved_at = NOW(), resolution = $1, resolved_by = $2
        WHERE id = $3
        "#
    )
    .bind(&req.resolution)
    .bind(user_id)
    .bind(conflict_id)
    .execute(&state.db)
    .await?;
    
    Ok(Json(ResolveConflictResponse {
        message: "Conflict resolved successfully".into(),
        conflict_id: conflict_id.to_string(),
        resolution: req.resolution,
    }))
}

/// Detect conflicts for a batch of files
async fn detect_conflicts(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<DetectConflictsRequest>,
) -> Result<Json<DetectConflictsResponse>, AppError> {
    let user_id = extract_user_id(&state, &headers)?;
    
    let mut detected = Vec::new();
    
    for file_check in req.files {
        // Get remote file state
        let remote = sqlx::query_as::<_, (Uuid, Option<String>, DateTime<Utc>)>(
            r#"
            SELECT f.id, v.blob_hash, f.updated_at
            FROM files f
            LEFT JOIN versions v ON f.current_version_id = v.id
            WHERE f.path = $1 AND f.is_deleted = FALSE
              AND (f.owner_id = $2 OR f.owner_id IS NULL)
            "#
        )
        .bind(&file_check.path)
        .bind(user_id)
        .fetch_optional(&state.db)
        .await?;
        
        if let Some((file_id, remote_hash, remote_modified)) = remote {
            let remote_hash_str = remote_hash.unwrap_or_default();
            
            // Compare hashes to detect conflict
            if !remote_hash_str.is_empty() && remote_hash_str != file_check.local_hash {
                // Parse local modified time
                let local_modified = chrono::DateTime::parse_from_rfc3339(&file_check.local_modified_at)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                
                // Both modified since last sync - conflict
                let conflict_type = if local_modified > remote_modified {
                    "edit_edit" // Both sides edited
                } else {
                    "edit_edit" // Could differentiate based on timestamps
                };
                
                // Create conflict record
                sqlx::query(
                    r#"
                    INSERT INTO sync_conflicts (file_id, user_id, conflict_type, detected_at)
                    VALUES ($1, $2, $3, NOW())
                    ON CONFLICT DO NOTHING
                    "#
                )
                .bind(file_id)
                .bind(user_id)
                .bind(conflict_type)
                .execute(&state.db)
                .await?;
                
                detected.push(DetectedConflict {
                    path: file_check.path,
                    conflict_type: conflict_type.to_string(),
                    local_hash: file_check.local_hash,
                    remote_hash: Some(remote_hash_str),
                    requires_action: true,
                });
            }
        }
    }
    
    Ok(Json(DetectConflictsResponse { conflicts: detected }))
}

// ============================================================================
// HELPERS
// ============================================================================

/// Create a conflict path like "file (conflict 2023-12-23).txt"
fn create_conflict_path(original_path: &str) -> String {
    let now = Utc::now();
    let date_str = now.format("%Y-%m-%d_%H%M%S").to_string();
    
    if let Some(dot_pos) = original_path.rfind('.') {
        let (name, ext) = original_path.split_at(dot_pos);
        format!("{} (conflict {}){}", name, date_str, ext)
    } else {
        format!("{} (conflict {})", original_path, date_str)
    }
}
