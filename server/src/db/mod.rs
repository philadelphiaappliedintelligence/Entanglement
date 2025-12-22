pub mod chunks;
pub mod containers;
pub mod files;
pub mod models;
pub mod users;
pub mod versions;

use sqlx::postgres::PgPoolOptions;
use sqlx::{Pool, Postgres};

pub type DbPool = Pool<Postgres>;

// Re-export commonly used types
pub use models::{
    BlobContainer, Chunk, ChunkLocation, ChunkTier, File, FileVersion,
    FileWithVersion, NewBlobContainer, NewChunk, NewFileVersion, NewVersionChunk,
    TierConfig, VersionChunk,
};

/// Create a database connection pool
pub async fn create_pool(database_url: &str) -> anyhow::Result<DbPool> {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(database_url)
        .await?;
    Ok(pool)
}

/// Run database migrations using SQLx's built-in migration tracking.
/// Migrations are tracked in the `_sqlx_migrations` table and only run once.
pub async fn run_migrations(pool: &DbPool) -> anyhow::Result<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await?;
    Ok(())
}

/// Server statistics
pub struct Stats {
    pub total_users: i64,
    pub total_files: i64,
    pub total_versions: i64,
    pub total_blob_bytes: i64,
}

/// Get server statistics
pub async fn get_stats(pool: &DbPool) -> anyhow::Result<Stats> {
    let total_users: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await?;

    let total_files: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM files")
        .fetch_one(pool)
        .await?;

    let total_versions: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM versions")
        .fetch_one(pool)
        .await?;

    // Cast to BIGINT to avoid NUMERIC type mismatch
    let total_blob_bytes: (Option<i64>,) =
        sqlx::query_as("SELECT CAST(COALESCE(SUM(size_bytes), 0) AS BIGINT) FROM versions")
            .fetch_one(pool)
            .await?;

    Ok(Stats {
        total_users: total_users.0,
        total_files: total_files.0,
        total_versions: total_versions.0,
        total_blob_bytes: total_blob_bytes.0.unwrap_or(0),
    })
}

