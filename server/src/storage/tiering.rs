//! Dynamic Chunking Tier System
//!
//! Selects optimal FastCDC parameters based on file size and type.
//! Used by server-side chunking (currently client handles chunking).

#![allow(dead_code)]

use std::path::Path;

/// Configuration for FastCDC chunking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkConfig {
    pub min_size: usize,
    pub avg_size: usize,
    pub max_size: usize,
}

/// Tier definitions matching the Swift client exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    T0Inline,   // < 4KB (Not chunked by FastCDC usually, but here for completeness)
    T1Granular, // 4KB - 10MB (or source code)
    T2Standard, // 10MB - 500MB (Default)
    T3Large,    // 500MB - 5GB
    T4Jumbo,    // > 5GB (or disk images)
}

impl Tier {
    pub fn config(&self) -> ChunkConfig {
        match self {
            // T0 is special: implies whole file is one chunk, 
            // but if forced through FastCDC, we'd want small params. 
            // However, logic usually bypasses CDC for T0.
            Tier::T0Inline => ChunkConfig {
                min_size: 0,
                avg_size: 0,
                max_size: 0,
            },
            Tier::T1Granular => ChunkConfig {
                min_size: 2 * 1024,      // 2 KB
                avg_size: 4 * 1024,      // 4 KB
                max_size: 8 * 1024,      // 8 KB
            },
            Tier::T2Standard => ChunkConfig {
                min_size: 16 * 1024,     // 16 KB
                avg_size: 32 * 1024,     // 32 KB
                max_size: 64 * 1024,     // 64 KB
            },
            Tier::T3Large => ChunkConfig {
                min_size: 512 * 1024,    // 512 KB
                avg_size: 1024 * 1024,   // 1 MB
                max_size: 2 * 1024 * 1024, // 2 MB
            },
            Tier::T4Jumbo => ChunkConfig {
                min_size: 4 * 1024 * 1024, // 4 MB
                avg_size: 8 * 1024 * 1024, // 8 MB
                max_size: 16 * 1024 * 1024, // 16 MB
            },
        }
    }
    
    /// Returns the tier name as a string for logging
    pub fn name(&self) -> &'static str {
        match self {
            Tier::T0Inline => "T0Inline",
            Tier::T1Granular => "T1Granular",
            Tier::T2Standard => "T2Standard",
            Tier::T3Large => "T3Large",
            Tier::T4Jumbo => "T4Jumbo",
        }
    }
}

pub trait TierStrategy {
    fn determine_tier(path: &Path, size: u64) -> Tier;
}

pub struct DefaultTierStrategy;

impl TierStrategy for DefaultTierStrategy {
    fn determine_tier(path: &Path, size: u64) -> Tier {
        // Check extension for source code or disk images
        // We use string match to keep it simple and sync with Swift
        let ext = path.extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_lowercase())
            .unwrap_or_default();
            
        // Source code extensions (T1)
        let source_exts = [
            "c", "cpp", "h", "hpp", "rs", "swift", "go", "js", "ts", "py", 
            "txt", "md", "json", "xml", "yaml", "yml", "html", "css"
        ];
        
        // Disk image extensions (T4) - these ALWAYS use T4 regardless of size
        let disk_exts = ["iso", "qcow2", "vmdk", "dmg", "img"];
        
        // Check disk images FIRST - they always use T4 regardless of size
        let tier = if disk_exts.contains(&ext.as_str()) {
            // T4: Disk Images (always, regardless of size)
            Tier::T4Jumbo
        } else if size < 4 * 1024 {
            // T0: Inline (< 4KB)
            Tier::T0Inline
        } else if size > 5 * 1024 * 1024 * 1024 {
            // T4: Jumbo (> 5GB)
            Tier::T4Jumbo
        } else if size > 500 * 1024 * 1024 {
            // T3: Large (> 500MB)
            Tier::T3Large
        } else if size < 10 * 1024 * 1024 || source_exts.contains(&ext.as_str()) {
            // T1: Granular (< 10MB OR Source Code)
            Tier::T1Granular
        } else {
            // T2: Standard (Default)
            Tier::T2Standard
        };
        
        // Log tier selection for verification
        tracing::info!(
            "TierSelector: {} -> {} (size: {} bytes, ext: {})",
            path.display(),
            tier.name(),
            size,
            if ext.is_empty() { "<none>" } else { &ext }
        );
        
        tier
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_tier_selection_by_size() {
        // T0: < 4KB
        let tier = DefaultTierStrategy::determine_tier(&PathBuf::from("small.bin"), 1024);
        assert_eq!(tier, Tier::T0Inline);
        
        // T1: < 10MB
        let tier = DefaultTierStrategy::determine_tier(&PathBuf::from("medium.bin"), 5 * 1024 * 1024);
        assert_eq!(tier, Tier::T1Granular);
        
        // T2: 10MB - 500MB
        let tier = DefaultTierStrategy::determine_tier(&PathBuf::from("large.bin"), 100 * 1024 * 1024);
        assert_eq!(tier, Tier::T2Standard);
        
        // T3: 500MB - 5GB
        let tier = DefaultTierStrategy::determine_tier(&PathBuf::from("huge.bin"), 1024 * 1024 * 1024);
        assert_eq!(tier, Tier::T3Large);
        
        // T4: > 5GB
        let tier = DefaultTierStrategy::determine_tier(&PathBuf::from("massive.bin"), 6 * 1024 * 1024 * 1024);
        assert_eq!(tier, Tier::T4Jumbo);
    }
    
    #[test]
    fn test_tier_selection_by_extension() {
        // Source code -> T1 regardless of size
        let tier = DefaultTierStrategy::determine_tier(&PathBuf::from("code.rs"), 100 * 1024 * 1024);
        assert_eq!(tier, Tier::T1Granular);

        // Disk image -> T4 regardless of size
        let tier = DefaultTierStrategy::determine_tier(&PathBuf::from("disk.iso"), 1024);
        assert_eq!(tier, Tier::T4Jumbo);

        let tier = DefaultTierStrategy::determine_tier(&PathBuf::from("vm.vmdk"), 100 * 1024 * 1024);
        assert_eq!(tier, Tier::T4Jumbo);
    }

    #[test]
    fn test_inline_tier() {
        let tier = DefaultTierStrategy::determine_tier(&PathBuf::from("tiny.bin"), 2 * 1024);
        assert_eq!(tier, Tier::T0Inline);
    }

    #[test]
    fn test_granular_tier() {
        let tier = DefaultTierStrategy::determine_tier(&PathBuf::from("file.bin"), 100 * 1024);
        assert_eq!(tier, Tier::T1Granular);
    }

    #[test]
    fn test_standard_tier() {
        let tier = DefaultTierStrategy::determine_tier(&PathBuf::from("file.bin"), 50 * 1024 * 1024);
        assert_eq!(tier, Tier::T2Standard);
    }

    #[test]
    fn test_large_tier() {
        let tier = DefaultTierStrategy::determine_tier(&PathBuf::from("file.bin"), 1024 * 1024 * 1024);
        assert_eq!(tier, Tier::T3Large);
    }

    #[test]
    fn test_jumbo_tier() {
        let tier = DefaultTierStrategy::determine_tier(&PathBuf::from("file.bin"), 10 * 1024 * 1024 * 1024);
        assert_eq!(tier, Tier::T4Jumbo);
    }

    #[test]
    fn test_source_code_gets_granular() {
        let tier = DefaultTierStrategy::determine_tier(&PathBuf::from("large.rs"), 50 * 1024 * 1024);
        assert_eq!(tier, Tier::T1Granular);
    }

    #[test]
    fn test_disk_image_gets_jumbo() {
        let tier = DefaultTierStrategy::determine_tier(&PathBuf::from("small.iso"), 100);
        assert_eq!(tier, Tier::T4Jumbo);
    }
}
