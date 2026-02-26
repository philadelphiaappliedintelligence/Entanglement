use std::path::Path;

/// A content-addressed chunk produced by FastCDC + BLAKE3.
#[derive(Debug, Clone)]
pub struct Chunk {
    pub hash: String,
    pub data: Vec<u8>,
    pub offset: u64,
    pub length: u32,
}

/// FastCDC parameters for a given tier.
#[derive(Debug, Clone, Copy)]
pub struct ChunkConfig {
    pub min_size: u32,
    pub avg_size: u32,
    pub max_size: u32,
}

/// Chunking tiers matching the server's tiering.rs exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Inline,   // T0: < 4KB (no chunking)
    Granular, // T1: 4KB - 10MB (or source code)
    Standard, // T2: 10MB - 500MB
    Large,    // T3: 500MB - 5GB
    Jumbo,    // T4: > 5GB (or disk images)
}

impl Tier {
    pub fn id(&self) -> u8 {
        match self {
            Tier::Inline => 0,
            Tier::Granular => 1,
            Tier::Standard => 2,
            Tier::Large => 3,
            Tier::Jumbo => 4,
        }
    }

    /// Chunk size parameters matching server/src/storage/tiering.rs.
    pub fn config(&self) -> ChunkConfig {
        match self {
            Tier::Inline => ChunkConfig {
                min_size: 0,
                avg_size: 0,
                max_size: 0,
            },
            Tier::Granular => ChunkConfig {
                min_size: 2 * 1024,        // 2 KB
                avg_size: 4 * 1024,        // 4 KB
                max_size: 8 * 1024,        // 8 KB
            },
            Tier::Standard => ChunkConfig {
                min_size: 16 * 1024,       // 16 KB
                avg_size: 32 * 1024,       // 32 KB
                max_size: 64 * 1024,       // 64 KB
            },
            Tier::Large => ChunkConfig {
                min_size: 512 * 1024,      // 512 KB
                avg_size: 1024 * 1024,     // 1 MB
                max_size: 2 * 1024 * 1024, // 2 MB
            },
            Tier::Jumbo => ChunkConfig {
                min_size: 4 * 1024 * 1024,  // 4 MB
                avg_size: 8 * 1024 * 1024,  // 8 MB
                max_size: 16 * 1024 * 1024, // 16 MB
            },
        }
    }
}

/// Source code extensions that always use T1 Granular tier.
const SOURCE_EXTS: &[&str] = &[
    "c", "cpp", "h", "hpp", "rs", "swift", "go", "js", "ts", "py",
    "txt", "md", "json", "xml", "yaml", "yml", "html", "css",
];

/// Disk image extensions that always use T4 Jumbo tier.
const DISK_EXTS: &[&str] = &["iso", "qcow2", "vmdk", "dmg", "img"];

/// Select chunking tier based on file size and extension.
/// Mirrors the server's DefaultTierStrategy::determine_tier exactly.
pub fn select_tier(path: &Path, size: u64) -> Tier {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_lowercase())
        .unwrap_or_default();

    // Disk images always use T4 regardless of size
    if DISK_EXTS.contains(&ext.as_str()) {
        return Tier::Jumbo;
    }

    if size < 4 * 1024 {
        Tier::Inline
    } else if size > 5 * 1024 * 1024 * 1024 {
        Tier::Jumbo
    } else if size > 500 * 1024 * 1024 {
        Tier::Large
    } else if size < 10 * 1024 * 1024 || SOURCE_EXTS.contains(&ext.as_str()) {
        Tier::Granular
    } else {
        Tier::Standard
    }
}

/// Compute BLAKE3 hash of data, returned as hex string.
pub fn hash_file(data: &[u8]) -> String {
    blake3::hash(data).to_hex().to_string()
}

/// Chunk file data using FastCDC with tier-appropriate parameters.
/// Each chunk is hashed with BLAKE3 for content addressing.
pub fn chunk_file(path: &Path, data: &[u8]) -> Vec<Chunk> {
    let tier = select_tier(path, data.len() as u64);

    // Inline files: single chunk, no CDC
    if tier == Tier::Inline {
        let hash = blake3::hash(data).to_hex().to_string();
        return vec![Chunk {
            hash,
            data: data.to_vec(),
            offset: 0,
            length: data.len() as u32,
        }];
    }

    let config = tier.config();
    let chunker = fastcdc::v2020::FastCDC::new(data, config.min_size, config.avg_size, config.max_size);

    chunker
        .map(|c| {
            let chunk_data = &data[c.offset..c.offset + c.length];
            let hash = blake3::hash(chunk_data).to_hex().to_string();
            Chunk {
                hash,
                data: chunk_data.to_vec(),
                offset: c.offset as u64,
                length: c.length as u32,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_tier_selection_by_size() {
        assert_eq!(select_tier(&PathBuf::from("small.bin"), 1024), Tier::Inline);
        assert_eq!(select_tier(&PathBuf::from("med.bin"), 5 * 1024 * 1024), Tier::Granular);
        assert_eq!(select_tier(&PathBuf::from("large.bin"), 100 * 1024 * 1024), Tier::Standard);
        assert_eq!(select_tier(&PathBuf::from("huge.bin"), 1024 * 1024 * 1024), Tier::Large);
        assert_eq!(select_tier(&PathBuf::from("massive.bin"), 6 * 1024 * 1024 * 1024), Tier::Jumbo);
    }

    #[test]
    fn test_tier_selection_by_extension() {
        // Source code -> Granular regardless of size
        assert_eq!(select_tier(&PathBuf::from("code.rs"), 100 * 1024 * 1024), Tier::Granular);
        // Disk image -> Jumbo regardless of size
        assert_eq!(select_tier(&PathBuf::from("disk.iso"), 1024), Tier::Jumbo);
        assert_eq!(select_tier(&PathBuf::from("vm.vmdk"), 100 * 1024 * 1024), Tier::Jumbo);
    }

    #[test]
    fn test_blake3_hash() {
        let data = b"hello world";
        let hash = hash_file(data);
        // Verify against known BLAKE3 hash
        let expected = blake3::hash(data).to_hex().to_string();
        assert_eq!(hash, expected);
        assert_eq!(hash.len(), 64); // 256-bit = 64 hex chars
    }

    #[test]
    fn test_inline_no_chunking() {
        // File < 4KB should produce single chunk with no CDC splitting
        let data = vec![42u8; 100];
        let chunks = chunk_file(&PathBuf::from("tiny.bin"), &data);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].offset, 0);
        assert_eq!(chunks[0].length, 100);
        assert_eq!(chunks[0].data, data);
    }

    #[test]
    fn test_chunks_cover_entire_file() {
        // Generate data large enough for multiple chunks (Granular tier)
        let data: Vec<u8> = (0..50_000u32).map(|i| (i % 256) as u8).collect();
        let chunks = chunk_file(&PathBuf::from("medium.bin"), &data);

        assert!(!chunks.is_empty());

        // Verify chunks cover entire file with no gaps or overlaps
        let mut expected_offset: u64 = 0;
        for chunk in &chunks {
            assert_eq!(chunk.offset, expected_offset, "gap or overlap at offset {}", expected_offset);
            assert!(chunk.length > 0, "zero-length chunk at offset {}", expected_offset);
            assert_eq!(chunk.data.len(), chunk.length as usize);
            // Verify chunk data matches source
            let start = chunk.offset as usize;
            let end = start + chunk.length as usize;
            assert_eq!(&chunk.data[..], &data[start..end]);
            expected_offset += chunk.length as u64;
        }
        assert_eq!(expected_offset, data.len() as u64, "chunks don't cover entire file");
    }

    #[test]
    fn test_deterministic_chunking() {
        let data: Vec<u8> = (0..20_000u32).map(|i| (i % 256) as u8).collect();
        let path = PathBuf::from("repeat.bin");

        let chunks_a = chunk_file(&path, &data);
        let chunks_b = chunk_file(&path, &data);

        assert_eq!(chunks_a.len(), chunks_b.len());
        for (a, b) in chunks_a.iter().zip(chunks_b.iter()) {
            assert_eq!(a.hash, b.hash);
            assert_eq!(a.offset, b.offset);
            assert_eq!(a.length, b.length);
        }
    }

    #[test]
    fn test_empty_file() {
        let data: Vec<u8> = vec![];
        let chunks = chunk_file(&PathBuf::from("empty.bin"), &data);
        // Empty file is inline tier, should produce one chunk with empty data
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].length, 0);
        assert!(chunks[0].data.is_empty());
    }
}
