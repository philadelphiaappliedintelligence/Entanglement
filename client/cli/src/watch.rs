use crate::api::GrpcClient;
use crate::config::Config;
use crate::sync::{load_ignore_patterns, should_ignore};
use notify::{Event, EventKind, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::Path;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

/// Start watching a directory for changes and sync them
pub async fn start_watching(config: &Config, path: &Path) -> anyhow::Result<()> {
    let (tx, rx) = channel();
    
    let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
        if let Ok(event) = res {
            let _ = tx.send(event);
        }
    })?;

    watcher.watch(path, RecursiveMode::Recursive)?;

    let mut client = GrpcClient::connect(config).await?;
    let config = config.clone();
    let ignore_patterns = load_ignore_patterns(path);
    
    // Debounce: collect events for a short period before processing
    let mut pending_paths: HashSet<std::path::PathBuf> = HashSet::new();
    let mut last_event_time = Instant::now();
    let debounce_duration = Duration::from_millis(500);

    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(event) => {
                // Collect paths from event
                for event_path in event.paths {
                    // Skip ignored files
                    if should_ignore(&event_path, path, &ignore_patterns) {
                        continue;
                    }
                    
                    match event.kind {
                        EventKind::Create(_) | EventKind::Modify(_) => {
                            if event_path.is_file() {
                                pending_paths.insert(event_path);
                            }
                        }
                        EventKind::Remove(_) => {
                            // Handle deletes immediately
                            let remote_path = compute_remote_path(&config, path, &event_path)?;
                            println!("  delete: {}", remote_path);
                            if let Err(e) = client.delete_file(&remote_path).await {
                                eprintln!("  ! {}", e);
                            }
                        }
                        _ => {}
                    }
                }
                last_event_time = Instant::now();
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // Process pending paths if debounce period has passed
                if !pending_paths.is_empty() && last_event_time.elapsed() >= debounce_duration {
                    for file_path in pending_paths.drain() {
                        if file_path.exists() && file_path.is_file() {
                            let remote_path = compute_remote_path(&config, path, &file_path)?;
                            println!("  sync: {}", remote_path);
                            if let Err(e) = client.push_file(&file_path, &remote_path).await {
                                eprintln!("  ! {}", e);
                            }
                        }
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                break;
            }
        }
    }

    Ok(())
}

fn compute_remote_path(config: &Config, root: &Path, local_path: &Path) -> anyhow::Result<String> {
    if let Ok(rel) = local_path.strip_prefix(root) {
        return Ok(format!("/{}", rel.to_string_lossy().replace('\\', "/")));
    }
    
    if let Some(sync_root) = &config.sync_root {
        let sync_root = std::path::Path::new(sync_root);
        if let Ok(rel) = local_path.strip_prefix(sync_root) {
            return Ok(format!("/{}", rel.to_string_lossy().replace('\\', "/")));
        }
    }
    
    let filename = local_path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    Ok(format!("/{}", filename))
}
