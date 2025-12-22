//! Database operations for Content Defined Chunking (CDC)
//!
//! Supports BLAKE3 hashing and container-based storage.

use super::models::{Chunk, NewChunk, VersionChunk};
use super::DbPool;
use uuid::Uuid;

/// Legacy database representation of a chunk (for backwards compatibility)
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DbChunk {
    pub hash: String,
    pub size_bytes: i32,
    pub ref_count: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Database representation of a version-chunk mapping
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DbVersionChunk {
    pub id: Uuid,
    pub version_id: Uuid,
    pub chunk_hash: String,
    pub chunk_index: i32,
    pub chunk_offset: i64,
}

/// Create or update a chunk record
/// If the chunk already exists, increment its reference count
pub async fn upsert_chunk(
    pool: &DbPool,
    hash: &str,
    size_bytes: i32,
) -> anyhow::Result<DbChunk> {
    let chunk = sqlx::query_as::<_, DbChunk>(
        r#"
        INSERT INTO chunks (hash, size_bytes, ref_count)
        VALUES ($1, $2, 1)
        ON CONFLICT (hash) DO UPDATE
            SET ref_count = chunks.ref_count + 1
        RETURNING hash, size_bytes, ref_count, created_at
        "#,
    )
    .bind(hash)
    .bind(size_bytes)
    .fetch_one(pool)
    .await?;

    Ok(chunk)
}

/// Get a chunk by hash
pub async fn get_chunk(pool: &DbPool, hash: &str) -> anyhow::Result<Option<DbChunk>> {
    let chunk = sqlx::query_as::<_, DbChunk>(
        r#"
        SELECT hash, size_bytes, ref_count, created_at
        FROM chunks
        WHERE hash = $1
        "#,
    )
    .bind(hash)
    .fetch_optional(pool)
    .await?;

    Ok(chunk)
}

/// Check if a chunk exists
pub async fn chunk_exists(pool: &DbPool, hash: &str) -> anyhow::Result<bool> {
    let exists: (bool,) = sqlx::query_as(
        r#"
        SELECT EXISTS(SELECT 1 FROM chunks WHERE hash = $1)
        "#,
    )
    .bind(hash)
    .fetch_one(pool)
    .await?;

    Ok(exists.0)
}

/// Check which chunks from a list already exist (for delta sync)
pub async fn get_existing_chunks(pool: &DbPool, hashes: &[String]) -> anyhow::Result<Vec<String>> {
    if hashes.is_empty() {
        return Ok(vec![]);
    }
    
    let existing: Vec<(String,)> = sqlx::query_as(
        r#"
        SELECT hash FROM chunks WHERE hash = ANY($1)
        "#,
    )
    .bind(hashes)
    .fetch_all(pool)
    .await?;

    Ok(existing.into_iter().map(|(h,)| h).collect())
}

/// Find which chunks from a list are missing from the database
/// Returns the hashes that do NOT exist (for validation before creating versions)
pub async fn find_missing_chunks(pool: &DbPool, hashes: &[String]) -> anyhow::Result<Vec<String>> {
    if hashes.is_empty() {
        return Ok(vec![]);
    }
    
    let existing = get_existing_chunks(pool, hashes).await?;
    let existing_set: std::collections::HashSet<&String> = existing.iter().collect();
    
    Ok(hashes.iter()
        .filter(|h| !existing_set.contains(h))
        .cloned()
        .collect())
}

/// Get chunk sizes for a list of hashes (preserves order, returns hash->size map)
/// Used to calculate offsets when creating a version from chunk hashes
pub async fn get_chunk_sizes(pool: &DbPool, hashes: &[String]) -> anyhow::Result<std::collections::HashMap<String, i32>> {
    if hashes.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    
    let rows: Vec<(String, i32)> = sqlx::query_as(
        r#"
        SELECT hash, size_bytes FROM chunks WHERE hash = ANY($1)
        "#,
    )
    .bind(hashes)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().collect())
}

/// Decrement chunk reference count
/// Returns true if the chunk was deleted (ref_count reached 0)
pub async fn decrement_chunk_ref(pool: &DbPool, hash: &str) -> anyhow::Result<bool> {
    // First decrement
    sqlx::query(
        r#"
        UPDATE chunks SET ref_count = ref_count - 1 WHERE hash = $1
        "#,
    )
    .bind(hash)
    .execute(pool)
    .await?;

    // Then delete if ref_count is 0
    let result = sqlx::query(
        r#"
        DELETE FROM chunks WHERE hash = $1 AND ref_count <= 0
        "#,
    )
    .bind(hash)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Add a chunk to a version's manifest
pub async fn add_version_chunk(
    pool: &DbPool,
    version_id: Uuid,
    chunk_hash: &str,
    chunk_index: i32,
    chunk_offset: i64,
) -> anyhow::Result<DbVersionChunk> {
    let vc = sqlx::query_as::<_, DbVersionChunk>(
        r#"
        INSERT INTO version_chunks (version_id, chunk_hash, chunk_index, chunk_offset)
        VALUES ($1, $2, $3, $4)
        RETURNING id, version_id, chunk_hash, chunk_index, chunk_offset
        "#,
    )
    .bind(version_id)
    .bind(chunk_hash)
    .bind(chunk_index)
    .bind(chunk_offset)
    .fetch_one(pool)
    .await?;

    Ok(vc)
}

/// Get all chunks for a version, ordered by index
pub async fn get_version_chunks(
    pool: &DbPool,
    version_id: Uuid,
) -> anyhow::Result<Vec<DbVersionChunk>> {
    let chunks = sqlx::query_as::<_, DbVersionChunk>(
        r#"
        SELECT id, version_id, chunk_hash, chunk_index, chunk_offset
        FROM version_chunks
        WHERE version_id = $1
        ORDER BY chunk_index
        "#,
    )
    .bind(version_id)
    .fetch_all(pool)
    .await?;

    Ok(chunks)
}

/// Get chunk hashes for a version (for delta sync comparison)
pub async fn get_version_chunk_hashes(
    pool: &DbPool,
    version_id: Uuid,
) -> anyhow::Result<Vec<String>> {
    let hashes: Vec<(String,)> = sqlx::query_as(
        r#"
        SELECT chunk_hash
        FROM version_chunks
        WHERE version_id = $1
        ORDER BY chunk_index
        "#,
    )
    .bind(version_id)
    .fetch_all(pool)
    .await?;

    Ok(hashes.into_iter().map(|(h,)| h).collect())
}

/// Create a chunked version with all its chunks in a transaction
pub async fn create_chunked_version(
    pool: &DbPool,
    file_id: Uuid,
    blob_hash: &str,  // Overall file hash
    size_bytes: i64,
    chunks: &[(String, i32, i64)],  // (hash, size, offset)
) -> anyhow::Result<Uuid> {
    // Use a transaction to ensure atomicity
    let mut tx = pool.begin().await?;
    
    // Create version record
    let version_id: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO versions (file_id, blob_hash, size_bytes, is_chunked)
        VALUES ($1, $2, $3, TRUE)
        RETURNING id
        "#,
    )
    .bind(file_id)
    .bind(blob_hash)
    .bind(size_bytes)
    .fetch_one(&mut *tx)
    .await?;
    
    let version_id = version_id.0;
    
    // Insert/update chunks and create mappings
    for (index, (hash, size, offset)) in chunks.iter().enumerate() {
        // Upsert chunk
        sqlx::query(
            r#"
            INSERT INTO chunks (hash, size_bytes, ref_count)
            VALUES ($1, $2, 1)
            ON CONFLICT (hash) DO UPDATE
                SET ref_count = chunks.ref_count + 1
            "#,
        )
        .bind(hash)
        .bind(size)
        .execute(&mut *tx)
        .await?;
        
        // Create version-chunk mapping
        sqlx::query(
            r#"
            INSERT INTO version_chunks (version_id, chunk_hash, chunk_index, chunk_offset)
            VALUES ($1, $2, $3, $4)
            "#,
        )
        .bind(version_id)
        .bind(hash)
        .bind(index as i32)
        .bind(offset)
        .execute(&mut *tx)
        .await?;
    }
    
    // Update file's current version
    // NOTE: Does NOT update `updated_at` to preserve the original file modification date
    sqlx::query(
        r#"
        UPDATE files SET current_version_id = $1
        WHERE id = $2
        "#,
    )
    .bind(version_id)
    .bind(file_id)
    .execute(&mut *tx)
    .await?;
    
    tx.commit().await?;
    
    Ok(version_id)
}

/// Get statistics about chunk storage
#[allow(dead_code)]
pub async fn get_chunk_stats(pool: &DbPool) -> anyhow::Result<ChunkStats> {
    let stats: (i64, i64, i64) = sqlx::query_as(
        r#"
        SELECT 
            COUNT(*) as total_chunks,
            COALESCE(SUM(size_bytes), 0) as total_size,
            COALESCE(SUM(ref_count), 0) as total_refs
        FROM chunks
        "#,
    )
    .fetch_one(pool)
    .await?;

    Ok(ChunkStats {
        total_chunks: stats.0,
        total_size_bytes: stats.1,
        total_references: stats.2,
    })
}

#[derive(Debug)]
pub struct ChunkStats {
    pub total_chunks: i64,
    pub total_size_bytes: i64,
    pub total_references: i64,
}

impl ChunkStats {
    /// Calculate deduplication ratio
    /// Returns how much space is saved by deduplication (e.g., 2.0 = 50% savings)
    #[allow(dead_code)]
    pub fn dedup_ratio(&self) -> f64 {
        if self.total_chunks == 0 {
            return 1.0;
        }
        self.total_references as f64 / self.total_chunks as f64
    }
}

// =============================================================================
// New API with Container and Tier Support
// =============================================================================

use super::models::ChunkTier;

/// Chunk info for version creation
#[derive(Debug, Clone)]
pub struct ChunkInfo {
    pub hash: String,
    pub size_bytes: i32,
    pub offset_in_file: i64,
}

/// Create a chunked version with tier tracking
/// This is the primary API for creating new file versions.
///
/// Prerequisites: All chunks must already exist in the database.
pub async fn create_version_with_tier(
    pool: &DbPool,
    file_id: Uuid,
    blake3_hash: &str,
    size_bytes: i64,
    tier: ChunkTier,
    chunks: &[ChunkInfo],
) -> anyhow::Result<Uuid> {
    let mut tx = pool.begin().await?;
    
    // Create version record with tier and blake3_hash
    let version_id: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO versions (file_id, blob_hash, blake3_hash, size_bytes, tier_id, is_chunked)
        VALUES ($1, $2, $2, $3, $4, $5)
        RETURNING id
        "#,
    )
    .bind(file_id)
    .bind(blake3_hash)
    .bind(size_bytes)
    .bind(tier as i16)
    .bind(!chunks.is_empty()) // is_chunked = true if we have chunks
    .fetch_one(&mut *tx)
    .await?;
    
    let version_id = version_id.0;
    
    // Insert chunk mappings and increment ref counts
    for (index, chunk) in chunks.iter().enumerate() {
        // Increment chunk reference count
        sqlx::query(
            r#"
            UPDATE chunks SET ref_count = ref_count + 1 WHERE hash = $1
            "#,
        )
        .bind(&chunk.hash)
        .execute(&mut *tx)
        .await?;
        
        // Create version-chunk mapping
        sqlx::query(
            r#"
            INSERT INTO version_chunks (version_id, chunk_hash, chunk_index, chunk_offset)
            VALUES ($1, $2, $3, $4)
            "#,
        )
        .bind(version_id)
        .bind(&chunk.hash)
        .bind(index as i32)
        .bind(chunk.offset_in_file)
        .execute(&mut *tx)
        .await?;
    }
    
    // Update file's current version
    // NOTE: Does NOT update `updated_at` to preserve the original file modification date
    sqlx::query(
        r#"
        UPDATE files SET current_version_id = $1
        WHERE id = $2
        "#,
    )
    .bind(version_id)
    .bind(file_id)
    .execute(&mut *tx)
    .await?;
    
    tx.commit().await?;
    
    tracing::info!(
        "Created version {} for file {} with tier {:?} ({} chunks)",
        version_id, file_id, tier, chunks.len()
    );
    
    Ok(version_id)
}

/// Upsert a chunk with container location
pub async fn upsert_chunk_with_location(
    pool: &DbPool,
    new_chunk: &NewChunk,
) -> anyhow::Result<Chunk> {
    let chunk = sqlx::query_as::<_, Chunk>(
        r#"
        INSERT INTO chunks (hash, size_bytes, ref_count, container_id, offset_bytes, length_bytes)
        VALUES ($1, $2, 0, $3, $4, $5)
        ON CONFLICT (hash) DO UPDATE
            SET container_id = COALESCE(chunks.container_id, EXCLUDED.container_id),
                offset_bytes = COALESCE(chunks.offset_bytes, EXCLUDED.offset_bytes),
                length_bytes = COALESCE(chunks.length_bytes, EXCLUDED.length_bytes)
        RETURNING hash, size_bytes, ref_count, container_id, offset_bytes, length_bytes, created_at
        "#,
    )
    .bind(&new_chunk.hash)
    .bind(new_chunk.size_bytes)
    .bind(new_chunk.container_id)
    .bind(new_chunk.offset_bytes)
    .bind(new_chunk.length_bytes)
    .fetch_one(pool)
    .await?;

    Ok(chunk)
}

/// Get a chunk with full location info
pub async fn get_chunk_with_location(pool: &DbPool, hash: &str) -> anyhow::Result<Option<Chunk>> {
    let chunk = sqlx::query_as::<_, Chunk>(
        r#"
        SELECT hash, size_bytes, ref_count, container_id, offset_bytes, length_bytes, created_at
        FROM chunks
        WHERE hash = $1
        "#,
    )
    .bind(hash)
    .fetch_optional(pool)
    .await?;

    Ok(chunk)
}

/// Get all chunks for a version with their location info
pub async fn get_version_chunks_with_location(
    pool: &DbPool,
    version_id: Uuid,
) -> anyhow::Result<Vec<(VersionChunk, Chunk)>> {
    // We need to do a join here
    let rows: Vec<(Uuid, Uuid, String, i32, i64, String, i32, i32, Option<Uuid>, Option<i64>, Option<i32>, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        r#"
        SELECT 
            vc.id, vc.version_id, vc.chunk_hash, vc.chunk_index, vc.chunk_offset,
            c.hash, c.size_bytes, c.ref_count, c.container_id, c.offset_bytes, c.length_bytes, c.created_at
        FROM version_chunks vc
        JOIN chunks c ON vc.chunk_hash = c.hash
        WHERE vc.version_id = $1
        ORDER BY vc.chunk_index
        "#,
    )
    .bind(version_id)
    .fetch_all(pool)
    .await?;

    let results = rows.into_iter().map(|row| {
        let vc = VersionChunk {
            id: row.0,
            version_id: row.1,
            chunk_hash: row.2,
            chunk_index: row.3,
            chunk_offset: row.4,
        };
        let chunk = Chunk {
            hash: row.5,
            size_bytes: row.6,
            ref_count: row.7,
            container_id: row.8,
            offset_bytes: row.9,
            length_bytes: row.10,
            created_at: row.11,
        };
        (vc, chunk)
    }).collect();

    Ok(results)
}

/// Batch insert multiple chunks (for efficient bulk operations)
pub async fn batch_upsert_chunks(
    pool: &DbPool,
    chunks: &[NewChunk],
) -> anyhow::Result<()> {
    if chunks.is_empty() {
        return Ok(());
    }
    
    let mut tx = pool.begin().await?;
    
    for chunk in chunks {
        sqlx::query(
            r#"
            INSERT INTO chunks (hash, size_bytes, ref_count, container_id, offset_bytes, length_bytes)
            VALUES ($1, $2, 0, $3, $4, $5)
            ON CONFLICT (hash) DO NOTHING
            "#,
        )
        .bind(&chunk.hash)
        .bind(chunk.size_bytes)
        .bind(chunk.container_id)
        .bind(chunk.offset_bytes)
        .bind(chunk.length_bytes)
        .execute(&mut *tx)
        .await?;
    }
    
    tx.commit().await?;
    
    Ok(())
}

