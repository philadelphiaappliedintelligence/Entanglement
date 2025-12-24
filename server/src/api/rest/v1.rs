//! V1 API routes
//!
//! Preferred endpoints for new clients using container-based chunk storage.

use crate::api::AppState;
use crate::db::{chunks, files, versions, ChunkLocation, ChunkTier};
use crate::storage::blob_io;
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::error::{extract_user_id, validate_path, AppError};
use super::types::{DirectoryEntryResponse, ListDirectoryQuery, ListDirectoryResponse};
use super::chunks::{check_chunks, upload_chunk, download_chunk};

// ============================================================================
// ROUTES
// ============================================================================

pub fn v1_routes() -> Router<AppState> {
    Router::new()
        // Chunk deduplication check
        .route("/v1/chunks/check", post(check_chunks))
        // Chunk upload/download with container storage
        .route("/v1/chunks/:hash", axum::routing::put(upload_chunk))
        .route("/v1/chunks/:hash", get(download_chunk))
        // File manifest - finalize upload by linking chunks to a file path
        .route("/v1/files", post(create_v1_file))
        // Directory creation - creates a virtual folder (path ending in /)
        .route("/v1/files/directory", post(create_directory_v1))
        // Directory listing with virtual folders (must be before :id to avoid conflicts)
        .route("/v1/files/list", get(list_directory_v1))
        // Changed since - incremental sync (must be before :id to avoid conflicts)
        .route("/v1/files/changes", get(get_file_changes))
        // Folder download as ZIP
        .route("/v1/files/download-zip", get(download_folder_as_zip))
        // File download - stream file content from chunks (must be before :id)
        .route("/v1/files/:version_id/download", get(download_v1_file))
        // File metadata lookup by ID
        .route("/v1/files/:id", get(get_file_metadata_v1))
        // WebSocket sync notifications
        .route("/ws/sync", get(crate::api::ws::ws_handler))
}

// ============================================================================
// TYPES
// ============================================================================

/// Response for file metadata lookup (V1 API)
#[derive(Serialize)]
struct FileMetadataResponse {
    id: String,
    current_version_id: Option<String>,
    name: String,
    path: String,
    size_bytes: i64,
    updated_at: String,
}

#[derive(Deserialize)]
struct ChangesQuery {
    /// ISO8601 datetime - return files changed after this time
    since: Option<String>,
    /// Max number of changes to return (default 1000)
    limit: Option<i64>,
}

#[derive(Serialize)]
struct ChangesResponse {
    /// List of changed files
    changes: Vec<FileChangeResponse>,
    /// Current server time (use for next sync)
    server_time: String,
}

#[derive(Serialize)]
struct FileChangeResponse {
    id: String,
    path: String,
    /// "created", "modified", or "deleted"
    action: String,
    size_bytes: Option<i64>,
    blob_hash: Option<String>,
    is_directory: bool,
    updated_at: String,
}

/// Request to create a directory
#[derive(Deserialize)]
struct CreateDirectoryRequest {
    /// Directory path (will be normalized to end with /)
    path: String,
}

/// Response for directory creation
#[derive(Serialize)]
struct CreateDirectoryResponse {
    id: String,
    path: String,
    is_directory: bool,
    is_deleted: bool,
    size_bytes: i64,
    blob_hash: Option<String>,
    created_at: String,
    updated_at: String,
}

/// Request to create a file version from uploaded chunks
#[derive(Deserialize)]
struct V1CreateFileRequest {
    /// Virtual file path (e.g., "documents/contract.pdf")
    path: String,
    /// Total file size in bytes
    size_bytes: i64,
    /// File modification time (ISO8601)
    modified_at: String,
    /// Chunking tier used (0-4)
    tier_id: i16,
    /// BLAKE3 hash of the complete file content
    content_hash: String,
    /// Ordered list of chunk hashes that compose the file
    chunk_hashes: Vec<String>,
}

/// Response after successfully creating a file version
#[derive(Serialize)]
struct V1CreateFileResponse {
    id: String,
    version_id: String,
    path: String,
}

/// Error response when chunks are missing
#[derive(Serialize)]
struct MissingChunksError {
    error: String,
    missing_hashes: Vec<String>,
}

// ============================================================================
// HANDLERS
// ============================================================================

/// Get file metadata by ID (V1 API)
/// GET /v1/files/:id
///
/// Returns file metadata including current_version_id for download resolution.
async fn get_file_metadata_v1(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: axum::http::HeaderMap,
) -> Result<Json<FileMetadataResponse>, AppError> {
    let _user_id = extract_user_id(&state, &headers)?;

    let file = files::get_file_by_id_global(&state.db, id)
        .await?
        .ok_or_else(|| AppError::NotFound("File not found".into()))?;
    
    // Extract filename from path
    let name = std::path::Path::new(&file.path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    
    Ok(Json(FileMetadataResponse {
        id: file.id.to_string(),
        current_version_id: file.current_version_id.map(|v| v.to_string()),
        name,
        path: file.path,
        size_bytes: file.size_bytes.unwrap_or(0),
        updated_at: file.updated_at.to_rfc3339(),
    }))
}

/// List directory contents with virtual folder support
///
/// GET /v1/files/list?path=documents/
///
/// Returns direct children (files) and virtual folders (subdirectories)
async fn list_directory_v1(
    State(state): State<AppState>,
    Query(query): Query<ListDirectoryQuery>,
    headers: axum::http::HeaderMap,
) -> Result<Json<ListDirectoryResponse>, AppError> {
    let _user_id = extract_user_id(&state, &headers)?;
    
    // Normalize path: strip leading slash, keep trailing slash if present
    let normalized_path = query.path.trim_start_matches('/').to_string();
    
    let entries = files::list_directory(&state.db, &normalized_path).await?;
    
    let response_entries: Vec<DirectoryEntryResponse> = entries
        .into_iter()
        .map(|e| DirectoryEntryResponse {
            id: e.id,
            name: e.name,
            path: e.path,
            is_folder: e.is_folder,
            size_bytes: e.size_bytes,
            updated_at: e.updated_at.to_rfc3339(),
            version_id: e.version_id.map(|v| v.to_string()),
        })
        .collect();
    
    // Return the normalized path (what was actually queried)
    let response_path = if normalized_path.is_empty() {
        String::new()
    } else if normalized_path.ends_with('/') {
        normalized_path
    } else {
        format!("{}/", normalized_path)
    };
    
    Ok(Json(ListDirectoryResponse {
        entries: response_entries,
        path: response_path,
    }))
}

/// Get files changed since a timestamp (for incremental sync)
/// 
/// GET /v1/files/changes?since=2024-12-22T00:00:00Z&limit=1000
///
/// Returns files created, modified, or deleted since the given timestamp.
/// If `since` is omitted, returns all files (useful for first sync).
async fn get_file_changes(
    State(state): State<AppState>,
    Query(query): Query<ChangesQuery>,
    headers: axum::http::HeaderMap,
) -> Result<Json<ChangesResponse>, AppError> {
    let user_id = extract_user_id(&state, &headers)?;
    
    // Parse the since timestamp if provided
    let cursor = if let Some(since_str) = &query.since {
        Some(
            chrono::DateTime::parse_from_rfc3339(since_str)
                .map_err(|e| AppError::BadRequest(format!("Invalid since timestamp: {}", e)))?
                .with_timezone(&chrono::Utc)
        )
    } else {
        None
    };
    
    let limit = query.limit.unwrap_or(1000).min(10000); // Cap at 10k
    
    // Get changes from database
    let changes = files::get_changes(&state.db, user_id, cursor, limit).await?;
    
    // Convert to response format
    let response_changes: Vec<FileChangeResponse> = changes
        .into_iter()
        .map(|change| {
            // Determine action based on state
            let action = if change.is_deleted {
                "deleted"
            } else if cursor.is_some() && change.created_at > cursor.unwrap() {
                "created"
            } else {
                "modified"
            };
            
            FileChangeResponse {
                id: change.id.to_string(),
                path: change.path.clone(),
                action: action.to_string(),
                size_bytes: change.size_bytes,
                blob_hash: change.blob_hash,
                is_directory: change.path.ends_with('/'),
                updated_at: change.updated_at.to_rfc3339(),
            }
        })
        .collect();
    
    // Return current server time for use in next sync
    let server_time = chrono::Utc::now().to_rfc3339();
    
    Ok(Json(ChangesResponse {
        changes: response_changes,
        server_time,
    }))
}

/// Create a directory (virtual folder)
/// POST /v1/files/directory
/// 
/// Creates a file record with a path ending in "/" (virtual directory convention)
async fn create_directory_v1(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<CreateDirectoryRequest>,
) -> Result<Json<CreateDirectoryResponse>, AppError> {
    let user_id = extract_user_id(&state, &headers)?;
    
    // Normalize path: ensure it ends with /
    let mut dir_path = req.path.trim().to_string();
    if dir_path.is_empty() {
        return Err(AppError::BadRequest("Path cannot be empty".into()));
    }
    
    // SECURITY: Validate path to prevent path traversal
    validate_path(&dir_path)?;
    
    // Ensure leading slash
    if !dir_path.starts_with('/') {
        dir_path = format!("/{}", dir_path);
    }
    
    // Ensure trailing slash (directory convention)
    if !dir_path.ends_with('/') {
        dir_path.push('/');
    }
    
    // Create directory record (upsert) with ownership
    let file = files::upsert_file_with_owner(&state.db, &dir_path, user_id).await?;
    
    tracing::debug!("Created directory: {}", dir_path);
    
    // Notify connected clients about the new directory (send actual path for menu bar display)
    state.sync_hub.notify_file_changed(&dir_path, "create");
    
    Ok(Json(CreateDirectoryResponse {
        id: file.id.to_string(),
        path: dir_path,
        is_directory: true,
        is_deleted: file.is_deleted,
        size_bytes: 0,
        blob_hash: None,
        created_at: file.created_at.to_rfc3339(),
        updated_at: file.updated_at.to_rfc3339(),
    }))
}

/// Create a file version from previously uploaded chunks
/// POST /v1/files
/// 
/// This endpoint finalizes a chunked upload by:
/// 1. Validating that all chunks exist in the database
/// 2. Creating a version record that links the chunks to a file path
/// 3. Setting the file's current version
async fn create_v1_file(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<V1CreateFileRequest>,
) -> Result<axum::response::Response, AppError> {
    let user_id = extract_user_id(&state, &headers)?;
    
    // 1. Validate path is not empty
    if req.path.trim().is_empty() {
        return Err(AppError::BadRequest("Path cannot be empty".into()));
    }
    
    // SECURITY: Validate path to prevent path traversal
    validate_path(&req.path)?;
    
    // 2. Integrity check - ALL chunks must exist in the database
    let missing = chunks::find_missing_chunks(&state.db, &req.chunk_hashes).await?;
    if !missing.is_empty() {
        let body = MissingChunksError {
            error: "Missing chunks".into(),
            missing_hashes: missing,
        };
        return Ok((StatusCode::BAD_REQUEST, Json(body)).into_response());
    }
    
    // 3. Get chunk sizes from DB to calculate offsets
    let chunk_sizes = chunks::get_chunk_sizes(&state.db, &req.chunk_hashes).await?;
    
    // 4. Build chunk info list with calculated offsets
    let mut chunk_infos: Vec<chunks::ChunkInfo> = Vec::with_capacity(req.chunk_hashes.len());
    let mut current_offset: i64 = 0;
    
    for hash in &req.chunk_hashes {
        let size = chunk_sizes.get(hash)
            .copied()
            .ok_or_else(|| AppError::Internal(format!("Chunk size not found for {}", hash)))?;
        
        chunk_infos.push(chunks::ChunkInfo {
            hash: hash.clone(),
            size_bytes: size,
            offset_in_file: current_offset,
        });
        
        current_offset += size as i64;
    }
    
    // 5. Validate total size matches
    if current_offset != req.size_bytes {
        return Err(AppError::BadRequest(format!(
            "Size mismatch: chunks total {} bytes, but size_bytes is {}",
            current_offset, req.size_bytes
        )));
    }
    
    // 6. Parse modified_at timestamp
    let modified_at = chrono::DateTime::parse_from_rfc3339(&req.modified_at)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .ok();
    
    // 7. Upsert file record with owner (creates if not exists, updates timestamp if exists)
    let file = files::upsert_file_with_owner_and_dates(&state.db, &req.path, user_id, None, modified_at).await?;
    
    // 8. Create version with tier (transactional - links chunks and updates file)
    let tier = ChunkTier::from_i16(req.tier_id).unwrap_or_default();
    let version_id = chunks::create_version_with_tier(
        &state.db,
        file.id,
        &req.content_hash,
        req.size_bytes,
        tier,
        &chunk_infos,
    ).await?;
    
    tracing::debug!(
        "Created file version for path '{}' ({} chunks, {} bytes)",
        req.path, req.chunk_hashes.len(), req.size_bytes
    );

    // 9. Return 201 Created
    let response = V1CreateFileResponse {
        id: file.id.to_string(),
        version_id: version_id.to_string(),
        path: req.path.clone(),
    };

    // 10. Notify connected clients about the new file (send actual path for menu bar display)
    state.sync_hub.notify_file_changed(&req.path, "create");

    Ok((StatusCode::CREATED, Json(response)).into_response())
}

/// Download a file version by streaming its chunks
/// GET /v1/files/:version_id/download
///
/// Returns a streaming response that reconstructs the file from its chunks.
/// Memory-safe: only one chunk is in memory at a time.
async fn download_v1_file(
    State(state): State<AppState>,
    Path(version_id): Path<Uuid>,
    headers: axum::http::HeaderMap,
) -> Result<axum::response::Response, AppError> {
    let _user_id = extract_user_id(&state, &headers)?;
    
    // 1. Try to resolve as version first
    let (version, file_path) = match versions::get_version_ext(&state.db, version_id).await? {
        Some(v) => {
            // It's a version ID, get the associated file
            let f = files::get_file_by_version_id(&state.db, version_id)
                .await?
                .ok_or_else(|| AppError::NotFound("File not found for version".into()))?;
            (v, f.path)
        }
        None => {
            // Fallback: Try to resolve as file ID
            let f = files::get_file_by_id_global(&state.db, version_id)
                .await?
                .ok_or_else(|| AppError::NotFound("File/Version not found".into()))?;
            
            let current_version_id = f.current_version_id
                .ok_or_else(|| AppError::NotFound("File has no current version".into()))?;
                
            let v = versions::get_version_ext(&state.db, current_version_id)
                .await?
                .ok_or_else(|| AppError::NotFound("Current version not found".into()))?;
                
            (v, f.path)
        }
    };
    
    // 5. Extract filename from path for Content-Disposition
    let filename = std::path::Path::new(&file_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "download".to_string());
    
    // Sanitize filename for header (remove problematic characters)
    let safe_filename: String = filename
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '.' || *c == '_' || *c == '-')
        .collect();
    let safe_filename = if safe_filename.is_empty() { "download".to_string() } else { safe_filename };
    
    // 6. Determine MIME type based on file extension
    let content_type = mime_guess::from_path(&file_path)
        .first_or_octet_stream()
        .to_string();

    tracing::debug!(
        "Streaming download for version {} ({} bytes)",
        version_id, version.size_bytes
    );

    // 7. Determine stream source and return response
    if version.is_chunked {
        // Chunked file - get manifest
        let chunk_list = chunks::get_version_chunks_with_location(&state.db, version.id).await?;
        
        if chunk_list.is_empty() && version.size_bytes > 0 {
            return Err(AppError::NotFound("Version has no chunks".into()));
        }
        
        // Create async stream that yields chunk data in order
        let blob_manager = state.blob_manager.clone();
        
        let stream = async_stream::stream! {
            for (_vc, chunk) in chunk_list {
                match chunk.location() {
                     ChunkLocation::Container { container_id, offset, length } => {
                        let is_compressed = length < chunk.size_bytes;
                        let location = blob_io::ChunkLocation {
                            container_id,
                            offset: offset as u64,
                            length: length as u32,
                            compressed: is_compressed,
                        };
                        match blob_manager.read_chunk(&location).await {
                            Ok(data) => yield Ok::<_, std::io::Error>(axum::body::Bytes::from(data)),
                            Err(e) => {
                                tracing::error!("Failed to read chunk from container: {}", e);
                                yield Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()));
                                return;
                            }
                        }
                    },
                    ChunkLocation::Standalone { hash } => {
                        match blob_manager.read_legacy_blob(&hash) {
                            Ok(data) => yield Ok::<_, std::io::Error>(axum::body::Bytes::from(data)),
                            Err(e) => {
                                tracing::error!("Failed to read standalone chunk {}: {}", hash, e);
                                yield Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()));
                                return;
                            }
                        }
                    }
                }
            }
        };

        let body = Body::from_stream(stream);
        let response = axum::response::Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, &content_type[..])
            .header(header::CONTENT_LENGTH, version.size_bytes.to_string())
            .header(
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", safe_filename),
            )
            .body(body)
            .map_err(|e| AppError::Internal(format!("Failed to build response: {}", e)))?;
        
        return Ok(response);

    } else {
        // Legacy/Unchunked file - serve the single blob
        let blob_hash = version.content_hash(); // Use content hash
        
        if !state.blob_manager.legacy_exists(blob_hash)? {
             return Err(AppError::NotFound("Blob not found".into()));
        }
        
        let blob_manager = state.blob_manager.clone();
        let hash = blob_hash.to_string();
        
        let stream = async_stream::stream! {
            match blob_manager.read_legacy_blob(&hash) {
                Ok(bytes) => {
                     yield Ok::<_, std::io::Error>(axum::body::Bytes::from(bytes));
                },
                Err(e) => {
                    tracing::error!("Failed to read blob {}: {}", hash, e);
                    yield Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()));
                    return;
                }
            }
        };

        let body = Body::from_stream(stream);
        let response = axum::response::Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, &content_type[..])
            .header(header::CONTENT_LENGTH, version.size_bytes.to_string())
            .header(
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", safe_filename),
            )
            .body(body)
            .map_err(|e| AppError::Internal(format!("Failed to build response: {}", e)))?;
        
        return Ok(response);
    }
}

/// Query parameters for folder zip download
#[derive(Deserialize)]
struct DownloadZipQuery {
    path: String,
}

/// Download a folder as a ZIP archive
/// GET /v1/files/download-zip?path=documents/
///
/// Creates a ZIP archive containing all files in the folder and streams it.
async fn download_folder_as_zip(
    State(state): State<AppState>,
    Query(query): Query<DownloadZipQuery>,
    headers: axum::http::HeaderMap,
) -> Result<axum::response::Response, AppError> {
    let user_id = extract_user_id(&state, &headers)?;
    
    // Normalize folder path
    let folder_path = if query.path.ends_with('/') {
        query.path.clone()
    } else {
        format!("{}/", query.path)
    };
    
    // Validate path
    validate_path(&folder_path)?;
    
    // Get folder name for zip filename
    let folder_name = folder_path
        .trim_end_matches('/')
        .split('/')
        .last()
        .unwrap_or("download");
    
    let safe_folder_name: String = folder_name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    let zip_filename = if safe_folder_name.is_empty() { 
        "archive.zip".to_string() 
    } else { 
        format!("{}.zip", safe_folder_name) 
    };
    
    // Get all files under this folder (including nested folders)
    let all_files = files::list_files_by_user_under_path(&state.db, user_id, &folder_path).await?;
    
    if all_files.is_empty() {
        return Err(AppError::NotFound("No files found in folder".into()));
    }
    
    tracing::info!("Creating ZIP archive for {} with {} files", folder_path, all_files.len());
    
    // Build the ZIP in memory (for simplicity - could be optimized for very large folders)
    // For very large folders, we'd want to stream directly but zip crate doesn't support async
    let mut zip_buffer = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut zip_buffer);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        
        for file in &all_files {
            // Skip folders (they're virtual)
            if file.path.ends_with('/') {
                continue;
            }
            
            // Get version for this file
            let version_id = match file.current_version_id {
                Some(id) => id,
                None => continue, // Skip files without version
            };
            
            let version = match versions::get_version_ext(&state.db, version_id).await? {
                Some(v) => v,
                None => continue,
            };
            
            // Calculate relative path within the zip
            let relative_path = file.path.strip_prefix(&folder_path).unwrap_or(&file.path);
            
            // Read file content
            let content = if version.is_chunked {
                let chunk_list = chunks::get_version_chunks_with_location(&state.db, version.id).await?;
                let mut file_data = Vec::with_capacity(version.size_bytes as usize);
                
                for (_vc, chunk) in chunk_list {
                    match chunk.location() {
                        ChunkLocation::Container { container_id, offset, length } => {
                            let is_compressed = length < chunk.size_bytes;
                            let location = blob_io::ChunkLocation {
                                container_id,
                                offset: offset as u64,
                                length: length as u32,
                                compressed: is_compressed,
                            };
                            match state.blob_manager.read_chunk(&location).await {
                                Ok(data) => file_data.extend(data),
                                Err(e) => {
                                    tracing::warn!("Failed to read chunk for {}: {}", file.path, e);
                                    continue;
                                }
                            }
                        },
                        ChunkLocation::Standalone { hash } => {
                            match state.blob_manager.read_legacy_blob(&hash) {
                                Ok(data) => file_data.extend(data),
                                Err(e) => {
                                    tracing::warn!("Failed to read legacy chunk for {}: {}", file.path, e);
                                    continue;
                                }
                            }
                        }
                    }
                }
                file_data
            } else {
                // Legacy blob
                let blob_hash = version.content_hash();
                match state.blob_manager.read_legacy_blob(blob_hash) {
                    Ok(data) => data,
                    Err(e) => {
                        tracing::warn!("Failed to read blob for {}: {}", file.path, e);
                        continue;
                    }
                }
            };
            
            // Add file to zip
            if let Err(e) = zip.start_file(relative_path, options) {
                tracing::warn!("Failed to start zip entry for {}: {}", relative_path, e);
                continue;
            }
            if let Err(e) = std::io::Write::write_all(&mut zip, &content) {
                tracing::warn!("Failed to write zip entry for {}: {}", relative_path, e);
                continue;
            }
        }
        
        zip.finish().map_err(|e| AppError::Internal(format!("Failed to finalize zip: {}", e)))?;
    }
    
    let zip_data = zip_buffer.into_inner();
    let zip_size = zip_data.len();
    
    tracing::info!("ZIP archive created: {} bytes", zip_size);
    
    let body = Body::from(zip_data);
    let response = axum::response::Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/zip")
        .header(header::CONTENT_LENGTH, zip_size.to_string())
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", zip_filename),
        )
        .body(body)
        .map_err(|e| AppError::Internal(format!("Failed to build response: {}", e)))?;
    
    Ok(response)
}

