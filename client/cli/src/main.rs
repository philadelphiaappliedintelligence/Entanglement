use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod api;
mod config;
mod db;
mod sync;
mod tui;
mod watch;

use config::Config;

#[derive(Parser)]
#[command(name = "tangle")]
#[command(about = "Entanglement file sync client", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Interactive setup wizard
    Setup,
    /// Start syncing in background
    Start {
        /// Run in foreground (don't daemonize)
        #[arg(long)]
        foreground: bool,
    },
    /// Stop syncing
    Down,
    /// Show sync status
    Status,
    /// Logout and clear credentials
    Logout,
    /// List remote files
    Ls {
        /// Remote path prefix
        #[arg(default_value = "/")]
        path: String,
    },
    /// Show version history for a file
    History {
        /// Remote path
        path: String,
    },
}

fn pid_file() -> PathBuf {
    dirs::runtime_dir()
        .or_else(|| dirs::data_local_dir())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("tangle.pid")
}

fn is_sync_running() -> Option<u32> {
    let pid_path = pid_file();
    if pid_path.exists() {
        if let Ok(pid_str) = fs::read_to_string(&pid_path) {
            if let Ok(pid) = pid_str.trim().parse::<u32>() {
                #[cfg(unix)]
                {
                    let result = Command::new("kill")
                        .args(["-0", &pid.to_string()])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                    if result.map(|s| s.success()).unwrap_or(false) {
                        return Some(pid);
                    }
                }
                #[cfg(not(unix))]
                {
                    return Some(pid);
                }
            }
        }
        let _ = fs::remove_file(&pid_path);
    }
    None
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = Config::load()?;

    // Commands that don't need full init
    match &cli.command {
        Some(Commands::Down) => {
            return stop_sync();
        }
        Some(Commands::Start { foreground }) if !foreground => {
            if !config.is_configured() {
                println!("not configured. run: tangle setup");
                return Ok(());
            }
            return start_daemon();
        }
        None => {
            // Default: start syncing if configured, otherwise setup
            if config.is_configured() {
                if is_sync_running().is_some() {
                    println!("tangle already running");
                    return Ok(());
                }
                return start_daemon();
            } else {
                tui::run_setup().await?;
                return Ok(());
            }
        }
        _ => {}
    }

    // Initialize logging for foreground commands
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "tangle=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    match cli.command {
        Some(Commands::Setup) => {
            tui::run_setup().await?;
        }
        Some(Commands::Start { foreground: _ }) => {
            // Running in foreground mode
            start_sync_foreground(&config).await?;
        }
        Some(Commands::Down) => unreachable!(),
        Some(Commands::Status) => {
            status(&config)?;
        }
        Some(Commands::Logout) => {
            logout()?;
        }
        Some(Commands::Ls { path }) => {
            list(&config, &path).await?;
        }
        Some(Commands::History { path }) => {
            history(&config, &path).await?;
        }
        None => unreachable!(),
    }

    Ok(())
}

fn start_daemon() -> anyhow::Result<()> {
    if let Some(pid) = is_sync_running() {
        println!("tangle already running (pid {})", pid);
        return Ok(());
    }

    let config = Config::load()?;
    let exe = std::env::current_exe()?;
    
    let child = Command::new(&exe)
        .args(["start", "--foreground"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    let pid = child.id();
    
    let pid_path = pid_file();
    if let Some(parent) = pid_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&pid_path, pid.to_string())?;

    println!("tangle syncing {}", config.sync_root.as_deref().unwrap_or(""));
    println!("pid: {}", pid);

    Ok(())
}

fn stop_sync() -> anyhow::Result<()> {
    if let Some(pid) = is_sync_running() {
        #[cfg(unix)]
        {
            Command::new("kill")
                .args([&pid.to_string()])
                .status()?;
        }
        #[cfg(not(unix))]
        {
            Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/F"])
                .status()?;
        }
        
        let _ = fs::remove_file(pid_file());
        println!("tangle stopped");
    } else {
        println!("tangle not running");
    }
    Ok(())
}

fn status(config: &Config) -> anyhow::Result<()> {
    if let Some(server) = &config.server_url {
        println!("server: {} ({})", config.server_name.as_deref().unwrap_or("unknown"), server);
        
        if let Some(sync_root) = &config.sync_root {
            println!("folder: {}", sync_root);
        }
        
        if let Some(pid) = is_sync_running() {
            println!("sync: running (pid {})", pid);
        } else {
            println!("sync: stopped");
        }
    } else {
        println!("not configured");
        println!("run: tangle setup");
    }
    Ok(())
}

fn logout() -> anyhow::Result<()> {
    // Stop sync first
    if is_sync_running().is_some() {
        stop_sync()?;
    }
    
    let mut config = Config::load()?;
    config.token = None;
    config.user_id = None;
    config.save()?;
    println!("logged out");
    Ok(())
}

async fn start_sync_foreground(config: &Config) -> anyhow::Result<()> {
    config.require_auth()?;
    
    let sync_root = config.sync_root.as_ref()
        .ok_or_else(|| anyhow::anyhow!("No sync folder configured. Run: tangle setup"))?;
    
    let path = std::path::Path::new(sync_root);
    if !path.exists() {
        std::fs::create_dir_all(path)?;
    }

    // Save PID
    let pid_path = pid_file();
    if let Some(parent) = pid_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&pid_path, std::process::id().to_string())?;
    
    // Initial sync
    sync::sync_directory(config, path).await?;
    
    // Then watch for changes
    let result = watch::start_watching(config, path).await;
    
    // Cleanup PID file
    let _ = fs::remove_file(pid_file());
    
    result
}

async fn list(config: &Config, prefix: &str) -> anyhow::Result<()> {
    config.require_auth()?;
    
    let mut client = api::GrpcClient::connect(config).await?;
    let files = client.list_files(prefix).await?;
    
    if files.is_empty() {
        println!("no files");
        return Ok(());
    }
    
    for file in files {
        let size = format_size(file.size_bytes as u64);
        let deleted = if file.is_deleted { " [deleted]" } else { "" };
        println!("{:>10}  {}{}", size, file.path, deleted);
    }
    
    Ok(())
}

async fn history(config: &Config, path: &str) -> anyhow::Result<()> {
    config.require_auth()?;
    
    let mut client = api::GrpcClient::connect(config).await?;
    let versions = client.list_versions(path).await?;
    
    if versions.is_empty() {
        println!("no versions found");
        return Ok(());
    }
    
    println!("versions of {}:", path);
    for v in versions {
        let time = chrono::DateTime::from_timestamp(v.created_at, 0)
            .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let size = format_size(v.size_bytes as u64);
        println!("  {}  {}  {}", &v.version_id[..8], time, size);
    }
    
    Ok(())
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
