//! Chunk-based routes (CDC for Delta Sync)
//!
//! Handles chunk upload, download, existence check, and chunked file creation.

use crate::api::AppState;
use crate::db::{chunks, files, versions, ChunkTier};
use crate::storage::store_chunk;
use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use blake3;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

use super::error::{extract_user_id, validate_path, AppError};

// ============================================================================
// TYPES
// ============================================================================

/// Request to check which chunks already exist on the server
#[derive(Deserialize)]
pub struct CheckChunksRequest {
    pub hashes: Vec<String>,
}

/// Response indicating which chunks exist
#[derive(Serialize)]
pub struct CheckChunksResponse {
    pub existing: Vec<String>,
    pub missing: Vec<String>,
}

/// Request to create a file from chunks
#[derive(Deserialize)]
pub struct CreateChunkedFileRequest {
    pub path: String,
    pub file_hash: String,
    pub size_bytes: i64,
    /// Ordered list of chunk hashes
    pub chunks: Vec<ChunkInfo>,
    /// Original filesystem creation time (ISO8601)
    pub created_at: Option<String>,
    /// Original filesystem modification time (ISO8601)
    pub updated_at: Option<String>,
}

#[derive(Deserialize)]
pub struct ChunkInfo {
    pub hash: String,
    pub size: i32,
    pub offset: i64,
}

/// Response after creating chunked file
#[derive(Serialize)]
pub struct CreateChunkedFileResponse {
    pub id: String,
    pub version_id: String,
    pub path: String,
    pub chunk_count: usize,
}

/// Get chunk manifest for a file (for delta sync)
#[derive(Serialize)]
pub struct FileChunksResponse {
    pub file_id: String,
    pub version_id: String,
    pub is_chunked: bool,
    pub file_hash: String,
    pub size_bytes: i64,
    pub chunks: Vec<ChunkInfoResponse>,
}

#[derive(Serialize)]
pub struct ChunkInfoResponse {
    pub hash: String,
    pub size: i32,
    pub offset: i64,
    pub index: i32,
}

// ============================================================================
// HANDLERS
// ============================================================================

/// Check which chunks already exist (for delta sync)
/// Client sends list of chunk hashes, server responds with which ones it has
pub async fn check_chunks(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<CheckChunksRequest>,
) -> Result<Json<CheckChunksResponse>, AppError> {
    let _user_id = extract_user_id(&state, &headers)?;
    
    let existing = chunks::get_existing_chunks(&state.db, &req.hashes).await?;
    let existing_set: HashSet<&String> = existing.iter().collect();
    
    let missing: Vec<String> = req.hashes.iter()
        .filter(|h| !existing_set.contains(h))
        .cloned()
        .collect();
    
    Ok(Json(CheckChunksResponse { existing, missing }))
}

/// Upload a single chunk
/// PUT /chunks/{hash} with raw binary body
/// 
/// Optional header X-Chunk-Tier: 0-4 to specify compression tier
pub async fn upload_chunk(
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
            "Chunk hash mismatch: expected {}, got {}",
            hash, computed_hash
        )));
    }
    
    // Check if chunk already exists
    if chunks::chunk_exists(&state.db, &hash).await? {
        // Chunk already exists - idempotent success
        return Ok(StatusCode::OK);
    }
    
    // Get tier from header, default to Standard (2)
    let tier = headers
        .get("X-Chunk-Tier")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i16>().ok())
        .and_then(ChunkTier::from_i16)
        .unwrap_or(ChunkTier::Standard);
    
    // Store chunk using BlobManager (with compression for tiers 0-2)
    store_chunk(&state.blob_manager, &state.db, &hash, &body, tier)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to store chunk: {}", e)))?;
    
    // Chunk upload logging - trace level to avoid log spam
    tracing::trace!("Chunk uploaded: {} ({} bytes)", hash.get(..8).unwrap_or(&hash), body.len());
    
    Ok(StatusCode::CREATED)
}

/// Download a single chunk
/// GET /chunks/{hash}
pub async fn download_chunk(
    State(state): State<AppState>,
    Path(hash): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let _user_id = extract_user_id(&state, &headers)?;
    
    // First, try to get chunk info from database to find its location
    if let Some(chunk) = chunks::get_chunk_with_location(&state.db, &hash).await? {
        // Check if chunk is stored in a container
        if let (Some(container_id), Some(offset), Some(length)) = 
            (chunk.container_id, chunk.offset_bytes, chunk.length_bytes) 
        {
            // Determine if compressed: if length < size_bytes, it was compressed
            let is_compressed = length < chunk.size_bytes;
            
            // Read from container using BlobManager
            use crate::storage::blob_io::ChunkLocation;
            let location = ChunkLocation {
                container_id,
                offset: offset as u64,
                length: length as u32,
                compressed: is_compressed,
            };
            
            let content = state.blob_manager.read_chunk(&location)
                .await
                .map_err(|e| AppError::Internal(format!("Failed to read chunk: {}", e)))?;
            
            return Ok((
                [(header::CONTENT_TYPE, header::HeaderValue::from_static("application/octet-stream"))],
                content,
            ));
        }
    }
    
    // Fallback: Try legacy blob store
    let content = state.blob_manager.read_legacy_blob(&hash)?;
    
    Ok((
        [(header::CONTENT_TYPE, header::HeaderValue::from_static("application/octet-stream"))],
        content,
    ))
}

/// Create a file from chunks (chunked upload complete)
pub async fn create_chunked_file(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<CreateChunkedFileRequest>,
) -> Result<Json<CreateChunkedFileResponse>, AppError> {
    let user_id = extract_user_id(&state, &headers)?;
    
    // SECURITY: Validate path to prevent path traversal
    validate_path(&req.path)?;
    
    // Get unique chunk hashes (file may have duplicate chunks for repeating content)
    let unique_hashes: HashSet<String> = req.chunks.iter().map(|c| c.hash.clone()).collect();
    let chunk_hashes: Vec<String> = unique_hashes.into_iter().collect();
    
    // Verify all unique chunks exist
    let existing = chunks::get_existing_chunks(&state.db, &chunk_hashes).await?;
    let existing_set: HashSet<String> = existing.into_iter().collect();
    
    let missing: Vec<String> = chunk_hashes.iter()
        .filter(|h| !existing_set.contains(*h))
        .cloned()
        .collect();
    
    if !missing.is_empty() {
        return Err(AppError::BadRequest(format!(
            "Missing chunks: {:?}",
            missing
        )));
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
    
    // Upsert file record with owner and client-provided dates
    let file = files::upsert_file_with_owner_and_dates(&state.db, &req.path, user_id, created_at, updated_at).await?;
    
    // Create version with chunks
    let chunk_tuples: Vec<(String, i32, i64)> = req.chunks.iter()
        .map(|c| (c.hash.clone(), c.size, c.offset))
        .collect();
    
    let version_id = chunks::create_chunked_version(
        &state.db,
        file.id,
        &req.file_hash,
        req.size_bytes,
        &chunk_tuples,
    ).await?;
    
    Ok(Json(CreateChunkedFileResponse {
        id: file.id.to_string(),
        version_id: version_id.to_string(),
        path: req.path,
        chunk_count: req.chunks.len(),
    }))
}

/// Get chunk manifest for current version of a file
pub async fn get_file_chunks(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<Json<FileChunksResponse>, AppError> {
    let _user_id = extract_user_id(&state, &headers)?;

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
    
    // Check if this version uses chunks
    let is_chunked: (bool,) = sqlx::query_as(
        "SELECT COALESCE(is_chunked, FALSE) FROM versions WHERE id = $1"
    )
    .bind(version_id)
    .fetch_one(&state.db)
    .await?;
    
    if !is_chunked.0 {
        // Non-chunked file - return single "chunk" representing entire file
        return Ok(Json(FileChunksResponse {
            file_id: file_id.to_string(),
            version_id: version_id.to_string(),
            is_chunked: false,
            file_hash: version.blob_hash.clone(),
            size_bytes: version.size_bytes,
            chunks: vec![ChunkInfoResponse {
                hash: version.blob_hash,
                size: version.size_bytes as i32,
                offset: 0,
                index: 0,
            }],
        }));
    }
    
    // Get chunk manifest
    let version_chunks = chunks::get_version_chunks(&state.db, version_id).await?;
    
    let chunk_infos: Vec<ChunkInfoResponse> = version_chunks.iter()
        .map(|vc| {
            ChunkInfoResponse {
                hash: vc.chunk_hash.clone(),
                size: 0,  // Will be filled from chunks table
                offset: vc.chunk_offset,
                index: vc.chunk_index,
            }
        })
        .collect();
    
    // Get chunk sizes
    let mut chunk_infos_with_size = Vec::new();
    for mut ci in chunk_infos {
        if let Some(chunk) = chunks::get_chunk(&state.db, &ci.hash).await? {
            ci.size = chunk.size_bytes;
        }
        chunk_infos_with_size.push(ci);
    }
    
    Ok(Json(FileChunksResponse {
        file_id: file_id.to_string(),
        version_id: version_id.to_string(),
        is_chunked: true,
        file_hash: version.blob_hash,
        size_bytes: version.size_bytes,
        chunks: chunk_infos_with_size,
    }))
}
