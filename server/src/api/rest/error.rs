//! Error handling for REST API
//!
//! Provides the `AppError` type used across all REST endpoints and helper functions.

use crate::api::AppState;
use crate::auth;
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
            AppError::Internal(msg) => {
                // SECURITY: Log full details server-side, return generic message to client
                tracing::error!(details = %msg, "Internal server error");
                (StatusCode::INTERNAL_SERVER_ERROR, "An internal error occurred".to_string())
            }
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

/// Validate and normalize a file path to prevent path traversal and injection attacks.
/// Returns the normalized path on success, or an error if the path is invalid.
pub fn validate_path(path: &str) -> Result<String, AppError> {
    // 1. Reject empty paths
    if path.is_empty() {
        return Err(AppError::BadRequest("Path cannot be empty".into()));
    }

    // 2. Reject null bytes (could truncate path in C-based systems)
    if path.contains('\0') {
        return Err(AppError::BadRequest("Path contains invalid null byte".into()));
    }

    // 3. Decode percent-encoding before validation to prevent bypass via %2e%2e
    let decoded = percent_decode(path);

    // 4. Normalize: collapse duplicate slashes, resolve `.` components
    let mut normalized = String::with_capacity(decoded.len());
    let mut prev_was_slash = false;
    for segment in decoded.split('/') {
        if segment.is_empty() {
            if !prev_was_slash {
                normalized.push('/');
                prev_was_slash = true;
            }
            continue;
        }
        if segment == "." {
            // Skip current-directory references
            continue;
        }
        if segment == ".." {
            return Err(AppError::BadRequest("Path contains invalid traversal sequence '..'".into()));
        }
        if prev_was_slash {
            // Already have a slash from previous iteration
        } else {
            normalized.push('/');
        }
        normalized.push_str(segment);
        prev_was_slash = false;
    }
    // Preserve trailing slash if original had one (directory indicator)
    if decoded.ends_with('/') && !normalized.ends_with('/') {
        normalized.push('/');
    }
    if normalized.is_empty() {
        normalized.push('/');
    }

    // 5. Ensure path starts with /
    if !normalized.starts_with('/') {
        normalized.insert(0, '/');
    }

    // 6. Reject backslashes (Windows path injection)
    if normalized.contains('\\') {
        return Err(AppError::BadRequest("Path contains invalid backslash".into()));
    }

    // 7. Reject control characters
    if normalized.chars().any(|c| c.is_control()) {
        return Err(AppError::BadRequest("Path contains invalid control characters".into()));
    }

    // 8. Whitelist valid characters: alphanumeric, /, ., -, _, space
    if !normalized.chars().all(|c| {
        c.is_alphanumeric() || matches!(c, '/' | '.' | '-' | '_' | ' ')
    }) {
        return Err(AppError::BadRequest("Path contains invalid characters".into()));
    }

    Ok(normalized)
}

/// Simple percent-decoding for path validation.
/// Decodes %XX sequences to their byte values.
fn percent_decode(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (
                hex_val(bytes[i + 1]),
                hex_val(bytes[i + 2]),
            ) {
                result.push((hi << 4 | lo) as char);
                i += 3;
                continue;
            }
        }
        result.push(bytes[i] as char);
        i += 1;
    }
    result
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_paths() {
        assert!(validate_path("/foo").is_ok());
        assert!(validate_path("/foo/bar.txt").is_ok());
        assert!(validate_path("/foo bar/baz.txt").is_ok());
    }

    #[test]
    fn test_rejects_empty() {
        assert!(validate_path("").is_err());
    }

    #[test]
    fn test_rejects_traversal() {
        assert!(validate_path("/../etc/passwd").is_err());
        assert!(validate_path("/foo/../bar").is_err());
        assert!(validate_path("/foo/%2e%2e/bar").is_err());
    }

    #[test]
    fn test_rejects_null_bytes() {
        assert!(validate_path("/foo\0bar").is_err());
    }

    #[test]
    fn test_normalizes_slashes() {
        let result = validate_path("//foo///bar").unwrap();
        assert_eq!(result, "/foo/bar");
    }

    #[test]
    fn test_rejects_invalid_chars() {
        assert!(validate_path("/foo<bar").is_err());
        assert!(validate_path("/foo>bar").is_err());
        assert!(validate_path("/foo|bar").is_err());
    }

    #[test]
    fn test_rejects_backslash() {
        assert!(validate_path("/foo\\bar").is_err());
    }
}

/// Get the parent directory path for a file path
/// e.g., "/documents/file.txt" -> "/documents/"
/// e.g., "/file.txt" -> "/"
#[allow(dead_code)]
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
