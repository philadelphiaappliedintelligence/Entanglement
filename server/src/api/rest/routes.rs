use crate::api::AppState;
use crate::auth;
use crate::db::{files, users, versions};
use crate::storage::blob::BlobError;
use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};

use serde::{Deserialize, Serialize};
use blake3;
use uuid::Uuid;

/// Get the parent directory path for a file path
/// e.g., "/documents/file.txt" -> "/documents/"
/// e.g., "/file.txt" -> "/"
fn get_parent_path(path: &str) -> String {
    if path.is_empty() || path == "/" {
        return "/".to_string();
    }
    
    // Remove trailing slash for processing
    let clean_path = path.trim_end_matches('/');
    
    if let Some(pos) = clean_path.rfind('/') {
        if pos == 0 {
            "/".to_string()
        } else {
            format!("{}/", &clean_path[..pos])
        }
    } else {
        "/".to_string()
    }
}

// Auth routes

pub fn auth_routes() -> Router<AppState> {
    Router::new()
        .route("/auth/register", post(register))
        .route("/auth/login", post(login))
        .route("/auth/refresh", post(refresh_token))
}

#[derive(Deserialize)]
struct RegisterRequest {
    email: String,
    password: String,
}

#[derive(Serialize)]
struct AuthResponse {
    token: String,
    refresh_token: String,
    user_id: String,
    /// Token expiration time in seconds (24 hours)
    expires_in: i64,
}

async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    let password_hash = auth::hash_password(&req.password)?;
    let user = users::create_user(&state.db, &req.email, &password_hash).await?;

    let token = auth::create_access_token(&state.config.jwt_secret, user.id)?;
    let refresh_token = auth::create_refresh_token(&state.config.jwt_secret, user.id)?;

    Ok(Json(AuthResponse {
        token,
        refresh_token,
        user_id: user.id.to_string(),
        expires_in: 24 * 60 * 60, // 24 hours in seconds
    }))
}

#[derive(Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
}

async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    tracing::info!("Login attempt for email: {}", req.email);
    
    let user = match users::get_user_by_email(&state.db, &req.email).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            tracing::warn!("User not found: {}", req.email);
            return Err(AppError::Unauthorized("Invalid credentials".into()));
        }
        Err(e) => {
            tracing::error!("Database error during login: {}", e);
            return Err(AppError::Internal("Database error".into()));
        }
    };

    match auth::verify_password(&req.password, &user.password_hash) {
        Ok(true) => {}
        Ok(false) => {
            tracing::warn!("Invalid password for user: {}", req.email);
            return Err(AppError::Unauthorized("Invalid credentials".into()));
        }
        Err(e) => {
            tracing::error!("Password verification error: {}", e);
            return Err(AppError::Internal("Authentication error".into()));
        }
    }

    let token = match auth::create_access_token(&state.config.jwt_secret, user.id) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Token creation error: {}", e);
            return Err(AppError::Internal("Token generation failed".into()));
        }
    };
    
    let refresh_token = match auth::create_refresh_token(&state.config.jwt_secret, user.id) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Refresh token creation error: {}", e);
            return Err(AppError::Internal("Token generation failed".into()));
        }
    };

    tracing::info!("Login successful for user: {}", user.id);
    
    Ok(Json(AuthResponse {
        token,
        refresh_token,
        user_id: user.id.to_string(),
        expires_in: 24 * 60 * 60, // 24 hours in seconds
    }))
}

#[derive(Deserialize)]
struct RefreshRequest {
    refresh_token: String,
}

/// Refresh an access token using a refresh token
async fn refresh_token(
    State(state): State<AppState>,
    Json(req): Json<RefreshRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    // Verify the refresh token
    let user_id = auth::verify_refresh_token(&state.config.jwt_secret, &req.refresh_token)
        .map_err(|_| AppError::Unauthorized("Invalid or expired refresh token".into()))?;

    // Create new tokens
    let token = auth::create_access_token(&state.config.jwt_secret, user_id)?;
    let new_refresh_token = auth::create_refresh_token(&state.config.jwt_secret, user_id)?;

    Ok(Json(AuthResponse {
        token,
        refresh_token: new_refresh_token,
        user_id: user_id.to_string(),
        expires_in: 24 * 60 * 60, // 24 hours in seconds
    }))
}

// File routes

pub fn file_routes() -> Router<AppState> {
    Router::new()
        .route("/files", get(list_files))
        .route("/files", post(upload_file))
        .route("/files/:id", get(get_file))
        .route("/files/:id", axum::routing::patch(update_file))
        .route("/files/:id", axum::routing::delete(delete_file))
        .route("/files/:id/download", get(download_file))
        .route("/files/:id/versions", get(list_file_versions))
        .route("/files/:id/restore/:version_id", post(restore_version))
        // Raw binary blob upload - most efficient
        .route("/blobs/:hash", axum::routing::put(upload_blob))
        .route("/blobs/:hash", get(download_blob))
        // Chunk-based upload/download (CDC for delta sync)
        .route("/chunks/check", post(check_chunks))
        .route("/chunks/:hash", axum::routing::put(upload_chunk))
        .route("/chunks/:hash", get(download_chunk))
        .route("/files/chunked", post(create_chunked_file))
        .route("/files/:id/chunks", get(get_file_chunks))
}

/// V1 API routes with container-based chunk storage
/// These are the preferred endpoints for new clients
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
        // File download - stream file content from chunks (must be before :id)
        .route("/v1/files/:version_id/download", get(download_v1_file))
        // File metadata lookup by ID
        .route("/v1/files/:id", get(get_file_metadata_v1))
        // WebSocket sync notifications
        .route("/ws/sync", get(crate::api::ws::ws_handler))
}

#[derive(Deserialize)]
struct ListFilesQuery {
    prefix: Option<String>,
    include_deleted: Option<bool>,
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Serialize)]
struct FileResponse {
    id: String,
    path: String,
    size_bytes: Option<i64>,
    blob_hash: Option<String>,
    is_directory: bool,
    is_deleted: bool,
    created_at: String,
    updated_at: String,
}

#[derive(Serialize)]
struct ListFilesResponse {
    files: Vec<FileResponse>,
    total: i64,
}

// =============================================================================
// Virtual Directory Listing
// =============================================================================

#[derive(Serialize)]
struct DirectoryEntryResponse {
    id: String,
    name: String,
    path: String,
    is_folder: bool,
    size_bytes: i64,
    updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    version_id: Option<String>,
}

#[derive(Serialize)]
struct ListDirectoryResponse {
    entries: Vec<DirectoryEntryResponse>,
    path: String,
}

#[derive(Deserialize)]
struct ListDirectoryQuery {
    #[serde(default)]
    path: String,
}

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

// =============================================================================
// Changed Since API (Incremental Sync)
// =============================================================================

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

// =============================================================================
// Directory Creation (V1 API)
// =============================================================================

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

// Upload file endpoint - accepts JSON with path and base64 content
#[derive(Deserialize)]
struct UploadRequest {
    path: String,
    content: String,  // base64 encoded
}

#[derive(Serialize)]
struct UploadResponse {
    id: String,
    path: String,
    blob_hash: String,
    size_bytes: i64,
}

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

// Raw binary blob upload - most efficient method
// Client computes hash, uploads raw bytes to PUT /blobs/{hash}
async fn upload_blob(
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

// Download raw blob by hash
async fn download_blob(
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

// Create/update file metadata after blob is uploaded
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

pub fn metadata_routes() -> Router<AppState> {
    Router::new()
        .route("/metadata", post(create_file_metadata))
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

// Soft delete a file - keeps blob for history, marks as deleted
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

async fn download_file(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let _user_id = extract_user_id(&state, &headers)?;

    tracing::info!("Download request for file ID: {}", id);

    use crate::db::{chunks, versions};

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

#[derive(Serialize)]
struct VersionResponse {
    id: String,
    blob_hash: String,
    size_bytes: i64,
    created_at: String,
    created_by: String,
}

#[derive(Serialize)]
struct ListVersionsResponse {
    versions: Vec<VersionResponse>,
    total: i64,
}

#[derive(Deserialize)]
struct ListVersionsQuery {
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Deserialize)]
struct UpdateFileRequest {
    path: String,
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
    tracing::info!("ID type: {}", if id.len() == 64 && id.chars().all(|c| c.is_ascii_hexdigit()) { "HASH" } else if let Ok(_) = Uuid::parse_str(&id) { "UUID" } else { "OTHER" });

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

async fn list_file_versions(
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

#[derive(Serialize)]
struct RestoreResponse {
    success: bool,
    new_version_id: String,
}

async fn restore_version(
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

// Admin routes

pub fn admin_routes() -> Router<AppState> {
    Router::new()
        .route("/admin/stats", get(get_stats))
        .route("/server/info", get(get_server_info))
}

#[derive(Serialize)]
struct ServerInfo {
    name: String,
    version: String,
    grpc_port: u16,
}

async fn get_server_info(State(state): State<AppState>) -> Json<ServerInfo> {
    Json(ServerInfo {
        name: state.config.server_name.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        grpc_port: state.config.grpc_port,
    })
}

#[derive(Serialize)]
struct StatsResponse {
    total_users: i64,
    total_files: i64,
    total_versions: i64,
    total_blob_bytes: i64,
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

// Helper functions

fn extract_user_id(state: &AppState, headers: &axum::http::HeaderMap) -> Result<Uuid, AppError> {
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized("Missing authorization header".into()))?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| AppError::Unauthorized("Invalid authorization format".into()))?;

    let user_id = auth::verify_token(&state.config.jwt_secret, token)?;
    Ok(user_id)
}

// Error handling

#[derive(Debug)]
enum AppError {
    BadRequest(String),
    Unauthorized(String),
    NotFound(String),
    Internal(String),
}

// ============================================================================
// PATH VALIDATION
// ============================================================================

/// Validate a file path to prevent path traversal attacks
/// Returns an error if the path contains dangerous sequences
fn validate_path(path: &str) -> Result<(), AppError> {
    // Check for path traversal sequences
    if path.contains("..") {
        return Err(AppError::BadRequest("Path contains invalid sequence '..'".into()));
    }
    
    // Check for null bytes (could truncate path in C-based systems)
    if path.contains('\0') {
        return Err(AppError::BadRequest("Path contains invalid null byte".into()));
    }
    
    // Check for backslashes (potential Windows path injection)
    if path.contains('\\') {
        return Err(AppError::BadRequest("Path contains invalid backslash".into()));
    }
    
    // Check for control characters
    if path.chars().any(|c| c.is_control() && c != '\t') {
        return Err(AppError::BadRequest("Path contains invalid control characters".into()));
    }
    
    // Ensure path doesn't start with multiple slashes (could be protocol-relative URL)
    if path.starts_with("//") {
        return Err(AppError::BadRequest("Path cannot start with double slash".into()));
    }
    
    Ok(())
}

// ============================================================================
// CHUNK-BASED ENDPOINTS (CDC for Delta Sync)
// ============================================================================

/// Request to check which chunks already exist on the server
#[derive(Deserialize)]
struct CheckChunksRequest {
    hashes: Vec<String>,
}

/// Response indicating which chunks exist
#[derive(Serialize)]
struct CheckChunksResponse {
    existing: Vec<String>,
    missing: Vec<String>,
}

// ============================================================================
// V1 FILE MANIFEST ENDPOINT
// ============================================================================

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
    
    use crate::db::{chunks, files, ChunkTier};
    
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

    // Log for debugging - use debug level to avoid production verbosity
    tracing::debug!(
        "Created file version for path: {}",
        req.path
    );

    // 10. Notify connected clients about the new file (send actual path for menu bar display)
    state.sync_hub.notify_file_changed(&req.path, "create");

    Ok((StatusCode::CREATED, Json(response)).into_response())
}

// ============================================================================
// V1 FILE DOWNLOAD ENDPOINT
// ============================================================================

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
    
    use crate::db::{chunks, files, versions, ChunkLocation};
    use crate::storage::blob_io;
    use axum::body::Body;
    
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
        let blob_store = state.blob_store.clone();
        
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
                        match blob_store.read(&hash) {
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
        
        if !state.blob_store.exists(blob_hash)? {
             return Err(AppError::NotFound("Blob not found".into()));
        }
        
        let blob_store = state.blob_store.clone();
        let hash = blob_hash.to_string();
        
        let stream = async_stream::stream! {
            match blob_store.read(&hash) {
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

/// Check which chunks already exist (for delta sync)
/// Client sends list of chunk hashes, server responds with which ones it has
async fn check_chunks(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<CheckChunksRequest>,
) -> Result<Json<CheckChunksResponse>, AppError> {
    let _user_id = extract_user_id(&state, &headers)?;
    
    use crate::db::chunks;
    
    let existing = chunks::get_existing_chunks(&state.db, &req.hashes).await?;
    let existing_set: std::collections::HashSet<&String> = existing.iter().collect();
    
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
async fn upload_chunk(
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
    use crate::db::chunks;
    if chunks::chunk_exists(&state.db, &hash).await? {
        // Chunk already exists - idempotent success
        return Ok(StatusCode::OK);
    }
    
    // Get tier from header, default to Standard (2)
    use crate::db::ChunkTier;
    let tier = headers
        .get("X-Chunk-Tier")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i16>().ok())
        .and_then(ChunkTier::from_i16)
        .unwrap_or(ChunkTier::Standard);
    
    // Store chunk using BlobManager (with compression for tiers 0-2)
    use crate::storage::store_chunk;
    store_chunk(&state.blob_manager, &state.db, &hash, &body, tier)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to store chunk: {}", e)))?;
    
    // Chunk upload logging - trace level to avoid log spam
    tracing::trace!("Chunk uploaded: {} ({} bytes)", hash.get(..8).unwrap_or(&hash), body.len());
    
    Ok(StatusCode::CREATED)
}

/// Download a single chunk
/// GET /chunks/{hash}
async fn download_chunk(
    State(state): State<AppState>,
    Path(hash): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let _user_id = extract_user_id(&state, &headers)?;
    
    // First, try to get chunk info from database to find its location
    use crate::db::chunks;
    
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
    let content = state.blob_store.read(&hash)?;
    
    Ok((
        [(header::CONTENT_TYPE, header::HeaderValue::from_static("application/octet-stream"))],
        content,
    ))
}

/// Request to create a file from chunks
#[derive(Deserialize)]
struct CreateChunkedFileRequest {
    path: String,
    file_hash: String,
    size_bytes: i64,
    /// Ordered list of chunk hashes
    chunks: Vec<ChunkInfo>,
    /// Original filesystem creation time (ISO8601)
    created_at: Option<String>,
    /// Original filesystem modification time (ISO8601)
    updated_at: Option<String>,
}

#[derive(Deserialize)]
struct ChunkInfo {
    hash: String,
    size: i32,
    offset: i64,
}

/// Response after creating chunked file
#[derive(Serialize)]
struct CreateChunkedFileResponse {
    id: String,
    version_id: String,
    path: String,
    chunk_count: usize,
}

/// Create a file from chunks (chunked upload complete)
async fn create_chunked_file(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<CreateChunkedFileRequest>,
) -> Result<Json<CreateChunkedFileResponse>, AppError> {
    let user_id = extract_user_id(&state, &headers)?;
    
    // SECURITY: Validate path to prevent path traversal
    validate_path(&req.path)?;
    
    use crate::db::{chunks, files};
    use std::collections::HashSet;
    
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

/// Get chunk manifest for a file (for delta sync)
#[derive(Serialize)]
struct FileChunksResponse {
    file_id: String,
    version_id: String,
    is_chunked: bool,
    file_hash: String,
    size_bytes: i64,
    chunks: Vec<ChunkInfoResponse>,
}

#[derive(Serialize)]
struct ChunkInfoResponse {
    hash: String,
    size: i32,
    offset: i64,
    index: i32,
}

/// Get chunk manifest for current version of a file
async fn get_file_chunks(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<Json<FileChunksResponse>, AppError> {
    let _user_id = extract_user_id(&state, &headers)?;

    use crate::db::{chunks, files, versions};

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

// ============================================================================
// ERROR HANDLING
// ============================================================================

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match self {
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            AppError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
        };

        let body = serde_json::json!({ "error": message });
        (status, Json(body)).into_response()
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        // SECURITY: Log the full error server-side but return generic message to client
        tracing::error!("Internal error: {}", err);
        AppError::Internal("An internal error occurred".to_string())
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        // SECURITY: Log the full database error server-side but return generic message to client
        // This prevents leaking database schema/query information
        tracing::error!("Database error: {}", err);
        AppError::Internal("An internal error occurred".to_string())
    }
}

impl From<BlobError> for AppError {
    fn from(err: BlobError) -> Self {
        tracing::error!("Storage error: {}", err);
        match err {
            BlobError::NotFound(hash) => {
                // Blob not found is a 404, not 500 - helps client understand what's missing
                AppError::NotFound(format!("Blob not found: {}", hash))
            }
            BlobError::InvalidHash(hash) => {
                AppError::BadRequest(format!("Invalid blob hash: {}", hash))
            }
            BlobError::Io(_) => {
                // SECURITY: Don't expose IO details to client
                AppError::Internal("Storage error".to_string())
            }
        }
    }
}

