//! REST API module
//!
//! Organized into domain-specific submodules for maintainability.

mod admin;
mod auth;
mod blobs;
mod chunks;
mod error;
mod files;
mod types;
mod v1;
mod versions;

use crate::api::AppState;
use axum::extract::DefaultBodyLimit;
use axum::http::{header, HeaderValue, Method};
use axum::Router;
use std::net::SocketAddr;
use tower_http::cors::CorsLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::Level;

// Re-export router functions for external use
pub use admin::admin_routes;
pub use auth::auth_routes;
pub use blobs::metadata_routes;
pub use files::file_routes;
pub use v1::v1_routes;

pub async fn serve(addr: SocketAddr, state: AppState) -> anyhow::Result<()> {
    // CORS: Read allowed origins from CORS_ORIGINS env var (comma-separated)
    // Falls back to localhost for development
    let cors_origins: Vec<HeaderValue> = std::env::var("CORS_ORIGINS")
        .unwrap_or_else(|_| "http://localhost:3000,http://127.0.0.1:3000".to_string())
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    
    tracing::info!("CORS allowed origins: {:?}", cors_origins);
    
    let cors = CorsLayer::new()
        .allow_origin(cors_origins)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::ACCEPT,
        ])
        .allow_credentials(true)
        // Expose X-Request-Id header to clients
        .expose_headers(vec![header::HeaderName::from_static("x-request-id")]);

    // SECURITY: Body size limit - 1GB max for file uploads
    let body_limit = DefaultBodyLimit::max(1024 * 1024 * 1024); // 1GB

    // Request ID header name
    let x_request_id = header::HeaderName::from_static("x-request-id");

    // Tracing layer with request ID included in spans
    let trace_layer = TraceLayer::new_for_http()
        .make_span_with(DefaultMakeSpan::new().include_headers(true).level(Level::INFO))
        .on_response(DefaultOnResponse::new().level(Level::INFO));

    // Build app with request ID middleware
    let app = Router::new()
        .merge(auth_routes())
        .merge(file_routes())
        .merge(v1_routes())
        .merge(metadata_routes())
        .merge(admin_routes())
        .layer(cors)
        .layer(body_limit)
        // Request ID: Generate UUID, set on request, propagate to response
        .layer(PropagateRequestIdLayer::new(x_request_id.clone()))
        .layer(SetRequestIdLayer::new(x_request_id, MakeRequestUuid))
        .layer(trace_layer)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    
    // Graceful shutdown: wait for SIGTERM or SIGINT
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// Wait for shutdown signal (SIGTERM or SIGINT)
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("Received SIGINT, starting graceful shutdown...");
        },
        _ = terminate => {
            tracing::info!("Received SIGTERM, starting graceful shutdown...");
        },
    }
}
