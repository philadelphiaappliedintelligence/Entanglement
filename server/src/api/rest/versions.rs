//! Version history routes
//!
//! Handles file version listing and restoration.

use crate::api::AppState;
use crate::db::{files, versions};
use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::error::{extract_user_id, AppError};

// ============================================================================
// TYPES
// ============================================================================

#[derive(Serialize)]
pub struct VersionResponse {
    pub id: String,
    pub blob_hash: String,
    pub size_bytes: i64,
    pub created_at: String,
    pub created_by: String,
}

#[derive(Serialize)]
pub struct ListVersionsResponse {
    pub versions: Vec<VersionResponse>,
    pub total: i64,
}

#[derive(Deserialize)]
pub struct ListVersionsQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Serialize)]
pub struct RestoreResponse {
    pub success: bool,
    pub new_version_id: String,
}

// ============================================================================
// HANDLERS
// ============================================================================

pub async fn list_file_versions(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ListVersionsQuery>,
    headers: axum::http::HeaderMap,
) -> Result<Json<ListVersionsResponse>, AppError> {
    let _user_id = extract_user_id(&state, &headers)?;
    let file_id = Uuid::parse_str(&id).map_err(|_| AppError::BadRequest("Invalid file ID".into()))?;

    // No ownership check for shared folder system
    let _file = files::get_file_by_id_global(&state.db, file_id)
        .await?
        .ok_or_else(|| AppError::NotFound("File not found".into()))?;

    let (version_list, total) = versions::list_versions(
        &state.db,
        file_id,
        query.limit.unwrap_or(50),
        query.offset.unwrap_or(0),
    )
    .await?;

    let versions = version_list
        .into_iter()
        .map(|v| VersionResponse {
            id: v.id.to_string(),
            blob_hash: v.blob_hash,
            size_bytes: v.size_bytes,
            created_at: v.created_at.to_rfc3339(),
            created_by: v.created_by.map(|u| u.to_string()).unwrap_or_default(),
        })
        .collect();

    Ok(Json(ListVersionsResponse { versions, total }))
}

pub async fn restore_version(
    State(state): State<AppState>,
    Path((file_id, version_id)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
) -> Result<Json<RestoreResponse>, AppError> {
    let _user_id = extract_user_id(&state, &headers)?;
    let file_id =
        Uuid::parse_str(&file_id).map_err(|_| AppError::BadRequest("Invalid file ID".into()))?;
    let version_id = Uuid::parse_str(&version_id)
        .map_err(|_| AppError::BadRequest("Invalid version ID".into()))?;

    // No ownership check for shared folder system
    let file = files::get_file_by_id_global(&state.db, file_id)
        .await?
        .ok_or_else(|| AppError::NotFound("File not found".into()))?;

    // Get the version to restore
    let old_version = versions::get_version(&state.db, version_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Version not found".into()))?;

    // Verify version belongs to this file
    if old_version.file_id != file_id {
        return Err(AppError::BadRequest(
            "Version does not belong to this file".into(),
        ));
    }

    // Create a new version with the same blob hash
    let new_version = versions::create_version_global(
        &state.db,
        file.id,
        &old_version.blob_hash,
        old_version.size_bytes,
    )
    .await?;

    // Update current version and undelete if needed
    files::set_current_version(&state.db, file.id, new_version.id).await?;
    if file.is_deleted {
        files::undelete(&state.db, file.id).await?;
    }

    Ok(Json(RestoreResponse {
        success: true,
        new_version_id: new_version.id.to_string(),
    }))
}
