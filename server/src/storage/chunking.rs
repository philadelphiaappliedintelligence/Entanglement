//! Content Defined Chunking (CDC) using FastCDC + BLAKE3
//!
//! This module implements content-based file chunking for efficient delta sync.
//! It supports dynamic tiering based on file size/type.
//!
//! Currently unused: Client-side Swift implementation handles chunking.
//! Preserved for future server-side re-chunking or alternate client support.

#![allow(dead_code)]

use blake3;
use std::fs::File;
use std::io::{self, Read, BufReader};
use std::path::Path;
use super::tiering::{TierStrategy, DefaultTierStrategy, ChunkConfig};

/// Represents a chunk of file content
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Byte offset from start of file
    pub offset: u64,
    /// Length of this chunk in bytes
    pub length: u32,
    /// BLAKE3 hash of this chunk's content (32 bytes)
    pub hash: [u8; 32],
}

impl Chunk {
    /// Returns the hash as a lowercase hex string (64 chars)
    pub fn hash_hex(&self) -> String {
        hex::encode(self.hash)
    }
    
    /// End offset (exclusive) of this chunk
    pub fn end_offset(&self) -> u64 {
        self.offset + self.length as u64
    }
}

/// Result of chunking a file
#[derive(Debug, Clone)]
pub struct ChunkManifest {
    /// Total file size in bytes
    pub total_size: u64,
    /// BLAKE3 hash of the entire file (32 bytes)
    pub file_hash: [u8; 32],
    /// Ordered list of chunks
    pub chunks: Vec<Chunk>,
}

impl ChunkManifest {
    /// Returns the file hash as a lowercase hex string (64 chars)
    pub fn file_hash_hex(&self) -> String {
        hex::encode(self.file_hash)
    }
    
    /// Returns total number of chunks
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }
    
    /// Find chunks that differ between two manifests (for delta sync)
    pub fn diff(&self, other: &ChunkManifest) -> ChunkDiff {
        let self_hashes: std::collections::HashSet<[u8; 32]> = 
            self.chunks.iter().map(|c| c.hash).collect();
        let other_hashes: std::collections::HashSet<[u8; 32]> = 
            other.chunks.iter().map(|c| c.hash).collect();
        
        // Chunks in self but not in other (need to upload)
        let to_upload: Vec<Chunk> = self.chunks.iter()
            .filter(|c| !other_hashes.contains(&c.hash))
            .cloned()
            .collect();
        
        // Chunks in other but not in self (can reuse from server)
        let reusable: Vec<Chunk> = other.chunks.iter()
            .filter(|c| self_hashes.contains(&c.hash))
            .cloned()
            .collect();
            
        // Chunks in other that we don't have (need to download)
        let to_download: Vec<Chunk> = other.chunks.iter()
            .filter(|c| !self_hashes.contains(&c.hash))
            .cloned()
            .collect();
        
        ChunkDiff {
            to_upload,
            reusable,
            to_download,
        }
    }
}

/// Difference between two chunk manifests
#[derive(Debug, Clone)]
pub struct ChunkDiff {
    pub to_upload: Vec<Chunk>,
    pub reusable: Vec<Chunk>,
    pub to_download: Vec<Chunk>,
}

impl ChunkDiff {
    pub fn bytes_to_upload(&self) -> u64 {
        self.to_upload.iter().map(|c| c.length as u64).sum()
    }
    
    pub fn bytes_to_download(&self) -> u64 {
        self.to_download.iter().map(|c| c.length as u64).sum()
    }
    
    pub fn bytes_reusable(&self) -> u64 {
        self.reusable.iter().map(|c| c.length as u64).sum()
    }
}

/// Chunk a file using FastCDC algorithm with Dynamic Tiering
pub fn chunk_file(path: &Path) -> io::Result<ChunkManifest> {
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    let total_size = metadata.len();
    
    // Determine tier
    let tier = DefaultTierStrategy::determine_tier(path, total_size);
    let config = tier.config();
    
    // Read entire file into memory for chunking (MVP)
    // In production, we should stream this
    let mut reader = BufReader::new(file);
    let mut data = Vec::with_capacity(total_size as usize);
    reader.read_to_end(&mut data)?;
    
    chunk_data_with_config(&data, config)
}

/// Chunk raw data using FastCDC algorithm with specific config
pub fn chunk_data_with_config(data: &[u8], config: ChunkConfig) -> io::Result<ChunkManifest> {
    // Calculate full file hash using BLAKE3 (one-shot for whole file)
    let file_hash: [u8; 32] = *blake3::hash(data).as_bytes();
    
    // Handle T0 (Inline) or empty config
    if config.max_size == 0 || data.len() < config.min_size {
        // Just return one chunk for the whole file
        return Ok(ChunkManifest {
            total_size: data.len() as u64,
            file_hash,
            chunks: vec![Chunk {
                offset: 0,
                length: data.len() as u32,
                hash: file_hash,
            }],
        });
    }
    
    use fastcdc::v2020::FastCDC;
    
    // Create FastCDC chunker
    // Note: fastcdc crate uses u32 for sizes
    let chunker = FastCDC::new(
        data, 
        config.min_size as u32, 
        config.avg_size as u32, 
        config.max_size as u32
    );
    
    let mut chunks = Vec::new();
    
    for chunk_info in chunker {
        // Hash this chunk using BLAKE3 (one-shot per chunk)
        let chunk_data = &data[chunk_info.offset..chunk_info.offset + chunk_info.length];
        let chunk_hash: [u8; 32] = *blake3::hash(chunk_data).as_bytes();
        
        chunks.push(Chunk {
            offset: chunk_info.offset as u64,
            length: chunk_info.length as u32,
            hash: chunk_hash,
        });
    }
    
    Ok(ChunkManifest {
        total_size: data.len() as u64,
        file_hash,
        chunks,
    })
}

/// Legacy wrapper for compatibility (uses standard tier)
pub fn chunk_data(data: &[u8]) -> io::Result<ChunkManifest> {
    // Default to Standard tier parameters
    let config = ChunkConfig {
        min_size: 16 * 1024,
        avg_size: 32 * 1024,
        max_size: 64 * 1024,
    };
    chunk_data_with_config(data, config)
}

/// Reassemble chunks into complete file data
pub fn reassemble_chunks(chunks: &[(&[u8; 32], &[u8])], expected_size: u64) -> io::Result<Vec<u8>> {
    let mut result = Vec::with_capacity(expected_size as usize);
    
    for (expected_hash, data) in chunks {
        // Verify chunk hash using BLAKE3
        let actual_hash: [u8; 32] = *blake3::hash(data).as_bytes();
        
        if &actual_hash != *expected_hash {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Chunk hash mismatch: expected {}, got {}",
                    hex::encode(expected_hash),
                    hex::encode(actual_hash)
                ),
            ));
        }
        
        result.extend_from_slice(data);
    }
    
    if result.len() as u64 != expected_size {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "Size mismatch: expected {} bytes, got {}",
                expected_size,
                result.len()
            ),
        ));
    }
    
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blake3_hash_format() {
        let data = b"hello world";
        let hash = blake3::hash(data);
        let hex_str = hash.to_hex().to_string();
        
        // BLAKE3 produces 64-char lowercase hex
        assert_eq!(hex_str.len(), 64);
        assert!(hex_str.chars().all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()));
        
        // Known BLAKE3 hash for "hello world"
        assert_eq!(
            hex_str,
            "d74981efa70a0c880b8d8c1985d075dbcbf679b99a5f9914e5aaf96b831a9e24"
        );
    }
    
    #[test]
    fn test_chunk_hash_consistency() {
        let data = b"test chunk data for hashing";
        let hash1: [u8; 32] = *blake3::hash(data).as_bytes();
        let hash2: [u8; 32] = *blake3::hash(data).as_bytes();
        assert_eq!(hash1, hash2);
    }
}
