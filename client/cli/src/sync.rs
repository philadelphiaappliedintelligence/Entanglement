use crate::api::GrpcClient;
use crate::config::Config;
use std::fs;
use std::path::Path;

/// Default ignore patterns (always applied)
const DEFAULT_IGNORE_PATTERNS: &[&str] = &[
    ".DS_Store",
    ".Spotlight-V100",
    ".Trashes",
    "._*",
    "Thumbs.db",
    "desktop.ini",
    ".git",
    ".git/",
    "node_modules/",
    ".entanglement",
];

/// Load ignore patterns from .entanglementignore file
pub fn load_ignore_patterns(root: &Path) -> Vec<String> {
    let mut patterns: Vec<String> = DEFAULT_IGNORE_PATTERNS.iter().map(|s| s.to_string()).collect();
    
    let ignore_file = root.join(".entanglementignore");
    if let Ok(content) = fs::read_to_string(&ignore_file) {
        for line in content.lines() {
            let trimmed = line.trim();
            // Skip empty lines and comments
            if !trimmed.is_empty() && !trimmed.starts_with('#') {
                patterns.push(trimmed.to_string());
            }
        }
        println!("  loaded {} patterns from .entanglementignore", 
            content.lines().filter(|l| !l.trim().is_empty() && !l.trim().starts_with('#')).count());
    }
    
    patterns
}

/// Check if a path matches any ignore pattern
pub fn should_ignore(path: &Path, root: &Path, patterns: &[String]) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let relative_str = relative.to_string_lossy();
    let filename = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    
    for pattern in patterns {
        // Directory pattern (ends with /)
        if pattern.ends_with('/') {
            let dir_name = &pattern[..pattern.len() - 1];
            for component in relative.components() {
                if component.as_os_str().to_string_lossy() == dir_name {
                    return true;
                }
            }
        }
        // Wildcard pattern
        else if pattern.contains('*') {
            if glob_match(pattern, &filename) || glob_match(pattern, &relative_str) {
                return true;
            }
        }
        // Exact match
        else if filename == *pattern || relative_str == *pattern {
            return true;
        }
    }
    
    false
}

/// Simple glob pattern matching (supports * and ?)
fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern_bytes = pattern.as_bytes();
    let text_bytes = text.as_bytes();
    
    let mut pi = 0;
    let mut ti = 0;
    let mut star_pi: Option<usize> = None;
    let mut star_ti: Option<usize> = None;
    
    while ti < text_bytes.len() {
        if pi < pattern_bytes.len() && (pattern_bytes[pi] == b'?' || pattern_bytes[pi] == text_bytes[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < pattern_bytes.len() && pattern_bytes[pi] == b'*' {
            star_pi = Some(pi);
            star_ti = Some(ti);
            pi += 1;
        } else if let (Some(sp), Some(st)) = (star_pi, star_ti) {
            pi = sp + 1;
            star_ti = Some(st + 1);
            ti = st + 1;
        } else {
            return false;
        }
    }
    
    // Check remaining pattern chars are all *
    while pi < pattern_bytes.len() && pattern_bytes[pi] == b'*' {
        pi += 1;
    }
    
    pi == pattern_bytes.len()
}

/// Sync a directory with the server (push local changes)
pub async fn sync_directory(config: &Config, path: &Path) -> anyhow::Result<()> {
    let mut client = GrpcClient::connect(config).await?;
    let ignore_patterns = load_ignore_patterns(path);
    
    let walker = walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file());

    let mut count = 0;
    let mut skipped = 0;
    for entry in walker {
        let file_path = entry.path();
        
        // Skip ignored files
        if should_ignore(file_path, path, &ignore_patterns) {
            skipped += 1;
            continue;
        }
        
        let remote_path = compute_remote_path(config, path, file_path)?;
        println!("  push: {}", remote_path);
        
        if let Err(e) = client.push_file(file_path, &remote_path).await {
            eprintln!("  ! error: {}", e);
        } else {
            count += 1;
        }
    }
    
    if skipped > 0 {
        println!("  skipped {} ignored files", skipped);
    }
    if count > 0 {
        println!("  synced {} files", count);
    }
    Ok(())
}

fn compute_remote_path(config: &Config, root: &Path, local_path: &Path) -> anyhow::Result<String> {
    // Try to get relative path from the sync root
    if let Ok(rel) = local_path.strip_prefix(root) {
        return Ok(format!("/{}", rel.to_string_lossy().replace('\\', "/")));
    }
    
    // Try sync_root from config
    if let Some(sync_root) = &config.sync_root {
        let sync_root = std::path::Path::new(sync_root);
        if let Ok(rel) = local_path.strip_prefix(sync_root) {
            return Ok(format!("/{}", rel.to_string_lossy().replace('\\', "/")));
        }
    }
    
    // Fall back to filename only
    let filename = local_path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    Ok(format!("/{}", filename))
}
