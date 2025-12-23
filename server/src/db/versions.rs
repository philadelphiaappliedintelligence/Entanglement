//! Database operations for file versions with BLAKE3 and tier support.

#![allow(dead_code)]

use super::DbPool;
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Legacy Version struct (for backwards compatibility)
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Version {
    pub id: Uuid,
    pub file_id: Uuid,
    pub blob_hash: String,
    pub size_bytes: i64,
    pub created_at: DateTime<Utc>,
    pub created_by: Option<Uuid>,
}

/// Extended Version struct with tier and BLAKE3 support
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct VersionExt {
    pub id: Uuid,
    pub file_id: Uuid,
    pub blob_hash: String,
    pub blake3_hash: Option<String>,
    pub size_bytes: i64,
    pub tier_id: i16,
    pub is_chunked: bool,
    pub created_at: DateTime<Utc>,
    pub created_by: Option<Uuid>,
}

impl VersionExt {
    /// Get the content hash (prefers blake3_hash)
    pub fn content_hash(&self) -> &str {
        self.blake3_hash.as_deref().unwrap_or(&self.blob_hash)
    }
}

/// Create a new version for a file
pub async fn create_version(
    pool: &DbPool,
    file_id: Uuid,
    blob_hash: &str,
    size_bytes: i64,
    created_by: Uuid,
) -> anyhow::Result<Version> {
    let version = sqlx::query_as::<_, Version>(
        r#"
        INSERT INTO versions (file_id, blob_hash, size_bytes, created_by)
        VALUES ($1, $2, $3, $4)
        RETURNING id, file_id, blob_hash, size_bytes, created_at, created_by
        "#,
    )
    .bind(file_id)
    .bind(blob_hash)
    .bind(size_bytes)
    .bind(created_by)
    .fetch_one(pool)
    .await?;

    Ok(version)
}

/// Create a new version without user tracking (for indexing)
pub async fn create_version_global(
    pool: &DbPool,
    file_id: Uuid,
    blob_hash: &str,
    size_bytes: i64,
) -> anyhow::Result<Version> {
    let version = sqlx::query_as::<_, Version>(
        r#"
        INSERT INTO versions (file_id, blob_hash, size_bytes, created_by)
        VALUES ($1, $2, $3, NULL)
        RETURNING id, file_id, blob_hash, size_bytes, created_at, created_by
        "#,
    )
    .bind(file_id)
    .bind(blob_hash)
    .bind(size_bytes)
    .fetch_one(pool)
    .await?;

    Ok(version)
}

/// Get a version by ID
pub async fn get_version(pool: &DbPool, version_id: Uuid) -> anyhow::Result<Option<Version>> {
    let version = sqlx::query_as::<_, Version>(
        r#"
        SELECT id, file_id, blob_hash, size_bytes, created_at, created_by
        FROM versions
        WHERE id = $1
        "#,
    )
    .bind(version_id)
    .fetch_optional(pool)
    .await?;

    Ok(version)
}

/// List versions for a file (newest first)
pub async fn list_versions(
    pool: &DbPool,
    file_id: Uuid,
    limit: i64,
    offset: i64,
) -> anyhow::Result<(Vec<Version>, i64)> {
    let versions = sqlx::query_as::<_, Version>(
        r#"
        SELECT id, file_id, blob_hash, size_bytes, created_at, created_by
        FROM versions
        WHERE file_id = $1
        ORDER BY created_at DESC
        LIMIT $2 OFFSET $3
        "#,
    )
    .bind(file_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    let total: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*)
        FROM versions
        WHERE file_id = $1
        "#,
    )
    .bind(file_id)
    .fetch_one(pool)
    .await?;

    Ok((versions, total.0))
}

/// Get the latest version for a file
#[allow(dead_code)]
pub async fn get_latest_version(pool: &DbPool, file_id: Uuid) -> anyhow::Result<Option<Version>> {
    let version = sqlx::query_as::<_, Version>(
        r#"
        SELECT id, file_id, blob_hash, size_bytes, created_at, created_by
        FROM versions
        WHERE file_id = $1
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(file_id)
    .fetch_optional(pool)
    .await?;

    Ok(version)
}

// =============================================================================
// Extended Version API with BLAKE3 and Tier Support
// =============================================================================

use super::models::ChunkTier;

/// Create a new version with tier and BLAKE3 hash
pub async fn create_version_with_tier(
    pool: &DbPool,
    file_id: Uuid,
    blake3_hash: &str,
    size_bytes: i64,
    tier: ChunkTier,
    is_chunked: bool,
    created_by: Option<Uuid>,
) -> anyhow::Result<VersionExt> {
    let version = sqlx::query_as::<_, VersionExt>(
        r#"
        INSERT INTO versions (file_id, blob_hash, blake3_hash, size_bytes, tier_id, is_chunked, created_by)
        VALUES ($1, $2, $2, $3, $4, $5, $6)
        RETURNING id, file_id, blob_hash, blake3_hash, size_bytes, tier_id, is_chunked, created_at, created_by
        "#,
    )
    .bind(file_id)
    .bind(blake3_hash)
    .bind(size_bytes)
    .bind(tier as i16)
    .bind(is_chunked)
    .bind(created_by)
    .fetch_one(pool)
    .await?;

    Ok(version)
}

/// Get a version by ID with extended info
pub async fn get_version_ext(pool: &DbPool, version_id: Uuid) -> anyhow::Result<Option<VersionExt>> {
    let version = sqlx::query_as::<_, VersionExt>(
        r#"
        SELECT id, file_id, blob_hash, blake3_hash, size_bytes, tier_id, is_chunked, created_at, created_by
        FROM versions
        WHERE id = $1
        "#,
    )
    .bind(version_id)
    .fetch_optional(pool)
    .await?;

    Ok(version)
}

/// Find a version by its BLAKE3 hash (for deduplication)
#[allow(dead_code)]
pub async fn find_version_by_blake3(
    pool: &DbPool,
    blake3_hash: &str,
) -> anyhow::Result<Option<VersionExt>> {
    let version = sqlx::query_as::<_, VersionExt>(
        r#"
        SELECT id, file_id, blob_hash, blake3_hash, size_bytes, tier_id, is_chunked, created_at, created_by
        FROM versions
        WHERE blake3_hash = $1
        LIMIT 1
        "#,
    )
    .bind(blake3_hash)
    .fetch_optional(pool)
    .await?;

    Ok(version)
}

/// Get the latest version for a file with extended info
pub async fn get_latest_version_ext(
    pool: &DbPool,
    file_id: Uuid,
) -> anyhow::Result<Option<VersionExt>> {
    let version = sqlx::query_as::<_, VersionExt>(
        r#"
        SELECT id, file_id, blob_hash, blake3_hash, size_bytes, tier_id, is_chunked, created_at, created_by
        FROM versions
        WHERE file_id = $1
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(file_id)
    .fetch_optional(pool)
    .await?;

    Ok(version)
}

