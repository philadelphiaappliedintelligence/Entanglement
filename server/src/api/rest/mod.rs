mod routes;

use crate::api::AppState;
use axum::extract::DefaultBodyLimit;
use axum::http::{header, HeaderValue, Method};
use axum::Router;
use std::net::SocketAddr;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

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
        .allow_credentials(true);

    // SECURITY: Body size limit - 1GB max for file uploads
    let body_limit = DefaultBodyLimit::max(1024 * 1024 * 1024); // 1GB

    // Build app WITHOUT rate limiting for now (Docker networking issue)
    // TODO: Re-enable rate limiting with proper key extractor for Docker
    let app = Router::new()
        .merge(routes::auth_routes())
        .merge(routes::file_routes())
        .merge(routes::v1_routes())
        .merge(routes::metadata_routes())
        .merge(routes::admin_routes())
        .layer(cors)
        .layer(body_limit)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

