//! Blob Container Storage Engine
//!
//! Manages physical storage of chunks in container files (packfiles).
//! Each container is an append-only file that stores multiple chunks
//! for efficient disk I/O.
//!
//! ## Container Format
//!
//! ```text
//! +------------------+-------------------+-------------------+
//! | Header (8 bytes) | Chunk 1 (var)     | Chunk 2 (var)     | ...
//! +------------------+-------------------+-------------------+
//! ```
//!
//! Header:
//! - Bytes 0-3: Magic "ENTG" (0x454E5447)
//! - Byte 4: Version (0x01)
//! - Bytes 5-7: Reserved (0x00)

use crate::db::{self, containers, ChunkTier, DbPool, NewChunk};
use anyhow::{anyhow, Context, Result};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

// Container file constants
const MAGIC_BYTES: &[u8; 4] = b"ENTG";
const FORMAT_VERSION: u8 = 0x01;
const HEADER_SIZE: u64 = 8;
const DEFAULT_MAX_CONTAINER_SIZE: u64 = 64 * 1024 * 1024; // 64 MB
const ZSTD_COMPRESSION_LEVEL: i32 = 3;

/// Location of a chunk within the storage system
#[derive(Debug, Clone)]
pub struct ChunkLocation {
    pub container_id: Uuid,
    pub offset: u64,
    pub length: u32,
    pub compressed: bool,
}

/// An open container file ready for writing
#[allow(dead_code)]
struct OpenContainer {
    id: Uuid,
    disk_path: PathBuf,
    file: std::fs::File,
    current_offset: u64,
}

/// Manages blob container storage
///
/// Thread-safe: uses a Mutex to serialize writes to the current container.
pub struct BlobManager {
    base_path: PathBuf,
    db_pool: DbPool,
    /// Guards the current open container to prevent concurrent writes
    current_container: Arc<Mutex<Option<OpenContainer>>>,
    max_container_size: u64,
}

impl BlobManager {
    /// Create a new BlobManager
    pub fn new(base_path: impl AsRef<Path>, db_pool: DbPool) -> Result<Self> {
        let base_path = base_path.as_ref().to_path_buf();
        std::fs::create_dir_all(&base_path)
            .context("Failed to create blob storage directory")?;

        Ok(Self {
            base_path,
            db_pool,
            current_container: Arc::new(Mutex::new(None)),
            max_container_size: DEFAULT_MAX_CONTAINER_SIZE,
        })
    }

    /// Write a chunk to storage
    ///
    /// Returns the location where the chunk was stored.
    /// Compresses data using Zstd for tiers 0-2, skips compression for tiers 3-4.
    pub async fn write_chunk(
        &self,
        hash: &str,
        data: &[u8],
        tier: ChunkTier,
    ) -> Result<ChunkLocation> {
        // Determine if we should compress based on tier
        let should_compress = matches!(tier, ChunkTier::Inline | ChunkTier::Granular | ChunkTier::Standard);
        
        // Compress if needed
        let (write_data, compressed) = if should_compress && !data.is_empty() {
            let compressed = zstd::encode_all(data, ZSTD_COMPRESSION_LEVEL)
                .context("Zstd compression failed")?;
            
            // Only use compressed version if it's actually smaller
            if compressed.len() < data.len() {
                (compressed, true)
            } else {
                (data.to_vec(), false)
            }
        } else {
            (data.to_vec(), false)
        };

        let data_len = write_data.len() as u32;

        // Lock the container for writing
        let mut guard = self.current_container.lock().await;

        // Get or create an open container
        let container = self
            .get_or_create_container(&mut guard, data_len as u64)
            .await?;

        // Write the chunk data
        let offset = container.current_offset;
        container
            .file
            .write_all(&write_data)
            .context("Failed to write chunk data")?;
        container.file.flush().context("Failed to flush chunk data")?;

        // Update offset
        container.current_offset += data_len as u64;

        // Update container stats in database
        containers::add_chunk_to_container(&self.db_pool, container.id, data_len as i64)
            .await
            .context("Failed to update container stats")?;

        let location = ChunkLocation {
            container_id: container.id,
            offset,
            length: data_len,
            compressed,
        };

        tracing::debug!(
            "Wrote chunk {} to container {} at offset {} ({} bytes, compressed={})",
            hash, container.id, offset, data_len, compressed
        );

        Ok(location)
    }

    /// Read a chunk from storage
    pub async fn read_chunk(&self, location: &ChunkLocation) -> Result<Vec<u8>> {
        // Get container info from database
        let container = containers::get_container(&self.db_pool, location.container_id)
            .await?
            .ok_or_else(|| anyhow!("Container {} not found", location.container_id))?;

        let file_path = self.base_path.join(&container.disk_path);
        
        let mut file = std::fs::File::open(&file_path)
            .with_context(|| format!("Failed to open container file: {}", file_path.display()))?;

        // Seek to chunk offset
        file.seek(SeekFrom::Start(location.offset))
            .context("Failed to seek to chunk offset")?;

        // Read chunk data
        let mut data = vec![0u8; location.length as usize];
        file.read_exact(&mut data)
            .context("Failed to read chunk data")?;

        // Decompress if needed
        if location.compressed {
            let decompressed = zstd::decode_all(&data[..])
                .context("Zstd decompression failed")?;
            Ok(decompressed)
        } else {
            Ok(data)
        }
    }

    /// Get or create an open container for writing
    async fn get_or_create_container<'a>(
        &self,
        guard: &'a mut Option<OpenContainer>,
        required_size: u64,
    ) -> Result<&'a mut OpenContainer> {
        // Check if current container has space
        let needs_new = match guard.as_ref() {
            Some(container) => {
                container.current_offset + required_size > self.max_container_size
            }
            None => true,
        };

        if needs_new {
            // Seal current container if exists
            if let Some(old_container) = guard.take() {
                self.seal_container_internal(old_container.id).await?;
            }

            // Create new container
            let new_container = self.create_container().await?;
            *guard = Some(new_container);
        }

        Ok(guard.as_mut().unwrap())
    }

    /// Create a new container file
    async fn create_container(&self) -> Result<OpenContainer> {
        // Generate path: YYYY/MM/pack_<uuid>.blob
        let now = chrono::Utc::now();
        let year_month = now.format("%Y/%m").to_string();
        let container_id = Uuid::new_v4();
        let filename = format!("pack_{}.blob", container_id.simple());
        let relative_path = format!("{}/{}", year_month, filename);

        let full_path = self.base_path.join(&relative_path);

        // Create directory structure
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }

        // Create and initialize file with header
        let mut file = std::fs::File::create(&full_path)
            .with_context(|| format!("Failed to create container file: {}", full_path.display()))?;

        // Write header
        let header = Self::create_header();
        file.write_all(&header)
            .context("Failed to write container header")?;
        file.flush()?;

        // Create database entry
        let db_container = containers::create_container(
            &self.db_pool,
            &db::NewBlobContainer {
                disk_path: relative_path.clone(),
            },
        )
        .await
        .context("Failed to create container database entry")?;

        tracing::info!(
            "Created new container {} at {}",
            db_container.id, relative_path
        );

        Ok(OpenContainer {
            id: db_container.id,
            disk_path: full_path,
            file,
            current_offset: HEADER_SIZE,
        })
    }

    /// Create the 8-byte container header
    fn create_header() -> [u8; 8] {
        let mut header = [0u8; 8];
        header[0..4].copy_from_slice(MAGIC_BYTES);
        header[4] = FORMAT_VERSION;
        // Bytes 5-7 are reserved (already 0)
        header
    }

    /// Verify a container header
    #[allow(dead_code)]
    fn verify_header(header: &[u8; 8]) -> Result<()> {
        if &header[0..4] != MAGIC_BYTES {
            return Err(anyhow!("Invalid container magic bytes"));
        }
        if header[4] != FORMAT_VERSION {
            return Err(anyhow!(
                "Unsupported container version: {}",
                header[4]
            ));
        }
        Ok(())
    }

    /// Seal a container (mark as read-only)
    async fn seal_container_internal(&self, container_id: Uuid) -> Result<()> {
        containers::seal_container(&self.db_pool, container_id)
            .await
            .context("Failed to seal container in database")?;
        
        tracing::info!("Sealed container {}", container_id);
        Ok(())
    }

    /// Seal the current container (if any) and prepare for shutdown
    #[allow(dead_code)]
    pub async fn flush(&self) -> Result<()> {
        let mut guard = self.current_container.lock().await;
        if let Some(container) = guard.take() {
            container.file.sync_all()
                .context("Failed to sync container file")?;
            // Don't seal on normal flush - only seal when full
            *guard = Some(container);
        }
        Ok(())
    }

    /// Get the base storage path
    #[allow(dead_code)]
    pub fn base_path(&self) -> &Path {
        &self.base_path
    }

    // =========================================================================
    // LEGACY BLOB SUPPORT
    // These methods provide backwards compatibility with the old BlobStore
    // 2-char sharded storage format. New code should use write_chunk/read_chunk.
    // =========================================================================

    /// Get the legacy storage path for a blob hash (sharded by first 2 chars)
    fn legacy_blob_path(&self, hash: &str) -> Result<PathBuf> {
        if hash.len() < 4 {
            return Err(anyhow!("Invalid hash format: {}", hash));
        }
        let shard = &hash[..2];
        // Legacy blobs are stored at base_path/../ (parent of containers dir)
        let legacy_base = self.base_path.parent()
            .ok_or_else(|| anyhow!("Cannot get parent of base path"))?;
        Ok(legacy_base.join(shard).join(hash))
    }

    /// Check if a legacy blob exists
    pub fn legacy_exists(&self, hash: &str) -> Result<bool> {
        let path = self.legacy_blob_path(hash)?;
        Ok(path.exists())
    }

    /// Read a legacy blob (old BlobStore format)
    pub fn read_legacy_blob(&self, hash: &str) -> Result<Vec<u8>> {
        let path = self.legacy_blob_path(hash)?;
        
        if !path.exists() {
            return Err(anyhow!("Legacy blob not found: {}", hash));
        }
        
        let content = std::fs::read(&path)
            .with_context(|| format!("Failed to read legacy blob: {}", path.display()))?;
        
        tracing::debug!("Read legacy blob {} ({} bytes)", hash, content.len());
        Ok(content)
    }

    /// Write a legacy blob (old BlobStore format)
    /// Used for backwards compatibility with index/export commands
    pub fn write_legacy_blob(&self, hash: &str, content: &[u8]) -> Result<()> {
        let path = self.legacy_blob_path(hash)?;

        // Create shard directory if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Write atomically using temp file
        let temp_path = path.with_extension("tmp");
        {
            let mut file = std::fs::File::create(&temp_path)?;
            file.write_all(content)?;
            file.sync_all()?;
        }

        // Rename to final path (atomic on most filesystems)
        std::fs::rename(&temp_path, &path)?;

        tracing::debug!("Wrote legacy blob {} ({} bytes)", hash, content.len());
        Ok(())
    }
}

/// Write a chunk to storage and record it in the database
///
/// This is a convenience function that combines BlobManager::write_chunk
/// with the database upsert.
pub async fn store_chunk(
    blob_manager: &BlobManager,
    db_pool: &DbPool,
    hash: &str,
    data: &[u8],
    tier: ChunkTier,
) -> Result<db::Chunk> {
    // Write to physical storage
    let location = blob_manager.write_chunk(hash, data, tier).await?;

    // Record in database
    let new_chunk = NewChunk {
        hash: hash.to_string(),
        size_bytes: data.len() as i32,
        container_id: Some(location.container_id),
        offset_bytes: Some(location.offset as i64),
        length_bytes: Some(location.length as i32),
    };

    let chunk = db::chunks::upsert_chunk_with_location(db_pool, &new_chunk)
        .await
        .context("Failed to record chunk in database")?;

    Ok(chunk)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_creation() {
        let header = BlobManager::create_header();
        assert_eq!(&header[0..4], b"ENTG");
        assert_eq!(header[4], 0x01);
        assert_eq!(&header[5..8], &[0, 0, 0]);
    }

    #[test]
    fn test_header_verification() {
        let header = BlobManager::create_header();
        assert!(BlobManager::verify_header(&header).is_ok());

        let bad_magic = [0u8; 8];
        assert!(BlobManager::verify_header(&bad_magic).is_err());

        let mut bad_version = BlobManager::create_header();
        bad_version[4] = 0xFF;
        assert!(BlobManager::verify_header(&bad_version).is_err());
    }

    #[test]
    fn test_zstd_compression() {
        let data = b"Hello, world! This is some test data that should compress well. ".repeat(100);
        let compressed = zstd::encode_all(&data[..], ZSTD_COMPRESSION_LEVEL).unwrap();
        
        // Should be smaller
        assert!(compressed.len() < data.len());
        
        // Should decompress correctly
        let decompressed = zstd::decode_all(&compressed[..]).unwrap();
        assert_eq!(decompressed, data);
    }
}

