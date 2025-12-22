//! Database models for Entanglement File Sync
//!
//! These structs map directly to the database schema and support
//! BLAKE3 hashing and Dynamic Chunking Tiers.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// =============================================================================
// Tier Enum
// =============================================================================

/// Dynamic Chunking Tier
/// Matches the `tier_id` column in the database.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[repr(i16)]
pub enum ChunkTier {
    /// Tier 0: Inline (< 4KB) - No chunking, whole file
    Inline = 0,
    /// Tier 1: Granular (4KB-10MB or source code) - 2KB/4KB/8KB chunks
    Granular = 1,
    /// Tier 2: Standard (10MB-500MB) - 16KB/32KB/64KB chunks
    Standard = 2,
    /// Tier 3: Large (500MB-5GB) - 512KB/1MB/2MB chunks
    Large = 3,
    /// Tier 4: Jumbo (>5GB or disk images) - 4MB/8MB/16MB chunks
    Jumbo = 4,
}

impl ChunkTier {
    /// Convert from database smallint
    pub fn from_i16(value: i16) -> Option<Self> {
        match value {
            0 => Some(ChunkTier::Inline),
            1 => Some(ChunkTier::Granular),
            2 => Some(ChunkTier::Standard),
            3 => Some(ChunkTier::Large),
            4 => Some(ChunkTier::Jumbo),
            _ => None,
        }
    }
    
    /// Get the tier name
    pub fn name(&self) -> &'static str {
        match self {
            ChunkTier::Inline => "Inline",
            ChunkTier::Granular => "Granular",
            ChunkTier::Standard => "Standard",
            ChunkTier::Large => "Large",
            ChunkTier::Jumbo => "Jumbo",
        }
    }
    
    /// Get FastCDC parameters (min, avg, max) in bytes
    pub fn chunk_sizes(&self) -> (usize, usize, usize) {
        match self {
            ChunkTier::Inline => (0, 0, 0),
            ChunkTier::Granular => (2 * 1024, 4 * 1024, 8 * 1024),
            ChunkTier::Standard => (16 * 1024, 32 * 1024, 64 * 1024),
            ChunkTier::Large => (512 * 1024, 1024 * 1024, 2 * 1024 * 1024),
            ChunkTier::Jumbo => (4 * 1024 * 1024, 8 * 1024 * 1024, 16 * 1024 * 1024),
        }
    }
}

impl Default for ChunkTier {
    fn default() -> Self {
        ChunkTier::Standard
    }
}

// =============================================================================
// Blob Container
// =============================================================================

/// A blob container (pack file) that stores multiple chunks.
/// Enables efficient disk I/O by batching small chunks together.
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct BlobContainer {
    pub id: Uuid,
    /// Path on disk relative to blob storage root (e.g., "2024/05/pack_abc.blob")
    pub disk_path: String,
    /// Current total size of all chunks in this container
    pub total_size: i64,
    /// Number of chunks stored in this container
    pub chunk_count: i32,
    /// Whether this container is sealed (read-only, no more appends)
    pub is_sealed: bool,
    pub created_at: DateTime<Utc>,
    /// When the container was sealed (NULL if still open)
    pub sealed_at: Option<DateTime<Utc>>,
}

/// Input for creating a new blob container
#[derive(Debug, Clone)]
pub struct NewBlobContainer {
    pub disk_path: String,
}

// =============================================================================
// Chunk
// =============================================================================

/// A content-addressed chunk identified by its BLAKE3 hash.
/// Chunks are stored either in a blob container or as standalone files.
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct Chunk {
    /// BLAKE3 hash of the chunk content (64-char hex, primary key)
    pub hash: String,
    /// Size of chunk content in bytes
    pub size_bytes: i32,
    /// Reference count (number of versions using this chunk)
    pub ref_count: i32,
    /// Container storing this chunk (NULL = standalone blob file)
    pub container_id: Option<Uuid>,
    /// Byte offset within the container
    pub offset_bytes: Option<i64>,
    /// Length of data in container (should equal size_bytes)
    pub length_bytes: Option<i32>,
    pub created_at: DateTime<Utc>,
}

impl Chunk {
    /// Check if this chunk is stored in a container
    pub fn is_containerized(&self) -> bool {
        self.container_id.is_some()
    }
    
    /// Get the storage location info
    pub fn location(&self) -> ChunkLocation {
        match (self.container_id, self.offset_bytes, self.length_bytes) {
            (Some(container_id), Some(offset), Some(length)) => {
                ChunkLocation::Container {
                    container_id,
                    offset,
                    length,
                }
            }
            _ => ChunkLocation::Standalone { hash: self.hash.clone() },
        }
    }
}

/// Where a chunk's data is physically stored
#[derive(Debug, Clone)]
pub enum ChunkLocation {
    /// Stored as a standalone blob file (legacy or large chunks)
    Standalone { hash: String },
    /// Stored inside a blob container
    Container {
        container_id: Uuid,
        offset: i64,
        length: i32,
    },
}

/// Input for creating a new chunk
#[derive(Debug, Clone)]
pub struct NewChunk {
    /// BLAKE3 hash (64-char hex)
    pub hash: String,
    /// Size in bytes
    pub size_bytes: i32,
    /// Optional container location
    pub container_id: Option<Uuid>,
    pub offset_bytes: Option<i64>,
    pub length_bytes: Option<i32>,
}

// =============================================================================
// File Version
// =============================================================================

/// A version represents a specific state of a file's content.
/// This is the "real file node" in our version-centric model.
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct FileVersion {
    pub id: Uuid,
    pub file_id: Uuid,
    /// Legacy content hash (may be SHA-256 for old versions)
    pub blob_hash: String,
    /// BLAKE3 hash of complete file content (64-char hex)
    pub blake3_hash: Option<String>,
    /// Total file size in bytes
    pub size_bytes: i64,
    /// Dynamic chunking tier used for this version
    pub tier_id: i16,
    /// Whether this version uses chunked storage
    pub is_chunked: bool,
    pub created_at: DateTime<Utc>,
    /// User who created this version (NULL for system-indexed files)
    pub created_by: Option<Uuid>,
}

impl FileVersion {
    /// Get the tier enum
    pub fn tier(&self) -> ChunkTier {
        ChunkTier::from_i16(self.tier_id).unwrap_or_default()
    }
    
    /// Get the content hash (prefers blake3_hash, falls back to blob_hash)
    pub fn content_hash(&self) -> &str {
        self.blake3_hash.as_deref().unwrap_or(&self.blob_hash)
    }
}

/// Input for creating a new file version
#[derive(Debug, Clone)]
pub struct NewFileVersion {
    pub file_id: Uuid,
    /// BLAKE3 hash of the complete file
    pub blake3_hash: String,
    pub size_bytes: i64,
    pub tier: ChunkTier,
    pub is_chunked: bool,
    pub created_by: Option<Uuid>,
}

// =============================================================================
// Version Chunk Mapping
// =============================================================================

/// Maps a version to its ordered chunks (the "chunk manifest")
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct VersionChunk {
    pub id: Uuid,
    pub version_id: Uuid,
    /// BLAKE3 hash of the chunk
    pub chunk_hash: String,
    /// Position in the file (0-indexed)
    pub chunk_index: i32,
    /// Byte offset in the complete file
    pub chunk_offset: i64,
}

/// Input for adding a chunk to a version
#[derive(Debug, Clone)]
pub struct NewVersionChunk {
    pub version_id: Uuid,
    pub chunk_hash: String,
    pub chunk_index: i32,
    pub chunk_offset: i64,
}

// =============================================================================
// File
// =============================================================================

/// A file in the sync system (identified by path)
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct File {
    pub id: Uuid,
    /// Virtual path (e.g., "/documents/report.pdf")
    pub path: String,
    /// Current version (latest)
    pub current_version_id: Option<Uuid>,
    /// Soft delete flag
    pub is_deleted: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// File with its current version info (for listings)
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct FileWithVersion {
    pub id: Uuid,
    pub path: String,
    pub current_version_id: Option<Uuid>,
    pub is_deleted: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    // From joined version
    pub size_bytes: Option<i64>,
    pub blob_hash: Option<String>,
    pub blake3_hash: Option<String>,
    pub tier_id: Option<i16>,
}

impl FileWithVersion {
    /// Get the content hash (prefers blake3_hash)
    pub fn content_hash(&self) -> Option<&str> {
        self.blake3_hash.as_deref().or(self.blob_hash.as_deref())
    }
    
    /// Get the tier if available
    pub fn tier(&self) -> Option<ChunkTier> {
        self.tier_id.and_then(ChunkTier::from_i16)
    }
}

// =============================================================================
// Tier Config (Reference Data)
// =============================================================================

/// Tier configuration from the database
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct TierConfig {
    pub tier_id: i16,
    pub name: String,
    pub min_chunk_bytes: i32,
    pub avg_chunk_bytes: i32,
    pub max_chunk_bytes: i32,
}








