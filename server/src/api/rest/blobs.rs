//! Blob storage routes
//!
//! Handles blob upload/download and file metadata creation.

use crate::api::AppState;
use crate::db::{files, versions};
use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use blake3;
use serde::Deserialize;

use super::error::{extract_user_id, validate_path, AppError};
use super::types::UploadResponse;

// ============================================================================
// ROUTES
// ============================================================================

pub fn metadata_routes() -> Router<AppState> {
    Router::new()
        .route("/metadata", post(create_file_metadata))
}

// ============================================================================
// TYPES
// ============================================================================

/// Create/update file metadata after blob is uploaded
#[derive(Deserialize)]
struct CreateFileRequest {
    path: String,
    blob_hash: String,
    size_bytes: i64,
    /// Original filesystem creation time (ISO8601)
    created_at: Option<String>,
    /// Original filesystem modification time (ISO8601)
    updated_at: Option<String>,
}

// ============================================================================
// HANDLERS
// ============================================================================

/// Raw binary blob upload - most efficient method
/// Client computes hash, uploads raw bytes to PUT /blobs/{hash}
pub async fn upload_blob(
    State(state): State<AppState>,
    Path(hash): Path<String>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Result<StatusCode, AppError> {
    let _user_id = extract_user_id(&state, &headers)?;
    
    // Verify the hash matches the content using BLAKE3
    let computed_hash = blake3::hash(&body).to_hex().to_string();
    
    if computed_hash != hash {
        return Err(AppError::BadRequest(format!(
            "Hash mismatch: expected {}, got {}", 
            hash, computed_hash
        )));
    }
    
    // Store blob if not exists (deduplication)
    if !state.blob_store.exists(&hash)? {
        state.blob_store.write(&hash, &body)?;
    }
    
    Ok(StatusCode::CREATED)
}

/// Download raw blob by hash
pub async fn download_blob(
    State(state): State<AppState>,
    Path(hash): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let _user_id = extract_user_id(&state, &headers)?;
    
    if !state.blob_store.exists(&hash)? {
        return Err(AppError::NotFound("Blob not found".into()));
    }
    
    let content = state.blob_store.read(&hash)?;
    
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/octet-stream")],
        content,
    ))
}

async fn create_file_metadata(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<CreateFileRequest>,
) -> Result<Json<UploadResponse>, AppError> {
    let _user_id = extract_user_id(&state, &headers)?;
    
    // SECURITY: Validate path to prevent path traversal
    validate_path(&req.path)?;
    
    // Verify blob exists
    if !state.blob_store.exists(&req.blob_hash)? {
        return Err(AppError::BadRequest("Blob not found - upload blob first".into()));
    }
    
    // Parse optional client-provided dates
    fn parse_date(s: &Option<String>) -> Option<chrono::DateTime<chrono::Utc>> {
        s.as_ref().and_then(|ds| {
            chrono::DateTime::parse_from_rfc3339(ds)
                .ok()
                .map(|dt| dt.with_timezone(&chrono::Utc))
        })
    }
    let created_at = parse_date(&req.created_at);
    let updated_at = parse_date(&req.updated_at);
    
    // Upsert file record with client-provided dates (shared folder system - no ownership)
    let file = files::upsert_file_with_dates(&state.db, &req.path, created_at, updated_at).await?;

    // Check if current version already has this hash (skip duplicate versions)
    if let Some(current_version_id) = file.current_version_id {
        if let Ok(Some(current_version)) = versions::get_version(&state.db, current_version_id).await {
            if current_version.blob_hash == req.blob_hash {
                // Same content, no new version needed
                return Ok(Json(UploadResponse {
                    id: file.id.to_string(),
                    path: req.path,
                    blob_hash: req.blob_hash,
                    size_bytes: req.size_bytes,
                }));
            }
        }
    }

    // Create new version (only if content changed) without user tracking (shared folder system)
    let version = versions::create_version_global(
        &state.db,
        file.id,
        &req.blob_hash,
        req.size_bytes,
    ).await?;
    
    // Update current version
    files::set_current_version(&state.db, file.id, version.id).await?;
    
    Ok(Json(UploadResponse {
        id: file.id.to_string(),
        path: req.path,
        blob_hash: req.blob_hash,
        size_bytes: req.size_bytes,
    }))
}
