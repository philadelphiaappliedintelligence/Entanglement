//! Authentication routes
//!
//! Handles user registration, login, token refresh, and admin user management.

use crate::api::AppState;
use crate::auth;
use crate::db::users;
use axum::{
    extract::{Path, State},
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::error::AppError;

// ============================================================================
// ROUTES
// ============================================================================

pub fn auth_routes() -> Router<AppState> {
    Router::new()
        // Public auth routes
        .route("/auth/login", post(login))
        .route("/auth/refresh", post(refresh_token))
        // Admin routes (require admin auth)
        .route("/admin/users", get(list_users))
        .route("/admin/users", post(create_user))
        .route("/admin/users/:id", delete(delete_user))
        .route("/admin/users/:id/password", put(reset_user_password))
        .route("/admin/users/:id/admin", put(toggle_admin))
        // Current user info
        .route("/auth/me", get(get_current_user))
}

// ============================================================================
// TYPES
// ============================================================================

#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Serialize)]
struct AuthResponse {
    token: String,
    refresh_token: String,
    user_id: String,
    username: String,
    is_admin: bool,
    /// Token expiration time in seconds (24 hours)
    expires_in: i64,
}

#[derive(Deserialize)]
struct RefreshRequest {
    refresh_token: String,
}

#[derive(Deserialize)]
struct CreateUserRequest {
    username: String,
    password: String,
    is_admin: Option<bool>,
}

#[derive(Serialize)]
struct UserResponse {
    id: String,
    username: String,
    is_admin: bool,
    created_at: String,
}

#[derive(Deserialize)]
struct ResetPasswordRequest {
    new_password: String,
}

#[derive(Deserialize)]
struct SetAdminRequest {
    is_admin: bool,
}

#[derive(Serialize)]
struct MessageResponse {
    message: String,
}

// ============================================================================
// HANDLERS - Public
// ============================================================================

async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    tracing::info!("Login attempt for username: {}", req.username);
    
    let user = match users::get_user_by_username(&state.db, &req.username).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            tracing::warn!("User not found: {}", req.username);
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
            tracing::warn!("Invalid password for user: {}", req.username);
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

    tracing::info!("Login successful for user: {} (admin: {})", user.id, user.is_admin);
    
    Ok(Json(AuthResponse {
        token,
        refresh_token,
        user_id: user.id.to_string(),
        username: user.username,
        is_admin: user.is_admin,
        expires_in: 24 * 60 * 60, // 24 hours in seconds
    }))
}

/// Refresh an access token using a refresh token
async fn refresh_token(
    State(state): State<AppState>,
    Json(req): Json<RefreshRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    // Verify the refresh token
    let user_id = auth::verify_refresh_token(&state.config.jwt_secret, &req.refresh_token)
        .map_err(|_| AppError::Unauthorized("Invalid or expired refresh token".into()))?;

    // Get user to return updated info
    let user = users::get_user_by_id(&state.db, user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".into()))?;

    // Create new tokens
    let token = auth::create_access_token(&state.config.jwt_secret, user_id)?;
    let new_refresh_token = auth::create_refresh_token(&state.config.jwt_secret, user_id)?;

    Ok(Json(AuthResponse {
        token,
        refresh_token: new_refresh_token,
        user_id: user_id.to_string(),
        username: user.username,
        is_admin: user.is_admin,
        expires_in: 24 * 60 * 60, // 24 hours in seconds
    }))
}

/// Get current user info
async fn get_current_user(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<UserResponse>, AppError> {
    let user_id = extract_user_id(&state, &headers)?;
    
    let user = users::get_user_by_id(&state.db, user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".into()))?;

    Ok(Json(UserResponse {
        id: user.id.to_string(),
        username: user.username,
        is_admin: user.is_admin,
        created_at: user.created_at.to_rfc3339(),
    }))
}

// ============================================================================
// HANDLERS - Admin Only
// ============================================================================

/// List all users (admin only)
async fn list_users(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<Vec<UserResponse>>, AppError> {
    require_admin(&state, &headers).await?;
    
    let users = users::list_users(&state.db).await?;
    
    let response: Vec<UserResponse> = users
        .into_iter()
        .map(|u| UserResponse {
            id: u.id.to_string(),
            username: u.username,
            is_admin: u.is_admin,
            created_at: u.created_at.to_rfc3339(),
        })
        .collect();
    
    Ok(Json(response))
}

/// Create a new user (admin only)
async fn create_user(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<CreateUserRequest>,
) -> Result<Json<UserResponse>, AppError> {
    require_admin(&state, &headers).await?;
    
    // Validate username
    if req.username.len() < 3 {
        return Err(AppError::BadRequest("Username must be at least 3 characters".into()));
    }
    if !req.username.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
        return Err(AppError::BadRequest("Username can only contain letters, numbers, underscores, and hyphens".into()));
    }
    
    // Check if username exists
    if users::get_user_by_username(&state.db, &req.username).await?.is_some() {
        return Err(AppError::BadRequest("Username already exists".into()));
    }
    
    // Validate password
    if req.password.len() < 4 {
        return Err(AppError::BadRequest("Password must be at least 4 characters".into()));
    }
    
    let password_hash = auth::hash_password(&req.password)?;
    let user = users::create_user(&state.db, &req.username, &password_hash, req.is_admin.unwrap_or(false)).await?;
    
    tracing::info!("Admin created new user: {}", user.username);
    
    Ok(Json(UserResponse {
        id: user.id.to_string(),
        username: user.username,
        is_admin: user.is_admin,
        created_at: user.created_at.to_rfc3339(),
    }))
}

/// Delete a user (admin only)
async fn delete_user(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(user_id): Path<Uuid>,
) -> Result<Json<MessageResponse>, AppError> {
    let admin_id = require_admin(&state, &headers).await?;
    
    // Prevent self-deletion
    if user_id == admin_id {
        return Err(AppError::BadRequest("Cannot delete yourself".into()));
    }
    
    let deleted = users::delete_user(&state.db, user_id).await?;
    
    if deleted {
        tracing::info!("Admin {} deleted user {}", admin_id, user_id);
        Ok(Json(MessageResponse {
            message: "User deleted successfully".into(),
        }))
    } else {
        Err(AppError::NotFound("User not found".into()))
    }
}

/// Reset a user's password (admin only)
async fn reset_user_password(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(user_id): Path<Uuid>,
    Json(req): Json<ResetPasswordRequest>,
) -> Result<Json<MessageResponse>, AppError> {
    require_admin(&state, &headers).await?;
    
    // Validate password
    if req.new_password.len() < 4 {
        return Err(AppError::BadRequest("Password must be at least 4 characters".into()));
    }
    
    let password_hash = auth::hash_password(&req.new_password)?;
    let updated = users::update_password(&state.db, user_id, &password_hash).await?;
    
    if updated {
        tracing::info!("Admin reset password for user {}", user_id);
        Ok(Json(MessageResponse {
            message: "Password updated successfully".into(),
        }))
    } else {
        Err(AppError::NotFound("User not found".into()))
    }
}

/// Toggle admin status (admin only)
async fn toggle_admin(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(user_id): Path<Uuid>,
    Json(req): Json<SetAdminRequest>,
) -> Result<Json<MessageResponse>, AppError> {
    let admin_id = require_admin(&state, &headers).await?;
    
    // Prevent self-demotion
    if user_id == admin_id && !req.is_admin {
        return Err(AppError::BadRequest("Cannot remove your own admin status".into()));
    }
    
    let updated = users::set_admin(&state.db, user_id, req.is_admin).await?;
    
    if updated {
        tracing::info!("Admin {} set user {} admin status to {}", admin_id, user_id, req.is_admin);
        Ok(Json(MessageResponse {
            message: format!("Admin status set to {}", req.is_admin),
        }))
    } else {
        Err(AppError::NotFound("User not found".into()))
    }
}

// ============================================================================
// HELPERS
// ============================================================================

/// Extract user ID from authorization header
fn extract_user_id(state: &AppState, headers: &axum::http::HeaderMap) -> Result<Uuid, AppError> {
    let auth_header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized("Missing authorization header".into()))?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| AppError::Unauthorized("Invalid authorization format".into()))?;

    auth::verify_token(&state.config.jwt_secret, token)
        .map_err(|_| AppError::Unauthorized("Invalid or expired token".into()))
}

/// Require user to be an admin, returns admin user ID
async fn require_admin(state: &AppState, headers: &axum::http::HeaderMap) -> Result<Uuid, AppError> {
    let user_id = extract_user_id(state, headers)?;
    
    let user = users::get_user_by_id(&state.db, user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".into()))?;
    
    if !user.is_admin {
        return Err(AppError::Unauthorized("Admin access required".into()));
    }
    
    Ok(user_id)
}
