//! Error handling for REST API
//!
//! Provides the `AppError` type used across all REST endpoints and helper functions.

use crate::api::AppState;
use crate::auth;
use crate::storage::blob::BlobError;
use axum::{
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use uuid::Uuid;

// ============================================================================
// ERROR TYPES
// ============================================================================

#[derive(Debug)]
pub enum AppError {
    BadRequest(String),
    Unauthorized(String),
    NotFound(String),
    Internal(String),
}

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

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Extract user ID from Authorization header
pub fn extract_user_id(state: &AppState, headers: &axum::http::HeaderMap) -> Result<Uuid, AppError> {
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

// ============================================================================
// PATH VALIDATION
// ============================================================================

/// Validate a file path to prevent path traversal attacks
/// Returns an error if the path contains dangerous sequences
pub fn validate_path(path: &str) -> Result<(), AppError> {
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

/// Get the parent directory path for a file path
/// e.g., "/documents/file.txt" -> "/documents/"
/// e.g., "/file.txt" -> "/"
pub fn get_parent_path(path: &str) -> String {
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
