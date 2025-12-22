use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum BlobError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Blob not found: {0}")]
    NotFound(String),
    #[error("Invalid hash format: {0}")]
    InvalidHash(String),
}

/// Content-addressable blob storage with directory sharding
pub struct BlobStore {
    base_path: PathBuf,
}

impl BlobStore {
    /// Create a new blob store at the given path
    pub fn new<P: AsRef<Path>>(base_path: P) -> Result<Self, BlobError> {
        let base_path = base_path.as_ref().to_path_buf();
        fs::create_dir_all(&base_path)?;
        Ok(Self { base_path })
    }

    /// Get the storage path for a blob hash (sharded by first 2 chars)
    fn blob_path(&self, hash: &str) -> Result<PathBuf, BlobError> {
        if hash.len() < 4 {
            return Err(BlobError::InvalidHash(hash.to_string()));
        }
        let shard = &hash[..2];
        Ok(self.base_path.join(shard).join(hash))
    }

    /// Check if a blob exists
    pub fn exists(&self, hash: &str) -> Result<bool, BlobError> {
        let path = self.blob_path(hash)?;
        Ok(path.exists())
    }

    /// Write a blob to storage
    pub fn write(&self, hash: &str, content: &[u8]) -> Result<(), BlobError> {
        let path = self.blob_path(hash)?;

        // Create shard directory if needed
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Write atomically using temp file
        let temp_path = path.with_extension("tmp");
        {
            let mut file = File::create(&temp_path)?;
            file.write_all(content)?;
            file.sync_all()?;
        }

        // Rename to final path (atomic on most filesystems)
        fs::rename(&temp_path, &path)?;

        tracing::debug!("Wrote blob {} ({} bytes)", hash, content.len());
        Ok(())
    }

    /// Read a blob from storage
    pub fn read(&self, hash: &str) -> Result<Vec<u8>, BlobError> {
        let path = self.blob_path(hash)?;

        if !path.exists() {
            return Err(BlobError::NotFound(hash.to_string()));
        }

        let mut file = File::open(&path)?;
        let mut content = Vec::new();
        file.read_to_end(&mut content)?;

        tracing::debug!("Read blob {} ({} bytes)", hash, content.len());
        Ok(content)
    }

    /// Delete a blob from storage
    #[allow(dead_code)]
    pub fn delete(&self, hash: &str) -> Result<(), BlobError> {
        let path = self.blob_path(hash)?;

        if path.exists() {
            fs::remove_file(&path)?;
            tracing::debug!("Deleted blob {}", hash);
        }

        Ok(())
    }

    /// Get total size of all blobs in storage
    #[allow(dead_code)]
    pub fn total_size(&self) -> Result<u64, BlobError> {
        let mut total = 0u64;

        for entry in fs::read_dir(&self.base_path)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                for file_entry in fs::read_dir(entry.path())? {
                    let file_entry = file_entry?;
                    if file_entry.file_type()?.is_file() {
                        total += file_entry.metadata()?.len();
                    }
                }
            }
        }

        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_blob_store() {
        let temp = tempdir().unwrap();
        let store = BlobStore::new(temp.path()).unwrap();

        let hash = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
        let content = b"hello world";

        // Initially doesn't exist
        assert!(!store.exists(hash).unwrap());

        // Write blob
        store.write(hash, content).unwrap();
        assert!(store.exists(hash).unwrap());

        // Read blob
        let read_content = store.read(hash).unwrap();
        assert_eq!(read_content, content);

        // Delete blob
        store.delete(hash).unwrap();
        assert!(!store.exists(hash).unwrap());
    }
}

