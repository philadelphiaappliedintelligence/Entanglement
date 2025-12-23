//! Authentication routes
//!
//! Handles user registration, login, and token refresh.

use crate::api::AppState;
use crate::auth;
use crate::db::users;
use axum::{
    extract::State,
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use super::error::AppError;

// ============================================================================
// ROUTES
// ============================================================================

pub fn auth_routes() -> Router<AppState> {
    Router::new()
        .route("/auth/register", post(register))
        .route("/auth/login", post(login))
        .route("/auth/refresh", post(refresh_token))
}

// ============================================================================
// TYPES
// ============================================================================

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

#[derive(Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
}

#[derive(Deserialize)]
struct RefreshRequest {
    refresh_token: String,
}

// ============================================================================
// HANDLERS
// ============================================================================

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
