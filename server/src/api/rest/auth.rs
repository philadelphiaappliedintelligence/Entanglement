//! Authentication routes
//!
//! Handles user registration, login, token refresh, and password reset.

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
        .route("/auth/forgot-password", post(forgot_password))
        .route("/auth/reset-password", post(reset_password))
        .route("/auth/send-verification", post(send_verification))
        .route("/auth/verify-email", post(verify_email))
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

#[derive(Deserialize)]
struct ForgotPasswordRequest {
    email: String,
}

#[derive(Serialize)]
struct ForgotPasswordResponse {
    message: String,
    /// In development mode, the reset token is included for testing
    #[serde(skip_serializing_if = "Option::is_none")]
    debug_token: Option<String>,
}

#[derive(Deserialize)]
struct ResetPasswordRequest {
    token: String,
    new_password: String,
}

#[derive(Serialize)]
struct ResetPasswordResponse {
    message: String,
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

/// Request a password reset email
/// 
/// Always returns success to prevent email enumeration attacks.
/// In development mode, includes the token in the response for testing.
async fn forgot_password(
    State(state): State<AppState>,
    Json(req): Json<ForgotPasswordRequest>,
) -> Result<Json<ForgotPasswordResponse>, AppError> {
    tracing::info!("Password reset requested for: {}", req.email);
    
    // Generate secure random token (32 hex characters)
    let token_bytes: [u8; 16] = rand::random();
    let token: String = token_bytes.iter().map(|b| format!("{:02x}", b)).collect();
    
    // Hash token for storage (never store plaintext)
    let token_hash = blake3::hash(token.as_bytes()).to_hex().to_string();
    
    // Try to find user and create reset token
    if let Ok(Some(user)) = users::get_user_by_email(&state.db, &req.email).await {
        // Store hashed token in database (expires in 1 hour)
        let expires_at = chrono::Utc::now() + chrono::Duration::hours(1);
        
        let _ = sqlx::query(
            r#"
            INSERT INTO password_reset_tokens (user_id, token_hash, expires_at)
            VALUES ($1, $2, $3)
            "#
        )
        .bind(user.id)
        .bind(&token_hash)
        .bind(expires_at)
        .execute(&state.db)
        .await;
        
        // In production, send email here
        // For now, log the token (development only)
        tracing::info!("Password reset token for {}: {} (hash: {})", 
            req.email, 
            token,
            &token_hash[..16]
        );
        
        // In development mode, return the token for testing
        #[cfg(debug_assertions)]
        return Ok(Json(ForgotPasswordResponse {
            message: "If this email exists, a reset link has been sent.".into(),
            debug_token: Some(token),
        }));
    }
    
    // Always return same response to prevent email enumeration
    Ok(Json(ForgotPasswordResponse {
        message: "If this email exists, a reset link has been sent.".into(),
        debug_token: None,
    }))
}

/// Reset password using a valid token
async fn reset_password(
    State(state): State<AppState>,
    Json(req): Json<ResetPasswordRequest>,
) -> Result<Json<ResetPasswordResponse>, AppError> {
    // Hash the provided token
    let token_hash = blake3::hash(req.token.as_bytes()).to_hex().to_string();
    
    // Find valid token
    let token_record = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid)>(
        r#"
        SELECT id, user_id FROM password_reset_tokens 
        WHERE token_hash = $1 
          AND expires_at > NOW() 
          AND used_at IS NULL
        "#
    )
    .bind(&token_hash)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        tracing::error!("Database error during password reset: {}", e);
        AppError::Internal("Database error".into())
    })?;
    
    let (token_id, user_id) = match token_record {
        Some(r) => r,
        None => {
            tracing::warn!("Invalid or expired password reset token");
            return Err(AppError::BadRequest("Invalid or expired reset token".into()));
        }
    };
    
    // Hash new password
    let password_hash = auth::hash_password(&req.new_password)?;
    
    // Update user password
    sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
        .bind(&password_hash)
        .bind(user_id)
        .execute(&state.db)
        .await
        .map_err(|e| {
            tracing::error!("Failed to update password: {}", e);
            AppError::Internal("Failed to update password".into())
        })?;
    
    // Mark token as used
    sqlx::query("UPDATE password_reset_tokens SET used_at = NOW() WHERE id = $1")
        .bind(token_id)
        .execute(&state.db)
        .await
        .ok();
    
    tracing::info!("Password reset successful for user: {}", user_id);
    
    Ok(Json(ResetPasswordResponse {
        message: "Password has been reset successfully.".into(),
    }))
}

// ============================================================================
// EMAIL VERIFICATION
// ============================================================================

#[derive(Serialize)]
struct SendVerificationResponse {
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    debug_token: Option<String>,
}

#[derive(Deserialize)]
struct VerifyEmailRequest {
    token: String,
}

#[derive(Serialize)]
struct VerifyEmailResponse {
    message: String,
    verified: bool,
}

/// Send email verification token
/// Requires authentication - uses JWT to identify user
async fn send_verification(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<SendVerificationResponse>, AppError> {
    // Extract user from Authorization header
    let auth_header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or_else(|| AppError::Unauthorized("Missing authorization".into()))?;
    
    let user_id = auth::verify_token(&state.config.jwt_secret, auth_header)
        .map_err(|_| AppError::Unauthorized("Invalid token".into()))?;
    
    // Generate verification token (32 hex characters)
    let token_bytes: [u8; 16] = rand::random();
    let token: String = token_bytes.iter().map(|b| format!("{:02x}", b)).collect();
    
    // Hash token for storage
    let token_hash = blake3::hash(token.as_bytes()).to_hex().to_string();
    
    // Store token (expires in 24 hours)
    let expires_at = chrono::Utc::now() + chrono::Duration::hours(24);
    
    sqlx::query(
        r#"
        INSERT INTO email_verification_tokens (user_id, token_hash, expires_at)
        VALUES ($1, $2, $3)
        "#
    )
    .bind(user_id)
    .bind(&token_hash)
    .bind(expires_at)
    .execute(&state.db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to create verification token: {}", e);
        AppError::Internal("Failed to create verification token".into())
    })?;
    
    tracing::info!("Verification token created for user: {}", user_id);
    
    // In development mode, return the token for testing
    #[cfg(debug_assertions)]
    return Ok(Json(SendVerificationResponse {
        message: "Verification email sent.".into(),
        debug_token: Some(token),
    }));
    
    #[cfg(not(debug_assertions))]
    Ok(Json(SendVerificationResponse {
        message: "Verification email sent.".into(),
        debug_token: None,
    }))
}

/// Verify email using token
async fn verify_email(
    State(state): State<AppState>,
    Json(req): Json<VerifyEmailRequest>,
) -> Result<Json<VerifyEmailResponse>, AppError> {
    // Hash the provided token
    let token_hash = blake3::hash(req.token.as_bytes()).to_hex().to_string();
    
    // Find valid token
    let token_record = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid)>(
        r#"
        SELECT id, user_id FROM email_verification_tokens 
        WHERE token_hash = $1 
          AND expires_at > NOW() 
          AND used_at IS NULL
        "#
    )
    .bind(&token_hash)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| {
        tracing::error!("Database error during email verification: {}", e);
        AppError::Internal("Database error".into())
    })?;
    
    let (token_id, user_id) = match token_record {
        Some(r) => r,
        None => {
            return Ok(Json(VerifyEmailResponse {
                message: "Invalid or expired verification token.".into(),
                verified: false,
            }));
        }
    };
    
    // Mark user as verified
    sqlx::query("UPDATE users SET email_verified = TRUE WHERE id = $1")
        .bind(user_id)
        .execute(&state.db)
        .await
        .map_err(|e| {
            tracing::error!("Failed to verify email: {}", e);
            AppError::Internal("Failed to verify email".into())
        })?;
    
    // Mark token as used
    sqlx::query("UPDATE email_verification_tokens SET used_at = NOW() WHERE id = $1")
        .bind(token_id)
        .execute(&state.db)
        .await
        .ok();
    
    tracing::info!("Email verified for user: {}", user_id);
    
    Ok(Json(VerifyEmailResponse {
        message: "Email verified successfully.".into(),
        verified: true,
    }))
}
