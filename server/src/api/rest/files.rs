//! File operations routes
//!
//! CRUD operations for files including list, get, update, delete, and download.

use crate::api::AppState;
use crate::db::{chunks, files, versions};
use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use blake3;
use serde::Deserialize;
use uuid::Uuid;

use super::blobs::{upload_blob, download_blob};
use super::chunks::{check_chunks, upload_chunk, download_chunk, create_chunked_file, get_file_chunks};
use super::error::{extract_user_id, validate_path, AppError};
use super::types::{FileResponse, ListFilesQuery, ListFilesResponse, UploadResponse};
use super::versions::{list_file_versions, restore_version};

// ============================================================================
// ROUTES
// ============================================================================

pub fn file_routes() -> Router<AppState> {
    Router::new()
        .route("/files", get(list_files))
        .route("/files", axum::routing::post(upload_file))
        .route("/files/:id", get(get_file))
        .route("/files/:id", axum::routing::patch(update_file))
        .route("/files/:id", axum::routing::delete(delete_file))
        .route("/files/:id/download", get(download_file))
        .route("/files/:id/versions", get(list_file_versions))
        .route("/files/:id/restore/:version_id", axum::routing::post(restore_version))
        // Raw binary blob upload - most efficient
        .route("/blobs/:hash", axum::routing::put(upload_blob))
        .route("/blobs/:hash", get(download_blob))
        // Chunk-based upload/download (CDC for delta sync)
        .route("/chunks/check", axum::routing::post(check_chunks))
        .route("/chunks/:hash", axum::routing::put(upload_chunk))
        .route("/chunks/:hash", get(download_chunk))
        .route("/files/chunked", axum::routing::post(create_chunked_file))
        .route("/files/:id/chunks", get(get_file_chunks))
}

// ============================================================================
// TYPES
// ============================================================================

#[derive(Deserialize)]
struct UpdateFileRequest {
    path: String,
}

/// Upload file endpoint - accepts JSON with path and base64 content
#[derive(Deserialize)]
struct UploadRequest {
    path: String,
    content: String,  // base64 encoded
}

// ============================================================================
// HANDLERS
// ============================================================================

async fn upload_file(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<UploadRequest>,
) -> Result<Json<UploadResponse>, AppError> {
    let _user_id = extract_user_id(&state, &headers)?;
    
    // SECURITY: Validate path to prevent path traversal
    validate_path(&req.path)?;
    
    // Decode base64 content
    use base64::{Engine, engine::general_purpose::STANDARD};
    let content = STANDARD.decode(&req.content)
        .map_err(|e| AppError::BadRequest(format!("Invalid base64: {}", e)))?;
    
    // Compute hash using BLAKE3
    let blob_hash = blake3::hash(&content).to_hex().to_string();
    
    // Store blob
    if !state.blob_store.exists(&blob_hash)? {
        state.blob_store.write(&blob_hash, &content)?;
    }
    
    // Upsert file record (shared folder system - no ownership)
    let file = files::upsert_file_global(&state.db, &req.path).await?;

    // Create version without user tracking (shared folder system)
    let version = versions::create_version_global(
        &state.db,
        file.id,
        &blob_hash,
        content.len() as i64,
    ).await?;
    
    // Update current version
    files::set_current_version(&state.db, file.id, version.id).await?;
    
    // Notify connected clients about the new file (send actual path for menu bar display)
    state.sync_hub.notify_file_changed(&req.path, "create");
    
    Ok(Json(UploadResponse {
        id: file.id.to_string(),
        path: req.path,
        blob_hash,
        size_bytes: content.len() as i64,
    }))
}

async fn list_files(
    State(state): State<AppState>,
    Query(query): Query<ListFilesQuery>,
    headers: axum::http::HeaderMap,
) -> Result<Json<ListFilesResponse>, AppError> {
    let user_id = extract_user_id(&state, &headers)?;

    let (file_list, total) = files::list_files(
        &state.db,
        user_id,
        query.prefix.as_deref(),
        query.include_deleted.unwrap_or(false),
        query.limit.unwrap_or(100),
        query.offset.unwrap_or(0),
    )
    .await?;

    let files = file_list
        .into_iter()
        .map(|f| FileResponse {
            id: f.id.to_string(),
            path: f.path.clone(),
            size_bytes: f.size_bytes,
            blob_hash: f.blob_hash,
            is_directory: f.path.ends_with('/'),
            is_deleted: f.is_deleted,
            created_at: f.created_at.to_rfc3339(),
            updated_at: f.updated_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(ListFilesResponse { files, total }))
}

async fn get_file(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<Json<FileResponse>, AppError> {
    let _user_id = extract_user_id(&state, &headers)?;

    let file = if let Ok(file_id) = Uuid::parse_str(&id) {
        // Try UUID first (regular files)
        files::get_file_by_id_global(&state.db, file_id)
            .await?
            .ok_or_else(|| AppError::NotFound("File not found".into()))?
    } else if id.len() == 64 && id.chars().all(|c| c.is_ascii_hexdigit()) {
        // BLAKE3 Hash (Virtual Folder OR Materialized Folder with Sticky ID)

        // 1. Check if we have a real record that "claims" this hash (Sticky ID lookup)
        if let Some(file) = files::get_file_by_original_hash(&state.db, &id).await? {
            // Found materialized folder via Sticky ID
            files::get_file_by_id_global(&state.db, file.id)
                .await?
                .ok_or_else(|| AppError::NotFound("File not found".into()))?
        } else {
            // 2. Fallback to Virtual Resolution (scan paths for virtual folders)
            // Get all paths that determine structure
            let all_paths: Vec<String> = sqlx::query_scalar(
                "SELECT path FROM files WHERE is_deleted = FALSE"
            )
            .fetch_all(&state.db)
            .await?;

            let mut found_path = None;
            let mut seen_dirs = std::collections::HashSet::new();

            // Look for directory paths matching this hash
            for raw_path in all_paths {
                // Ensure path starts with / for processing
                let path = if raw_path.starts_with('/') {
                    raw_path.clone()
                } else {
                    format!("/{}", raw_path)
                };

                // Scan character by character for directory separators
                for (i, c) in path.chars().enumerate() {
                    if c == '/' && i > 0 {
                        // Found a directory path (e.g., "/music/")
                        let candidate = &path[0..=i];

                        // Clean double slashes
                        let clean_candidate = candidate.replace("//", "/");

                        // Avoid duplicate work
                        if seen_dirs.contains(&clean_candidate) {
                            continue;
                        }
                        seen_dirs.insert(clean_candidate.clone());

                        // Check if this path's hash matches the requested ID
                        let hash = blake3::hash(clean_candidate.as_bytes()).to_hex().to_string();

                        if hash == id {
                            found_path = Some(clean_candidate);
                            break;
                        }
                    }
                }
                if found_path.is_some() {
                    break;
                }
            }

            if let Some(virtual_path) = found_path {
                // Return a virtual folder response
                return Ok(Json(FileResponse {
                    id: id, // Return the hash as ID for virtual folders
                    path: virtual_path,
                    size_bytes: Some(0),
                    blob_hash: None,
                    is_directory: true,
                    is_deleted: false,
                    created_at: chrono::Utc::now().to_rfc3339(),
                    updated_at: chrono::Utc::now().to_rfc3339(),
                }));
            } else {
                return Err(AppError::NotFound("Folder not found".into()));
            }
        }
    } else {
        return Err(AppError::BadRequest("Invalid file ID".into()));
    };

    Ok(Json(FileResponse {
        // Return the original_hash_id if it exists (Sticky ID), otherwise use the UUID
        id: file.original_hash_id.unwrap_or(file.id.to_string()),
        path: file.path.clone(),
        size_bytes: file.size_bytes,
        blob_hash: file.blob_hash,
        is_directory: file.path.ends_with('/'),
        is_deleted: file.is_deleted,
        created_at: file.created_at.to_rfc3339(),
        updated_at: file.updated_at.to_rfc3339(),
    }))
}

async fn update_file(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    Json(req): Json<UpdateFileRequest>,
) -> Result<Json<FileResponse>, AppError> {
    // CRITICAL DEBUG LOG
    tracing::info!("=== UPDATE_FILE REQUEST ===");
    tracing::info!("ID: {}", id);
    tracing::info!("New path: {}", req.path);
    tracing::info!("ID type: {}", if id.len() == 64 && id.chars().all(|c| c.is_ascii_hexdigit()) { "HASH" } else if Uuid::parse_str(&id).is_ok() { "UUID" } else { "OTHER" });

    let user_id = extract_user_id(&state, &headers)?;

    // Validate new path
    if req.path.trim().is_empty() {
        tracing::error!("ERROR: Empty path in request");
        return Err(AppError::BadRequest("Path cannot be empty".into()));
    }

    // Try to parse as UUID first (Real File or Real Folder)
    let updated_file = if let Ok(file_id) = Uuid::parse_str(&id) {
        files::move_file(&state.db, file_id, &req.path, user_id).await?
    } else if id.len() == 64 && id.chars().all(|c| c.is_ascii_hexdigit()) {
        // BLAKE3 Hash (Virtual Folder OR Materialized Folder with Sticky ID)
        
        // 1. Check if we have a real record that "claims" this hash (Sticky ID)
        if let Some(existing_file) = files::get_file_by_original_hash(&state.db, &id).await? {
            tracing::warn!("DEBUG: Found materialized folder via Sticky ID: {}", id);
            files::move_file(&state.db, existing_file.id, &req.path, user_id).await?
        } else {
            // 2. Fallback to Virtual Resolution (Scan all paths)
            // We need to resolve the hash to a path by scanning existing files.
            // CRITICAL: We must replicate `list_directory`'s normalization exactly.
            
            let all_paths: Vec<String> = sqlx::query_scalar(
                "SELECT path FROM files WHERE is_deleted = FALSE"
            )
            .fetch_all(&state.db)
            .await?;
            
            let mut found_path = None;
            let mut seen_dirs = std::collections::HashSet::new();
            
            tracing::warn!("DEBUG: Resolving Virtual ID: {}", id);

            'search: for raw_path in all_paths {
                // Ensure path starts with / for processing
                let path = if raw_path.starts_with('/') {
                    raw_path.clone()
                } else {
                    format!("/{}", raw_path)
                };

                // Scan character by character for directory separators
                for (i, c) in path.chars().enumerate() {
                    if c == '/' && i > 0 {
                         // Found a separator at 'i'. 
                         // Substring [0..=i] is a candidate directory path (e.g. "/music/")
                         let candidate = &path[0..=i];
                         
                         // DOUBLE SLASH REGRESSION FIX:
                         // Ensure no double-slashes before hashing
                         let clean_candidate = candidate.replace("//", "/");
                         
                         if seen_dirs.contains(&clean_candidate) {
                             continue;
                         }
                         seen_dirs.insert(clean_candidate.clone());
                         
                         let hash = blake3::hash(clean_candidate.as_bytes()).to_hex().to_string();
                         
                         if hash == id {
                             tracing::warn!("DEBUG: MATCH FOUND! Path: {}", clean_candidate);
                             found_path = Some(clean_candidate);
                             break 'search;
                         }
                    }
                }
            }

            if let Some(resolved_path) = found_path {
                // Found it! resolved_path (e.g. "/music/ppooll/")
                tracing::warn!("DEBUG: Found virtual folder at path: {}", resolved_path);
                tracing::warn!("DEBUG: Moving to: {}", req.path);
                files::move_path(&state.db, &resolved_path, &req.path, user_id).await?
            } else {
                tracing::error!("DEBUG: FAILED to find path for ID: {}", id);
                return Err(AppError::NotFound(format!("Folder not found for ID {}", id)));
            }
        }
    } else {
        return Err(AppError::BadRequest("Invalid file ID".into()));
    };

    let response_id = updated_file.original_hash_id.clone().unwrap_or(updated_file.id.to_string());
    tracing::warn!("=== UPDATE_FILE RESPONSE ===");
    tracing::warn!("Response ID: {}", response_id);
    tracing::warn!("Response path: {}", updated_file.path);
    tracing::warn!("Original hash ID: {:?}", updated_file.original_hash_id);

    // Notify connected clients about the move/rename (send actual path for menu bar display)
    state.sync_hub.notify_file_changed(&updated_file.path, "move");

    Ok(Json(FileResponse {
        // CRITICAL: Return the Sticky ID (original hash) if it exists.
        // The Client/OS expects the ID to remain constant across a move.
        // If we return the new internal UUID, the OS thinks the item was swapped and errors out.
        id: response_id,
        path: updated_file.path.clone(),
        size_bytes: None, // Simplified response for move operation
        blob_hash: None,
        is_directory: updated_file.path.ends_with('/'),
        is_deleted: updated_file.is_deleted,
        created_at: updated_file.created_at.to_rfc3339(),
        updated_at: updated_file.updated_at.to_rfc3339(),
    }))
}

/// Soft delete a file - keeps blob for history, marks as deleted
async fn delete_file(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<StatusCode, AppError> {
    let user_id = extract_user_id(&state, &headers)?;
    
    // Try to parse as UUID first (for regular files)
    let file_id = if let Ok(uuid) = Uuid::parse_str(&id) {
        uuid
    } else if id.len() == 64 && id.chars().all(|c| c.is_ascii_hexdigit()) {
        // BLAKE3 Hash (Virtual Folder OR Materialized Folder with Sticky ID)

        // 1. Check if we have a real record that "claims" this hash (Sticky ID lookup)
        if let Some(file) = files::get_file_by_original_hash(&state.db, &id).await? {
            file.id
        } else {
            // 2. Fallback to Virtual Resolution (scan paths for virtual folders)
            // Query all folders (paths ending in /) and find one whose hash matches
            let folders: Vec<(Uuid, String)> = sqlx::query_as(
                "SELECT id, path FROM files WHERE path LIKE '%/' AND is_deleted = FALSE"
            )
            .fetch_all(&state.db)
            .await?;

            let matching_folder = folders.into_iter().find(|(_uuid, path)| {
                let hash = blake3::hash(path.as_bytes()).to_hex().to_string();
                hash == id
            });

            match matching_folder {
                Some((uuid, _path)) => uuid,
                None => return Err(AppError::NotFound("Folder not found".into())),
            }
        }
    } else {
        return Err(AppError::BadRequest("Invalid file ID".into()));
    };
    
    // Get file info before deletion for notification
    let file_info = files::get_file_by_id_global(&state.db, file_id)
        .await?
        .ok_or_else(|| AppError::NotFound("File not found".into()))?;

    // Soft delete with ownership check - set is_deleted = true (recursive for directories)
    let deleted = files::soft_delete_recursive_with_owner(&state.db, file_id, user_id).await?;

    if !deleted {
        return Err(AppError::NotFound("File not found or access denied".into()));
    }

    // Notify connected clients about the deletion (send actual path for menu bar display)
    state.sync_hub.notify_file_changed(&file_info.path, "delete");
    
    Ok(StatusCode::NO_CONTENT)
}

async fn download_file(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let _user_id = extract_user_id(&state, &headers)?;

    tracing::info!("Download request for file ID: {}", id);

    let file_id = Uuid::parse_str(&id)
        .map_err(|_| AppError::BadRequest("Invalid file ID".into()))?;

    let file = files::get_file_by_id_global(&state.db, file_id)
        .await?
        .ok_or_else(|| AppError::NotFound("File not found".into()))?;
    
    let version_id = file.current_version_id
        .ok_or_else(|| AppError::NotFound("File has no version".into()))?;
    
    let version = versions::get_version(&state.db, version_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Version not found".into()))?;
    
    // Check if this is a chunked file
    let is_chunked: (bool,) = sqlx::query_as(
        "SELECT COALESCE(is_chunked, FALSE) FROM versions WHERE id = $1"
    )
    .bind(version_id)
    .fetch_one(&state.db)
    .await?;
    
    let content = if is_chunked.0 {
        // Chunked file - reassemble from chunks
        let version_chunks = chunks::get_version_chunks(&state.db, version_id).await?;
        
        // Pre-allocate buffer for efficiency
        let mut reassembled = Vec::with_capacity(version.size_bytes as usize);
        
        // Read and concatenate chunks in order
        for vc in version_chunks {
            let chunk_data = state.blob_store.read(&vc.chunk_hash)?;
            reassembled.extend_from_slice(&chunk_data);
        }
        
        reassembled
    } else {
        // Non-chunked file - read single blob
        let blob_hash = file.blob_hash
            .ok_or_else(|| AppError::NotFound("File has no content".into()))?;
        state.blob_store.read(&blob_hash)?
    };
    
    let filename = std::path::Path::new(&file.path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "download".to_string());

    // Determine MIME type based on file extension
    let content_type = mime_guess::from_path(&file.path)
        .first_or_octet_stream()
        .to_string();

    let content_disposition = format!("attachment; filename=\"{}\"", filename);

    // Safely convert to header values, falling back to defaults if invalid
    let content_type_header = header::HeaderValue::from_str(&content_type)
        .unwrap_or_else(|_| header::HeaderValue::from_static("application/octet-stream"));
    let content_disposition_header = header::HeaderValue::from_str(&content_disposition)
        .unwrap_or_else(|_| header::HeaderValue::from_static("attachment"));

    tracing::debug!("Serving file: {} (size: {} bytes, type: {})", file.path, content.len(), content_type);

    Ok((
        [
            (header::CONTENT_TYPE, content_type_header),
            (header::CONTENT_DISPOSITION, content_disposition_header),
        ],
        content,
    ))
}
