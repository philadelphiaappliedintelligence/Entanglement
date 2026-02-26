//! Database operations for Blob Containers
//!
//! Blob containers are append-only pack files that store multiple chunks
//! for efficient disk I/O. Reserved for future container-based storage.

#![allow(dead_code)]

use super::models::{BlobContainer, NewBlobContainer};
use super::DbPool;
use uuid::Uuid;

/// Create a new blob container
pub async fn create_container(
    pool: &DbPool,
    new_container: &NewBlobContainer,
) -> anyhow::Result<BlobContainer> {
    let container = sqlx::query_as::<_, BlobContainer>(
        r#"
        INSERT INTO blob_containers (disk_path)
        VALUES ($1)
        RETURNING id, disk_path, total_size, chunk_count, is_sealed, created_at, sealed_at
        "#,
    )
    .bind(&new_container.disk_path)
    .fetch_one(pool)
    .await?;

    Ok(container)
}

/// Get a container by ID
pub async fn get_container(pool: &DbPool, id: Uuid) -> anyhow::Result<Option<BlobContainer>> {
    let container = sqlx::query_as::<_, BlobContainer>(
        r#"
        SELECT id, disk_path, total_size, chunk_count, is_sealed, created_at, sealed_at
        FROM blob_containers
        WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(container)
}

/// Find an open container with enough space for a chunk
/// Returns the container with the most space available under the max size
pub async fn find_open_container(
    pool: &DbPool,
    max_container_size: i64,
    chunk_size: i32,
) -> anyhow::Result<Option<BlobContainer>> {
    let container = sqlx::query_as::<_, BlobContainer>(
        r#"
        SELECT id, disk_path, total_size, chunk_count, is_sealed, created_at, sealed_at
        FROM blob_containers
        WHERE is_sealed = FALSE
          AND total_size + $1 <= $2
        ORDER BY total_size DESC
        LIMIT 1
        "#,
    )
    .bind(chunk_size as i64)
    .bind(max_container_size)
    .fetch_optional(pool)
    .await?;

    Ok(container)
}

/// Update container stats after adding a chunk
pub async fn add_chunk_to_container(
    pool: &DbPool,
    container_id: Uuid,
    chunk_size: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        UPDATE blob_containers
        SET total_size = total_size + $2,
            chunk_count = chunk_count + 1
        WHERE id = $1
        "#,
    )
    .bind(container_id)
    .bind(chunk_size)
    .execute(pool)
    .await?;

    Ok(())
}

/// Seal a container (mark as read-only)
pub async fn seal_container(pool: &DbPool, container_id: Uuid) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        UPDATE blob_containers
        SET is_sealed = TRUE, sealed_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(container_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// List all containers
#[allow(dead_code)]
pub async fn list_containers(
    pool: &DbPool,
    include_sealed: bool,
) -> anyhow::Result<Vec<BlobContainer>> {
    let containers = sqlx::query_as::<_, BlobContainer>(
        r#"
        SELECT id, disk_path, total_size, chunk_count, is_sealed, created_at, sealed_at
        FROM blob_containers
        WHERE $1 OR is_sealed = FALSE
        ORDER BY created_at DESC
        "#,
    )
    .bind(include_sealed)
    .fetch_all(pool)
    .await?;

    Ok(containers)
}

/// Get container storage statistics
#[derive(Debug)]
pub struct ContainerStats {
    pub total_containers: i64,
    pub open_containers: i64,
    pub sealed_containers: i64,
    pub total_size_bytes: i64,
    pub total_chunks: i64,
}

#[allow(dead_code)]
pub async fn get_container_stats(pool: &DbPool) -> anyhow::Result<ContainerStats> {
    let stats: (i64, i64, i64, Option<i64>, Option<i64>) = sqlx::query_as(
        r#"
        SELECT 
            COUNT(*) as total,
            COUNT(*) FILTER (WHERE is_sealed = FALSE) as open_count,
            COUNT(*) FILTER (WHERE is_sealed = TRUE) as sealed_count,
            SUM(total_size) as total_size,
            SUM(chunk_count) as total_chunks
        FROM blob_containers
        "#,
    )
    .fetch_one(pool)
    .await?;

    Ok(ContainerStats {
        total_containers: stats.0,
        open_containers: stats.1,
        sealed_containers: stats.2,
        total_size_bytes: stats.3.unwrap_or(0),
        total_chunks: stats.4.unwrap_or(0),
    })
}











