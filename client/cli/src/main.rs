use clap::{Parser, Subcommand};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod api;
mod chunking;
mod config;
mod daemon;
mod db;
mod sync;

use config::Config;

#[derive(Parser)]
#[command(name = "tangle")]
#[command(about = "Entanglement file sync client")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Interactive setup (server URL + login)
    Setup,
    /// Start background sync daemon
    Start {
        /// Run in foreground (don't daemonize)
        #[arg(long)]
        foreground: bool,
    },
    /// Stop sync daemon
    Stop,
    /// Show daemon status and sync state
    Status,
    /// List synced files
    Ls {
        /// Path prefix filter
        #[arg(default_value = "/")]
        path: String,
    },
    /// Show version history for a file
    History {
        /// File path
        path: String,
    },
    /// Clear credentials and stop syncing
    Logout,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = Config::load()?;

    // Commands that don't need logging
    match &cli.command {
        Some(Commands::Stop) => return daemon::stop(),
        Some(Commands::Start { foreground }) if !foreground => {
            if !config.is_configured() {
                println!("not configured. run: tangle setup");
                return Ok(());
            }
            let pid = daemon::start()?;
            println!("tangle started (pid {})", pid);
            if let Some(dir) = &config.sync_directory {
                println!("syncing: {}", dir);
            }
            return Ok(());
        }
        None => {
            if config.is_configured() {
                if let Some(pid) = daemon::check_running()? {
                    println!("tangle already running (pid {})", pid);
                    return Ok(());
                }
                let pid = daemon::start()?;
                println!("tangle started (pid {})", pid);
                return Ok(());
            } else {
                return run_setup().await;
            }
        }
        _ => {}
    }

    // Initialize logging for foreground/interactive commands
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "tangle=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    match cli.command {
        Some(Commands::Setup) => run_setup().await,
        Some(Commands::Start { .. }) => {
            // Foreground mode
            config.require_auth()?;
            daemon::write_pid(std::process::id())?;
            let result = sync::run(&config).await;
            let _ = daemon::remove_pid();
            result
        }
        Some(Commands::Stop) => unreachable!(),
        Some(Commands::Status) => cmd_status(&config),
        Some(Commands::Ls { path }) => cmd_list(&config, &path).await,
        Some(Commands::History { path }) => cmd_history(&config, &path).await,
        Some(Commands::Logout) => cmd_logout(),
        None => unreachable!(),
    }
}

async fn run_setup() -> anyhow::Result<()> {
    println!("entanglement setup");
    println!();

    // Server URL
    let server_url = prompt("server url")?;
    let server_url = if server_url.starts_with("http") {
        server_url
    } else {
        format!("http://{}", server_url)
    };

    // Test connection
    print!("connecting... ");
    let client = api::ApiClient::new(&server_url);
    let info = client.get_server_info().await?;
    println!("connected to {} (v{})", info.name, info.version);

    // Login
    let username = prompt("username")?;
    let password = rpassword::prompt_password("password: ")?;

    print!("logging in... ");
    let tokens = client.login(&username, &password).await?;
    println!("ok");

    // Sync directory
    let default_dir = dirs::home_dir()
        .map(|h| h.join("Sync").to_string_lossy().to_string())
        .unwrap_or_else(|| "~/Sync".to_string());

    let sync_dir = prompt_default("sync directory", &default_dir)?;
    let sync_dir = expand_tilde(&sync_dir);

    std::fs::create_dir_all(&sync_dir)?;
    println!("sync directory: {}", sync_dir);

    // Save config
    let config = Config {
        server_url: Some(server_url),
        username: Some(username),
        auth_token: Some(tokens.token),
        refresh_token: Some(tokens.refresh_token),
        sync_directory: Some(sync_dir),
    };
    config.save()?;

    println!();
    println!("setup complete! run 'tangle start' to begin syncing.");
    Ok(())
}

fn prompt(label: &str) -> anyhow::Result<String> {
    use std::io::{self, Write};
    print!("{}: ", label);
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim().to_string();
    if trimmed.is_empty() {
        anyhow::bail!("{} is required", label);
    }
    Ok(trimmed)
}

fn prompt_default(label: &str, default: &str) -> anyhow::Result<String> {
    use std::io::{self, Write};
    print!("{} [{}]: ", label, default);
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();
    if trimmed.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

fn expand_tilde(path: &str) -> String {
    if path.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(&path[2..]).to_string_lossy().to_string();
        }
    }
    path.to_string()
}

fn cmd_status(config: &Config) -> anyhow::Result<()> {
    if let Some(server) = &config.server_url {
        println!("server: {}", server);
        if let Some(user) = &config.username {
            println!("user: {}", user);
        }
        if let Some(dir) = &config.sync_directory {
            println!("sync: {}", dir);
        }
        match daemon::check_running()? {
            Some(pid) => println!("daemon: running (pid {})", pid),
            None => println!("daemon: stopped"),
        }
    } else {
        println!("not configured");
        println!("run: tangle setup");
    }
    Ok(())
}

async fn cmd_list(config: &Config, _prefix: &str) -> anyhow::Result<()> {
    config.require_auth()?;
    let client = api::ApiClient::new(config.server_url()?);
    let files = client.list_files(config.auth_token()?).await?;

    if files.is_empty() {
        println!("no files");
        return Ok(());
    }

    for file in files {
        if file.is_deleted {
            continue;
        }
        let size = format_size(file.size_bytes as u64);
        let kind = if file.is_directory { "d" } else { "-" };
        println!("{} {:>10}  {}", kind, size, file.path);
    }
    Ok(())
}

async fn cmd_history(config: &Config, path: &str) -> anyhow::Result<()> {
    config.require_auth()?;
    let client = api::ApiClient::new(config.server_url()?);
    let token = config.auth_token()?;

    // Find file by path
    let files = client.list_files(token).await?;
    let normalized = format!("/{}", path.trim_start_matches('/'));
    let file = files
        .iter()
        .find(|f| f.path == normalized || f.path == path)
        .ok_or_else(|| anyhow::anyhow!("File not found: {}", path))?;

    let versions = client.get_file_versions(token, file.id).await?;
    if versions.is_empty() {
        println!("no versions");
        return Ok(());
    }

    println!("versions of {}:", path);
    for v in versions {
        let size = format_size(v.size_bytes as u64);
        println!("  {}  {}  {}", &v.id.to_string()[..8], v.created_at, size);
    }
    Ok(())
}

fn cmd_logout() -> anyhow::Result<()> {
    let _ = daemon::stop();
    let mut config = Config::load()?;
    config.auth_token = None;
    config.refresh_token = None;
    config.save()?;
    println!("logged out");
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
