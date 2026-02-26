use crate::api::ApiClient;
use crate::chunking;
use crate::config::Config;
use crate::db::{FileRecord, LocalDb};
use notify::{Event, EventKind, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tracing::{error, info, warn};

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

/// Run the sync engine: initial sync then watch for changes.
pub async fn run(config: &Config) -> anyhow::Result<()> {
    let sync_dir = config
        .sync_directory
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No sync directory configured"))?;
    let sync_path = PathBuf::from(sync_dir);

    if !sync_path.exists() {
        std::fs::create_dir_all(&sync_path)?;
    }

    let db = LocalDb::open()?;
    let api = ApiClient::new(config.server_url()?);
    let token = config.auth_token()?;
    let ignore_patterns = load_ignore_patterns(&sync_path);

    // Initial sync
    info!("starting initial sync");
    sync_local_changes(&api, token, &db, &sync_path, &ignore_patterns).await?;
    sync_remote_changes(&api, token, &db, &sync_path).await?;
    process_retries(&api, token, &db, &sync_path, &ignore_patterns).await;

    // Watch for changes
    info!("watching: {}", sync_dir);
    watch_and_sync(config, &api, &db, &sync_path, &ignore_patterns).await
}

/// Walk the sync directory and upload any files that have changed since last sync.
async fn sync_local_changes(
    api: &ApiClient,
    token: &str,
    db: &LocalDb,
    root: &Path,
    ignore_patterns: &[String],
) -> anyhow::Result<()> {
    let walker = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file());

    let mut count = 0;
    for entry in walker {
        let file_path = entry.path();
        if should_ignore(file_path, root, ignore_patterns) {
            continue;
        }

        if let Err(e) = upload_if_changed(api, token, db, root, file_path).await {
            warn!("sync failed {}: {}", file_path.display(), e);
            let remote_path = to_remote_path(root, file_path);
            let _ = db.add_retry(&remote_path, &e.to_string());
        } else {
            count += 1;
        }
    }

    if count > 0 {
        info!("synced {} local files", count);
    }
    Ok(())
}

/// Hash file, compare with DB, upload if changed.
async fn upload_if_changed(
    api: &ApiClient,
    token: &str,
    db: &LocalDb,
    root: &Path,
    file_path: &Path,
) -> anyhow::Result<()> {
    let data = std::fs::read(file_path)?;
    let hash = chunking::hash_file(&data);
    let remote_path = to_remote_path(root, file_path);

    // Skip if unchanged
    if let Some(record) = db.get_file(&remote_path)? {
        if record.blake3_hash == hash {
            return Ok(());
        }
    }

    info!("uploading: {}", remote_path);
    upload_file(api, token, file_path, &remote_path, &data, &hash).await?;

    let mtime = std::fs::metadata(file_path)?
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs() as i64;

    db.upsert_file(&FileRecord {
        path: remote_path.clone(),
        blake3_hash: hash,
        last_modified: mtime,
        sync_cursor: None,
    })?;
    let _ = db.clear_retry(&remote_path);

    Ok(())
}

/// Chunk a file, upload missing chunks to server, then create the file record.
async fn upload_file(
    api: &ApiClient,
    token: &str,
    file_path: &Path,
    remote_path: &str,
    data: &[u8],
    content_hash: &str,
) -> anyhow::Result<()> {
    let chunks = chunking::chunk_file(file_path, data);
    let tier = chunking::select_tier(file_path, data.len() as u64);

    // Check which chunks already exist on server
    let chunk_hashes: Vec<String> = chunks.iter().map(|c| c.hash.clone()).collect();
    let check = api.check_chunks(token, &chunk_hashes).await?;

    // Upload only missing chunks
    for chunk in &chunks {
        if check.missing.contains(&chunk.hash) {
            api.upload_chunk(token, &chunk.hash, &chunk.data, tier.id())
                .await?;
        }
    }

    // Create file record from chunks
    let modified_at = chrono::Utc::now().to_rfc3339();
    api.create_file(
        token,
        remote_path,
        data.len() as i64,
        &modified_at,
        tier.id(),
        content_hash,
        chunk_hashes,
    )
    .await?;

    Ok(())
}

/// Poll server for remote changes and download new/modified files.
async fn sync_remote_changes(
    api: &ApiClient,
    token: &str,
    db: &LocalDb,
    root: &Path,
) -> anyhow::Result<()> {
    let since = db.get_last_sync_time()?;
    let resp = api.get_changes(token, since.as_deref()).await?;

    let mut count = 0;
    for change in &resp.changes {
        if change.is_directory {
            continue;
        }

        let local_path = root.join(change.path.trim_start_matches('/'));

        match change.action.as_str() {
            "created" | "modified" => {
                // Skip if we already have this version
                if let Some(record) = db.get_file(&change.path)? {
                    if change.blob_hash.as_deref() == Some(&record.blake3_hash) {
                        continue;
                    }
                }

                match download_remote_file(api, token, db, &change.path, change.id, &local_path)
                    .await
                {
                    Ok(_) => count += 1,
                    Err(e) => warn!("download failed {}: {}", change.path, e),
                }
            }
            "deleted" => {
                if local_path.exists() {
                    info!("remote deleted: {}", change.path);
                    let _ = std::fs::remove_file(&local_path);
                }
                let _ = db.remove_file(&change.path);
                count += 1;
            }
            _ => {}
        }
    }

    db.set_last_sync_time(&resp.server_time)?;

    if count > 0 {
        info!("applied {} remote changes", count);
    }
    Ok(())
}

/// Download a file from the server and write it locally.
async fn download_remote_file(
    api: &ApiClient,
    token: &str,
    db: &LocalDb,
    remote_path: &str,
    file_id: uuid::Uuid,
    local_path: &Path,
) -> anyhow::Result<()> {
    let versions = api.get_file_versions(token, file_id).await?;
    let latest = versions
        .first()
        .ok_or_else(|| anyhow::anyhow!("No versions for {}", remote_path))?;

    info!("downloading: {}", remote_path);
    let data = api.download_file(token, latest.id).await?;

    if let Some(parent) = local_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(local_path, &data)?;

    let hash = chunking::hash_file(&data);
    let mtime = std::fs::metadata(local_path)?
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs() as i64;

    db.upsert_file(&FileRecord {
        path: remote_path.to_string(),
        blake3_hash: hash,
        last_modified: mtime,
        sync_cursor: None,
    })?;

    Ok(())
}

/// Retry previously failed uploads.
async fn process_retries(
    api: &ApiClient,
    token: &str,
    db: &LocalDb,
    root: &Path,
    ignore_patterns: &[String],
) {
    let retries = match db.get_pending_retries() {
        Ok(r) => r,
        Err(_) => return,
    };

    for retry in retries {
        let local_path = root.join(retry.path.trim_start_matches('/'));
        if local_path.exists() && !should_ignore(&local_path, root, ignore_patterns) {
            match upload_if_changed(api, token, db, root, &local_path).await {
                Ok(_) => info!("retry succeeded: {}", retry.path),
                Err(e) => warn!(
                    "retry failed (attempt {}): {}: {}",
                    retry.attempts + 1,
                    retry.path,
                    e
                ),
            }
        }
    }
}

/// Watch directory for filesystem events and sync changes.
async fn watch_and_sync(
    config: &Config,
    api: &ApiClient,
    db: &LocalDb,
    root: &Path,
    ignore_patterns: &[String],
) -> anyhow::Result<()> {
    let (tx, rx) = std::sync::mpsc::channel();

    let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
        if let Ok(event) = res {
            let _ = tx.send(event);
        }
    })?;

    watcher.watch(root, RecursiveMode::Recursive)?;

    let token = config.auth_token()?;
    let mut pending_paths: HashSet<PathBuf> = HashSet::new();
    let mut last_event = Instant::now();
    let mut last_poll = Instant::now();
    let debounce = Duration::from_millis(500);
    let poll_interval = Duration::from_secs(30);

    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(event) => {
                for path in event.paths {
                    if should_ignore(&path, root, ignore_patterns) {
                        continue;
                    }
                    match event.kind {
                        EventKind::Create(_) | EventKind::Modify(_) => {
                            if path.is_file() {
                                pending_paths.insert(path);
                            }
                        }
                        EventKind::Remove(_) => {
                            let remote = to_remote_path(root, &path);
                            info!("deleted: {}", remote);
                            let _ = db.remove_file(&remote);
                        }
                        _ => {}
                    }
                }
                last_event = Instant::now();
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // Process pending local changes after debounce
                if !pending_paths.is_empty() && last_event.elapsed() >= debounce {
                    for path in pending_paths.drain() {
                        if path.exists() && path.is_file() {
                            if let Err(e) = upload_if_changed(api, token, db, root, &path).await {
                                let remote = to_remote_path(root, &path);
                                error!("sync failed {}: {}", remote, e);
                                let _ = db.add_retry(&remote, &e.to_string());
                            }
                        }
                    }
                }

                // Periodically poll for remote changes
                if last_poll.elapsed() >= poll_interval {
                    if let Err(e) = sync_remote_changes(api, token, db, root).await {
                        warn!("remote sync poll failed: {}", e);
                    }
                    process_retries(api, token, db, root, ignore_patterns).await;
                    last_poll = Instant::now();
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    Ok(())
}

/// Convert a local filesystem path to a remote path (relative to sync root).
fn to_remote_path(root: &Path, local_path: &Path) -> String {
    let relative = local_path.strip_prefix(root).unwrap_or(local_path);
    format!("/{}", relative.to_string_lossy().replace('\\', "/"))
}

/// Load ignore patterns from .entanglementignore + defaults.
pub fn load_ignore_patterns(root: &Path) -> Vec<String> {
    let mut patterns: Vec<String> = DEFAULT_IGNORE_PATTERNS
        .iter()
        .map(|s| s.to_string())
        .collect();

    let ignore_file = root.join(".entanglementignore");
    if let Ok(content) = std::fs::read_to_string(&ignore_file) {
        for line in content.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() && !trimmed.starts_with('#') {
                patterns.push(trimmed.to_string());
            }
        }
    }

    patterns
}

/// Check if a path matches any ignore pattern.
pub fn should_ignore(path: &Path, root: &Path, patterns: &[String]) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let relative_str = relative.to_string_lossy();
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

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

/// Simple glob pattern matching (supports * and ?).
fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern_bytes = pattern.as_bytes();
    let text_bytes = text.as_bytes();

    let mut pi = 0;
    let mut ti = 0;
    let mut star_pi: Option<usize> = None;
    let mut star_ti: Option<usize> = None;

    while ti < text_bytes.len() {
        if pi < pattern_bytes.len()
            && (pattern_bytes[pi] == b'?' || pattern_bytes[pi] == text_bytes[ti])
        {
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

    while pi < pattern_bytes.len() && pattern_bytes[pi] == b'*' {
        pi += 1;
    }

    pi == pattern_bytes.len()
}
