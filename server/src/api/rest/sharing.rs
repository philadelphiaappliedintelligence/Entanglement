//! File sharing routes
//!
//! Handles share link creation, management, and access.

use crate::api::AppState;
use crate::auth;
use crate::db::{chunks, versions, ChunkLocation};
use crate::storage::blob_io;
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, StatusCode},
    routing::{get, post, delete},
    Json, Router,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::error::AppError;

// ============================================================================
// ROUTES
// ============================================================================

pub fn sharing_routes() -> Router<AppState> {
    Router::new()
        // Share link management (authenticated)
        .route("/shares", get(list_shares))
        .route("/shares", post(create_share))
        .route("/shares/:id", get(get_share))
        .route("/shares/:id", delete(revoke_share))
        // Public share access (token-based)
        .route("/share/:token", get(access_share))
        .route("/share/:token/download", get(download_shared_file))
        .route("/share/:token/download-zip", get(download_shared_folder_as_zip))
        .route("/share/:token/contents", get(list_shared_folder_contents))
        .route("/share/:token/download/*path", get(download_shared_file_by_path))
}

// ============================================================================
// TYPES
// ============================================================================

#[derive(Serialize)]
struct ShareResponse {
    id: String,
    file_id: String,
    file_path: String,
    token: String,
    share_url: String,
    can_view: bool,
    can_download: bool,
    can_edit: bool,
    password_protected: bool,
    expires_at: Option<String>,
    max_downloads: Option<i32>,
    download_count: i32,
    is_active: bool,
    created_at: String,
}

#[derive(Deserialize)]
struct CreateShareRequest {
    file_id: String,
    /// Optional password protection
    password: Option<String>,
    /// Permissions
    can_view: Option<bool>,
    can_download: Option<bool>,
    can_edit: Option<bool>,
    /// Expiration in hours from now
    expires_in_hours: Option<i64>,
    /// Maximum number of downloads
    max_downloads: Option<i32>,
}

#[derive(Deserialize)]
struct ListSharesQuery {
    file_id: Option<String>,
    include_expired: Option<bool>,
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Serialize)]
struct ListSharesResponse {
    shares: Vec<ShareResponse>,
    total: i64,
}

#[derive(Serialize)]
struct SharedFileInfo {
    name: String,
    size_bytes: i64,
    is_folder: bool,
    can_download: bool,
    password_required: bool,
}

#[derive(Serialize)]
struct SharedFolderFile {
    name: String,
    path: String,
    size_bytes: Option<i64>,
    is_folder: bool,
    updated_at: Option<String>,
}

#[derive(Serialize)]
struct SharedFolderContentsResponse {
    files: Vec<SharedFolderFile>,
}

#[derive(Deserialize)]
struct AccessShareQuery {
    password: Option<String>,
    /// Subpath within a shared folder for navigation
    path: Option<String>,
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

/// Generate a random share token (URL-safe)
fn generate_share_token() -> String {
    let bytes: [u8; 24] = rand::random();
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, bytes)
}

/// List user's shares
async fn list_shares(
    State(state): State<AppState>,
    Query(query): Query<ListSharesQuery>,
    headers: axum::http::HeaderMap,
) -> Result<Json<ListSharesResponse>, AppError> {
    let user_id = extract_user_id(&state, &headers)?;
    
    let limit = query.limit.unwrap_or(50);
    let offset = query.offset.unwrap_or(0);
    let include_expired = query.include_expired.unwrap_or(false);
    
    let shares = sqlx::query_as::<_, (Uuid, Uuid, String, String, bool, bool, bool, Option<String>, Option<DateTime<Utc>>, Option<i32>, i32, bool, DateTime<Utc>)>(
        r#"
        SELECT s.id, s.file_id, f.path, s.token, s.can_view, s.can_download, s.can_edit,
               s.password_hash, s.expires_at, s.max_downloads, s.download_count, s.is_active, s.created_at
        FROM share_links s
        JOIN files f ON s.file_id = f.id
        WHERE s.created_by = $1
          AND ($2::uuid IS NULL OR s.file_id = $2)
          AND ($3 OR s.is_active = TRUE)
          AND ($3 OR s.expires_at IS NULL OR s.expires_at > NOW())
        ORDER BY s.created_at DESC
        LIMIT $4 OFFSET $5
        "#
    )
    .bind(user_id)
    .bind(query.file_id.as_ref().and_then(|id| Uuid::parse_str(id).ok()))
    .bind(include_expired)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.db)
    .await?;
    
    let web_base_url = std::env::var("PUBLIC_WEB_URL").unwrap_or_else(|_| 
        std::env::var("PUBLIC_URL").unwrap_or_else(|_| "http://localhost:3000".to_string())
    );
    
    let share_responses: Vec<ShareResponse> = shares
        .into_iter()
        .map(|(id, file_id, path, token, can_view, can_download, can_edit, pw_hash, expires_at, max_dl, dl_count, is_active, created_at)| {
            ShareResponse {
                id: id.to_string(),
                file_id: file_id.to_string(),
                file_path: path,
                share_url: format!("{}/share.html#{}", web_base_url, token),
                token,
                can_view,
                can_download,
                can_edit,
                password_protected: pw_hash.is_some(),
                expires_at: expires_at.map(|t| t.to_rfc3339()),
                max_downloads: max_dl,
                download_count: dl_count,
                is_active,
                created_at: created_at.to_rfc3339(),
            }
        })
        .collect();
    
    // Get total count
    let total: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*)
        FROM share_links s
        WHERE s.created_by = $1
          AND ($2::uuid IS NULL OR s.file_id = $2)
          AND ($3 OR s.is_active = TRUE)
        "#
    )
    .bind(user_id)
    .bind(query.file_id.as_ref().and_then(|id| Uuid::parse_str(id).ok()))
    .bind(include_expired)
    .fetch_one(&state.db)
    .await?;
    
    Ok(Json(ListSharesResponse {
        shares: share_responses,
        total: total.0,
    }))
}

/// Create a share link
async fn create_share(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<CreateShareRequest>,
) -> Result<Json<ShareResponse>, AppError> {
    let user_id = extract_user_id(&state, &headers)?;
    
    // Validate file ID
    let file_id = if let Ok(uuid) = Uuid::parse_str(&req.file_id) {
        uuid
    } else if req.file_id.len() == 64 && req.file_id.chars().all(|c| c.is_ascii_hexdigit()) {
        // BLAKE3 hash - could be a materialized folder with sticky ID or a virtual folder
        
        // First try to find by original_hash_id (materialized folder)
        if let Some(file) = sqlx::query_as::<_, (Uuid,)>(
            "SELECT id FROM files WHERE original_hash_id = $1 AND is_deleted = FALSE"
        )
        .bind(&req.file_id)
        .fetch_optional(&state.db)
        .await?
        {
            file.0
        } else {
            // Try to find a virtual folder by resolving the hash
            // Get all paths and find one whose hash matches
            let all_paths: Vec<(Uuid, String)> = sqlx::query_as(
                "SELECT id, path FROM files WHERE is_deleted = FALSE AND (owner_id = $1 OR owner_id IS NULL)"
            )
            .bind(user_id)
            .fetch_all(&state.db)
            .await?;
            
            let mut found_id = None;
            let mut seen_dirs = std::collections::HashSet::new();
            
            for (id, raw_path) in &all_paths {
                let path = if raw_path.starts_with('/') {
                    raw_path.clone()
                } else {
                    format!("/{}", raw_path)
                };
                
                // Check the file/folder itself
                let hash = blake3::hash(path.as_bytes()).to_hex().to_string();
                if hash == req.file_id {
                    found_id = Some(*id);
                    break;
                }
                
                // Check parent directories (for virtual folders)
                for (i, c) in path.chars().enumerate() {
                    if c == '/' && i > 0 {
                        let candidate = &path[0..=i];
                        let clean_candidate = candidate.replace("//", "/");
                        
                        if seen_dirs.contains(&clean_candidate) {
                            continue;
                        }
                        seen_dirs.insert(clean_candidate.clone());
                        
                        let dir_hash = blake3::hash(clean_candidate.as_bytes()).to_hex().to_string();
                        if dir_hash == req.file_id {
                            // Virtual folder - use any file inside it as the anchor
                            found_id = Some(*id);
                            break;
                        }
                    }
                }
                if found_id.is_some() {
                    break;
                }
            }
            
            found_id.ok_or_else(|| AppError::NotFound("File or folder not found".into()))?
        }
    } else {
        return Err(AppError::BadRequest("Invalid file ID".into()));
    };
    
    // Verify file exists and user has access
    let file = sqlx::query_as::<_, (String,)>(
        "SELECT path FROM files WHERE id = $1 AND (owner_id = $2 OR owner_id IS NULL) AND is_deleted = FALSE"
    )
    .bind(file_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("File not found or access denied".into()))?;
    
    let file_path = file.0;
    
    // Generate share token
    let token = generate_share_token();
    
    // Hash password if provided
    let password_hash = if let Some(ref pw) = req.password {
        Some(auth::hash_password(pw)?)
    } else {
        None
    };
    
    // Calculate expiration
    let expires_at = req.expires_in_hours.map(|hours| Utc::now() + Duration::hours(hours));
    
    // Insert share record
    let share_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO share_links (id, file_id, created_by, token, password_hash, 
                                  can_view, can_download, can_edit, expires_at, max_downloads)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        "#
    )
    .bind(share_id)
    .bind(file_id)
    .bind(user_id)
    .bind(&token)
    .bind(&password_hash)
    .bind(req.can_view.unwrap_or(true))
    .bind(req.can_download.unwrap_or(true))
    .bind(req.can_edit.unwrap_or(false))
    .bind(expires_at)
    .bind(req.max_downloads)
    .execute(&state.db)
    .await?;
    
    let web_base_url = std::env::var("PUBLIC_WEB_URL").unwrap_or_else(|_| 
        std::env::var("PUBLIC_URL").unwrap_or_else(|_| "http://localhost:3000".to_string())
    );
    
    Ok(Json(ShareResponse {
        id: share_id.to_string(),
        file_id: file_id.to_string(),
        file_path,
        share_url: format!("{}/share.html#{}", web_base_url, token),
        token,
        can_view: req.can_view.unwrap_or(true),
        can_download: req.can_download.unwrap_or(true),
        can_edit: req.can_edit.unwrap_or(false),
        password_protected: password_hash.is_some(),
        expires_at: expires_at.map(|t| t.to_rfc3339()),
        max_downloads: req.max_downloads,
        download_count: 0,
        is_active: true,
        created_at: Utc::now().to_rfc3339(),
    }))
}

/// Get a specific share
async fn get_share(
    State(state): State<AppState>,
    Path(share_id): Path<Uuid>,
    headers: axum::http::HeaderMap,
) -> Result<Json<ShareResponse>, AppError> {
    let user_id = extract_user_id(&state, &headers)?;
    
    let share = sqlx::query_as::<_, (Uuid, Uuid, String, String, bool, bool, bool, Option<String>, Option<DateTime<Utc>>, Option<i32>, i32, bool, DateTime<Utc>)>(
        r#"
        SELECT s.id, s.file_id, f.path, s.token, s.can_view, s.can_download, s.can_edit,
               s.password_hash, s.expires_at, s.max_downloads, s.download_count, s.is_active, s.created_at
        FROM share_links s
        JOIN files f ON s.file_id = f.id
        WHERE s.id = $1 AND s.created_by = $2
        "#
    )
    .bind(share_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Share not found".into()))?;
    
    let (id, file_id, path, token, can_view, can_download, can_edit, pw_hash, expires_at, max_dl, dl_count, is_active, created_at) = share;
    let web_base_url = std::env::var("PUBLIC_WEB_URL").unwrap_or_else(|_| 
        std::env::var("PUBLIC_URL").unwrap_or_else(|_| "http://localhost:3000".to_string())
    );
    
    Ok(Json(ShareResponse {
        id: id.to_string(),
        file_id: file_id.to_string(),
        file_path: path,
        share_url: format!("{}/share.html#{}", web_base_url, token),
        token,
        can_view,
        can_download,
        can_edit,
        password_protected: pw_hash.is_some(),
        expires_at: expires_at.map(|t| t.to_rfc3339()),
        max_downloads: max_dl,
        download_count: dl_count,
        is_active,
        created_at: created_at.to_rfc3339(),
    }))
}

/// Revoke a share link
async fn revoke_share(
    State(state): State<AppState>,
    Path(share_id): Path<Uuid>,
    headers: axum::http::HeaderMap,
) -> Result<StatusCode, AppError> {
    let user_id = extract_user_id(&state, &headers)?;
    
    let result = sqlx::query(
        "UPDATE share_links SET is_active = FALSE WHERE id = $1 AND created_by = $2"
    )
    .bind(share_id)
    .bind(user_id)
    .execute(&state.db)
    .await?;
    
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Share not found".into()));
    }
    
    Ok(StatusCode::NO_CONTENT)
}

/// Access a shared file (public, token-based)
async fn access_share(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Query(query): Query<AccessShareQuery>,
) -> Result<Json<SharedFileInfo>, AppError> {
    // Look up share by token
    let share = sqlx::query_as::<_, (Uuid, bool, bool, Option<String>, Option<DateTime<Utc>>, Option<i32>, i32, bool)>(
        r#"
        SELECT s.file_id, s.can_view, s.can_download, s.password_hash, 
               s.expires_at, s.max_downloads, s.download_count, s.is_active
        FROM share_links s
        WHERE s.token = $1
        "#
    )
    .bind(&token)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Share link not found".into()))?;
    
    let (file_id, _can_view, can_download, password_hash, expires_at, max_downloads, download_count, is_active) = share;
    
    // Check if share is active
    if !is_active {
        return Err(AppError::BadRequest("This share link has been revoked".into()));
    }
    
    // Check expiration
    if let Some(exp) = expires_at {
        if exp < Utc::now() {
            return Err(AppError::BadRequest("This share link has expired".into()));
        }
    }
    
    // Check download limit
    if let Some(max) = max_downloads {
        if download_count >= max {
            return Err(AppError::BadRequest("Download limit reached for this share link".into()));
        }
    }
    
    // Check password
    if let Some(ref pw_hash) = password_hash {
        let provided_password = query.password
            .ok_or_else(|| AppError::Unauthorized("Password required".into()))?;
        
        if !auth::verify_password(&provided_password, pw_hash)? {
            return Err(AppError::Unauthorized("Invalid password".into()));
        }
    }
    
    // Get file info
    let file = sqlx::query_as::<_, (String, Option<i64>)>(
        r#"
        SELECT f.path, v.size_bytes
        FROM files f
        LEFT JOIN versions v ON f.current_version_id = v.id
        WHERE f.id = $1 AND f.is_deleted = FALSE
        "#
    )
    .bind(file_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Shared file not found".into()))?;
    
    let (path, size) = file;
    let name = std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "Shared File".to_string());
    
    Ok(Json(SharedFileInfo {
        name,
        size_bytes: size.unwrap_or(0),
        is_folder: path.ends_with('/'),
        can_download,
        password_required: password_hash.is_some(),
    }))
}

/// Download a shared file
async fn download_shared_file(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Query(query): Query<AccessShareQuery>,
) -> Result<axum::response::Response, AppError> {
    // Look up share by token
    let share = sqlx::query_as::<_, (Uuid, bool, Option<String>, Option<DateTime<Utc>>, Option<i32>, i32, bool)>(
        r#"
        SELECT s.file_id, s.can_download, s.password_hash, 
               s.expires_at, s.max_downloads, s.download_count, s.is_active
        FROM share_links s
        WHERE s.token = $1
        "#
    )
    .bind(&token)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Share link not found".into()))?;
    
    let (file_id, can_download, password_hash, expires_at, max_downloads, download_count, is_active) = share;
    
    // Validate share access
    if !is_active {
        return Err(AppError::BadRequest("This share link has been revoked".into()));
    }
    
    if !can_download {
        return Err(AppError::Unauthorized("Download not allowed for this share".into()));
    }
    
    if let Some(exp) = expires_at {
        if exp < Utc::now() {
            return Err(AppError::BadRequest("This share link has expired".into()));
        }
    }
    
    if let Some(max) = max_downloads {
        if download_count >= max {
            return Err(AppError::BadRequest("Download limit reached".into()));
        }
    }
    
    // Check password
    if let Some(ref pw_hash) = password_hash {
        let provided_password = query.password
            .ok_or_else(|| AppError::Unauthorized("Password required".into()))?;
        
        if !auth::verify_password(&provided_password, pw_hash)? {
            return Err(AppError::Unauthorized("Invalid password".into()));
        }
    }
    
    // Get file and version info
    let file = sqlx::query_as::<_, (String, Option<Uuid>)>(
        r#"
        SELECT f.path, f.current_version_id
        FROM files f
        WHERE f.id = $1 AND f.is_deleted = FALSE
        "#
    )
    .bind(file_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("File not found".into()))?;
    
    let (path, version_id) = file;
    let version_id = version_id.ok_or_else(|| AppError::NotFound("File has no version".into()))?;
    
    // Get version details
    let version = versions::get_version_ext(&state.db, version_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Version not found".into()))?;
    
    // Increment download counter
    sqlx::query("UPDATE share_links SET download_count = download_count + 1, last_accessed_at = NOW() WHERE token = $1")
        .bind(&token)
        .execute(&state.db)
        .await?;
    
    // Build response headers
    let filename = std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "download".to_string());
    
    // Sanitize filename for header
    let safe_filename: String = filename
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '.' || *c == '_' || *c == '-')
        .collect();
    let safe_filename = if safe_filename.is_empty() { "download".to_string() } else { safe_filename };
    
    let content_type = mime_guess::from_path(&path)
        .first_or_octet_stream()
        .to_string();
    
    // Stream content based on storage type
    if version.is_chunked {
        // Chunked file - stream from container storage
        let chunk_list = chunks::get_version_chunks_with_location(&state.db, version.id).await?;
        
        if chunk_list.is_empty() && version.size_bytes > 0 {
            return Err(AppError::NotFound("Version has no chunks".into()));
        }
        
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
        
        Ok(response)
    } else {
        // Legacy/Unchunked file - serve the single blob
        let blob_hash = version.content_hash();
        
        let content = state.blob_manager.read_legacy_blob(blob_hash)?;
        
        let body = Body::from(content);
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
        
        Ok(response)
    }
}


/// List contents of a shared folder
async fn list_shared_folder_contents(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Query(query): Query<AccessShareQuery>,
) -> Result<Json<SharedFolderContentsResponse>, AppError> {
    // Look up share by token
    let share = sqlx::query_as::<_, (Uuid, bool, Option<String>, Option<DateTime<Utc>>, Option<i32>, i32, bool)>(
        r#"
        SELECT s.file_id, s.can_view, s.password_hash, 
               s.expires_at, s.max_downloads, s.download_count, s.is_active
        FROM share_links s
        WHERE s.token = $1
        "#
    )
    .bind(&token)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Share link not found".into()))?;
    
    let (file_id, _can_view, password_hash, expires_at, max_downloads, download_count, is_active) = share;
    
    // Validate share access
    if !is_active {
        return Err(AppError::BadRequest("This share link has been revoked".into()));
    }
    
    if let Some(exp) = expires_at {
        if exp < Utc::now() {
            return Err(AppError::BadRequest("This share link has expired".into()));
        }
    }
    
    if let Some(max) = max_downloads {
        if download_count >= max {
            return Err(AppError::BadRequest("Download limit reached".into()));
        }
    }
    
    // Check password
    if let Some(ref pw_hash) = password_hash {
        let provided_password = query.password
            .ok_or_else(|| AppError::Unauthorized("Password required".into()))?;
        
        if !auth::verify_password(&provided_password, pw_hash)? {
            return Err(AppError::Unauthorized("Invalid password".into()));
        }
    }
    
    // Get the shared folder path (root of the share)
    let folder = sqlx::query_as::<_, (String,)>(
        "SELECT path FROM files WHERE id = $1 AND is_deleted = FALSE"
    )
    .bind(file_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Shared folder not found".into()))?;
    
    let root_folder_path = folder.0;
    
    // Ensure it's a folder
    if !root_folder_path.ends_with('/') {
        return Err(AppError::BadRequest("This share is not a folder".into()));
    }
    
    // Calculate the target path (root + optional subpath)
    let target_path = if let Some(ref subpath) = query.path {
        // Sanitize subpath to prevent path traversal
        let clean_subpath = subpath
            .trim_start_matches('/')
            .trim_end_matches('/');
        
        // Check for path traversal attempts
        if clean_subpath.contains("..") {
            return Err(AppError::BadRequest("Invalid path".into()));
        }
        
        if clean_subpath.is_empty() {
            root_folder_path.clone()
        } else {
            format!("{}{}/", root_folder_path, clean_subpath)
        }
    } else {
        root_folder_path.clone()
    };
    
    // Verify target path is within the shared folder
    if !target_path.starts_with(&root_folder_path) {
        return Err(AppError::BadRequest("Invalid path".into()));
    }
    
    // List files within this folder (direct children only)
    let files = sqlx::query_as::<_, (String, Option<i64>, DateTime<Utc>)>(
        r#"
        SELECT f.path, v.size_bytes, f.updated_at
        FROM files f
        LEFT JOIN versions v ON f.current_version_id = v.id
        WHERE f.path LIKE $1 || '%'
          AND f.path != $1
          AND f.is_deleted = FALSE
        ORDER BY f.path
        "#
    )
    .bind(&target_path)
    .fetch_all(&state.db)
    .await?;
    
    // Filter to only direct children and build response
    let mut result_files = Vec::new();
    let mut seen_dirs = std::collections::HashSet::new();
    
    for (path, size, updated_at) in files {
        // Get the relative path from the target folder
        let relative_path = path.strip_prefix(&target_path).unwrap_or(&path);
        
        // Check if this is a direct child or nested
        if let Some(slash_pos) = relative_path.find('/') {
            // This is a nested item - extract the direct child folder name
            let dir_name = &relative_path[..slash_pos];
            if !seen_dirs.contains(dir_name) {
                seen_dirs.insert(dir_name.to_string());
                result_files.push(SharedFolderFile {
                    name: dir_name.to_string(),
                    path: format!("{}{}/", target_path, dir_name),
                    size_bytes: None,
                    is_folder: true,
                    updated_at: Some(updated_at.to_rfc3339()),
                });
            }
        } else {
            // Direct child file
            result_files.push(SharedFolderFile {
                name: relative_path.to_string(),
                path: path.clone(),
                size_bytes: size,
                is_folder: false,
                updated_at: Some(updated_at.to_rfc3339()),
            });
        }
    }
    
    Ok(Json(SharedFolderContentsResponse { files: result_files }))
}

/// Download a file from within a shared folder by path
async fn download_shared_file_by_path(
    State(state): State<AppState>,
    Path((token, file_path)): Path<(String, String)>,
    Query(query): Query<AccessShareQuery>,
) -> Result<axum::response::Response, AppError> {
    // Look up share by token
    let share = sqlx::query_as::<_, (Uuid, bool, Option<String>, Option<DateTime<Utc>>, Option<i32>, i32, bool)>(
        r#"
        SELECT s.file_id, s.can_download, s.password_hash, 
               s.expires_at, s.max_downloads, s.download_count, s.is_active
        FROM share_links s
        WHERE s.token = $1
        "#
    )
    .bind(&token)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Share link not found".into()))?;
    
    let (folder_id, can_download, password_hash, expires_at, max_downloads, download_count, is_active) = share;
    
    // Validate share access
    if !is_active {
        return Err(AppError::BadRequest("This share link has been revoked".into()));
    }
    
    if !can_download {
        return Err(AppError::Unauthorized("Download not allowed for this share".into()));
    }
    
    if let Some(exp) = expires_at {
        if exp < Utc::now() {
            return Err(AppError::BadRequest("This share link has expired".into()));
        }
    }
    
    if let Some(max) = max_downloads {
        if download_count >= max {
            return Err(AppError::BadRequest("Download limit reached".into()));
        }
    }
    
    // Check password
    if let Some(ref pw_hash) = password_hash {
        let provided_password = query.password
            .ok_or_else(|| AppError::Unauthorized("Password required".into()))?;
        
        if !auth::verify_password(&provided_password, pw_hash)? {
            return Err(AppError::Unauthorized("Invalid password".into()));
        }
    }
    
    // Get the shared folder path
    let folder = sqlx::query_as::<_, (String,)>(
        "SELECT path FROM files WHERE id = $1 AND is_deleted = FALSE"
    )
    .bind(folder_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Shared folder not found".into()))?;
    
    let folder_path = folder.0;
    
    // Sanitize the subpath to prevent path traversal
    let clean_subpath = file_path
        .trim_start_matches('/')
        .trim_end_matches('/');
    
    // Check for path traversal attempts
    if clean_subpath.contains("..") {
        return Err(AppError::BadRequest("Invalid path".into()));
    }
    
    // Construct the full path
    let full_path = format!("{}{}", folder_path, clean_subpath);
    
    tracing::debug!("Download request: folder_path={}, clean_subpath={}, full_path={}", folder_path, clean_subpath, full_path);
    
    // Security: Ensure the requested path is within the shared folder
    if !full_path.starts_with(&folder_path) {
        return Err(AppError::Unauthorized("Access denied: path outside shared folder".into()));
    }
    
    // Get file and version info
    let file = sqlx::query_as::<_, (Uuid, String, Option<Uuid>)>(
        r#"
        SELECT f.id, f.path, f.current_version_id
        FROM files f
        WHERE f.path = $1 AND f.is_deleted = FALSE
        "#
    )
    .bind(&full_path)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| {
        tracing::warn!("File not found for path: {}", full_path);
        AppError::NotFound(format!("File not found: {}", clean_subpath))
    })?;
    
    let (_file_id, path, version_id) = file;
    let version_id = version_id.ok_or_else(|| AppError::NotFound("File has no version".into()))?;
    
    // Get version details
    let version = versions::get_version_ext(&state.db, version_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Version not found".into()))?;
    
    // Increment download counter
    sqlx::query("UPDATE share_links SET download_count = download_count + 1, last_accessed_at = NOW() WHERE token = $1")
        .bind(&token)
        .execute(&state.db)
        .await?;
    
    // Build response headers
    let filename = std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "download".to_string());
    
    // Sanitize filename for header
    let safe_filename: String = filename
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '.' || *c == '_' || *c == '-')
        .collect();
    let safe_filename = if safe_filename.is_empty() { "download".to_string() } else { safe_filename };
    
    let content_type = mime_guess::from_path(&path)
        .first_or_octet_stream()
        .to_string();
    
    // Stream content based on storage type
    if version.is_chunked {
        // Chunked file - stream from container storage
        let chunk_list = chunks::get_version_chunks_with_location(&state.db, version.id).await?;
        
        if chunk_list.is_empty() && version.size_bytes > 0 {
            return Err(AppError::NotFound("Version has no chunks".into()));
        }
        
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
        
        Ok(response)
    } else {
        // Legacy/Unchunked file - serve the single blob
        let blob_hash = version.content_hash();
        
        let content = state.blob_manager.read_legacy_blob(blob_hash)?;
        
        let body = Body::from(content);
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
        
        Ok(response)
    }
}

/// Download an entire shared folder as a ZIP archive
/// GET /share/:token/download-zip
async fn download_shared_folder_as_zip(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Query(query): Query<AccessShareQuery>,
) -> Result<axum::response::Response, AppError> {
    // 1. Look up share by token
    let share = sqlx::query_as::<_, (Uuid, bool, Option<String>, Option<DateTime<Utc>>, Option<i32>, i32, bool)>(
        r#"
        SELECT s.file_id, s.can_download, s.password_hash, 
               s.expires_at, s.max_downloads, s.download_count, s.is_active
        FROM share_links s
        WHERE s.token = $1
        "#
    )
    .bind(&token)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Share link not found".into()))?;
    
    let (file_id, can_download, password_hash, expires_at, max_downloads, download_count, is_active) = share;
    
    // 2. Validate share access
    if !is_active {
        return Err(AppError::BadRequest("This share link has been revoked".into()));
    }
    
    if !can_download {
        return Err(AppError::Unauthorized("Download not allowed for this share".into()));
    }
    
    if let Some(exp) = expires_at {
        if exp < Utc::now() {
            return Err(AppError::BadRequest("This share link has expired".into()));
        }
    }
    
    if let Some(max) = max_downloads {
        if download_count >= max {
            return Err(AppError::BadRequest("Download limit reached".into()));
        }
    }
    
    // Check password
    if let Some(ref pw_hash) = password_hash {
        let provided_password = query.password
            .ok_or_else(|| AppError::Unauthorized("Password required".into()))?;
        
        if !auth::verify_password(&provided_password, pw_hash)? {
            return Err(AppError::Unauthorized("Invalid password".into()));
        }
    }
    
    // 3. Get file info
    let file = sqlx::query_as::<_, (String,)>(
        r#"
        SELECT f.path
        FROM files f
        WHERE f.id = $1 AND f.is_deleted = FALSE
        "#
    )
    .bind(file_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("File not found".into()))?;
    
    let folder_path = file.0;
    
    // Must be a folder
    if !folder_path.ends_with('/') {
        return Err(AppError::BadRequest("This share is not a folder".into()));
    }
    
    // 4. Get folder name for zip filename
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
    
    // 5. Get all files under this folder (no owner check for public share)
    let all_files: Vec<crate::db::files::File> = sqlx::query_as(
        r#"
        SELECT id, path, current_version_id, is_deleted, created_at, updated_at, owner_id, original_hash_id
        FROM files
        WHERE path LIKE $1 AND is_deleted = FALSE
        ORDER BY path
        "#
    )
    .bind(format!("{}%", folder_path))
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Internal(format!("Failed to list files: {}", e)))?;
    
    if all_files.is_empty() {
        return Err(AppError::NotFound("No files found in folder".into()));
    }
    
    tracing::info!("Creating shared ZIP archive for {} with {} files", folder_path, all_files.len());
    
    // 5. Build the ZIP in memory
    let mut zip_buffer = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut zip_buffer);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        
        for f in &all_files {
            // Skip folders (virtual)
            if f.path.ends_with('/') {
                continue;
            }
            
            // Get version for this file
            let version_id = match f.current_version_id {
                Some(id) => id,
                None => continue,
            };
            
            let version = match versions::get_version_ext(&state.db, version_id).await? {
                Some(v) => v,
                None => continue,
            };
            
            // Calculate relative path within the zip
            let relative_path = f.path.strip_prefix(&folder_path).unwrap_or(&f.path);
            
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
                                    tracing::warn!("Failed to read chunk for {}: {}", f.path, e);
                                    continue;
                                }
                            }
                        },
                        ChunkLocation::Standalone { hash } => {
                            match state.blob_manager.read_legacy_blob(&hash) {
                                Ok(data) => file_data.extend(data),
                                Err(e) => {
                                    tracing::warn!("Failed to read legacy chunk for {}: {}", f.path, e);
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
                        tracing::warn!("Failed to read blob for {}: {}", f.path, e);
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
    
    tracing::info!("Shared ZIP archive created: {} bytes", zip_size);
    
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
