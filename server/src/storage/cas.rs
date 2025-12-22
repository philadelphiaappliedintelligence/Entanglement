//! Content Addressable Storage (CAS) utilities using BLAKE3
//!
//! Provides hashing functions for content-addressable storage operations.

use blake3;

/// Compute BLAKE3 hash of content, returning lowercase hex-encoded string (64 chars)
pub fn compute_hash(content: &[u8]) -> String {
    let hash = blake3::hash(content);
    hash.to_hex().to_string()
}

/// Verify that content matches expected hash
#[allow(dead_code)]
pub fn verify_hash(content: &[u8], expected_hash: &str) -> bool {
    compute_hash(content) == expected_hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_hash() {
        let content = b"hello world";
        let hash = compute_hash(content);
        
        // BLAKE3 hash for "hello world"
        assert_eq!(
            hash,
            "d74981efa70a0c880b8d8c1985d075dbcbf679b99a5f9914e5aaf96b831a9e24"
        );
        
        // Verify it's 64 chars lowercase hex
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()));
    }

    #[test]
    fn test_verify_hash() {
        let content = b"hello world";
        let hash = "d74981efa70a0c880b8d8c1985d075dbcbf679b99a5f9914e5aaf96b831a9e24";
        assert!(verify_hash(content, hash));
        assert!(!verify_hash(content, "invalid"));
    }
    
    #[test]
    fn test_empty_content() {
        let content = b"";
        let hash = compute_hash(content);
        
        // BLAKE3 hash for empty input
        assert_eq!(
            hash,
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }
}
