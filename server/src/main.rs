//! Entanglement File Sync Server (tangled)

use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod api;
mod auth;
mod config;
mod db;
mod storage;
mod tui;

use config::Config;

#[derive(Parser)]
#[command(name = "tangled")]
#[command(about = "Entanglement file sync server daemon", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Interactive setup wizard (TUI)
    Setup,
    /// Start the server (runs in background)
    Serve {
        /// Run in foreground (don't daemonize)
        #[arg(long)]
        foreground: bool,
    },
    /// Stop the server
    Down,
    /// Show server status
    Status,
    /// Index files from a folder into the server
    Index {
        /// Folder to index
        path: String,
    },
    /// Run database migrations
    Migrate,
    /// Reset database (drop all tables and data)
    Reset {
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
    /// Export all files to a plain folder (emergency recovery)
    Export {
        /// Output folder
        path: String,
    },
    /// User management
    User {
        #[command(subcommand)]
        command: UserCommands,
    },
}

#[derive(Subcommand)]
enum UserCommands {
    /// Create a new user
    Create {
        /// Username
        #[arg(long)]
        username: String,
        /// Make user an admin
        #[arg(long)]
        admin: bool,
    },
    /// List all users
    List,
}

fn pid_file() -> PathBuf {
    dirs::runtime_dir()
        .or_else(|| dirs::data_local_dir())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("tangled.pid")
}

fn is_server_running() -> Option<u32> {
    let pid_path = pid_file();
    if pid_path.exists() {
        if let Ok(pid_str) = fs::read_to_string(&pid_path) {
            if let Ok(pid) = pid_str.trim().parse::<u32>() {
                // Check if process is still running
                #[cfg(unix)]
                {
                    use std::os::unix::process::CommandExt;
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
        // Stale pid file, remove it
        let _ = fs::remove_file(&pid_path);
    }
    None
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Commands that don't need full init
    match &cli.command {
        Commands::Down => {
            return stop_server();
        }
        Commands::Status => {
            return show_status();
        }
        Commands::Serve { foreground } if !foreground => {
            return start_daemon();
        }
        _ => {}
    }

    // Initialize logging for foreground commands
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "tangled=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load configuration
    dotenvy::dotenv().ok();
    let config = Config::from_env()?;

    match cli.command {
        Commands::Setup => {
            tui::run_setup(config).await?;
        }
        Commands::Serve { foreground: _ } => {
            // Running in foreground mode
            run_server(config).await?;
        }
        Commands::Down => unreachable!(),
        Commands::Status => unreachable!(),
        Commands::Index { path } => {
            index_folder(&config, &path).await?;
        }
        Commands::Export { path } => {
            export_files(&config, &path).await?;
        }
        Commands::Migrate => {
            run_migrations(&config).await?;
        }
        Commands::Reset { force } => {
            reset_database(&config, force).await?;
        }
        Commands::User { command } => match command {
            UserCommands::Create { username, admin } => {
                create_user(&config, &username, admin).await?;
            }
            UserCommands::List => {
                list_users(&config).await?;
            }
        },
    }

    Ok(())
}

fn start_daemon() -> anyhow::Result<()> {
    // Check if already running
    if let Some(pid) = is_server_running() {
        println!("tangled already running (pid {})", pid);
        return Ok(());
    }

    // Get current executable path
    let exe = std::env::current_exe()?;
    
    // Spawn detached process with --foreground flag
    let child = Command::new(&exe)
        .args(["serve", "--foreground"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    let pid = child.id();
    
    // Save PID
    let pid_path = pid_file();
    if let Some(parent) = pid_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&pid_path, pid.to_string())?;

    // Load config to get ports
    dotenvy::dotenv().ok();
    let config = Config::from_env()?;

    println!("tangled serving on localhost:{}", config.rest_port);
    println!("pid: {}", pid);

    Ok(())
}

fn stop_server() -> anyhow::Result<()> {
    if let Some(pid) = is_server_running() {
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
        println!("tangled stopped");
    } else {
        println!("tangled not running");
    }
    Ok(())
}

fn show_status() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let config = Config::from_env()?;
    
    if let Some(pid) = is_server_running() {
        println!("tangled running");
        println!("  pid: {}", pid);
        println!("  rest: localhost:{}", config.rest_port);
    } else {
        println!("tangled not running");
    }
    Ok(())
}

async fn run_server(config: Config) -> anyhow::Result<()> {
    // Save PID for foreground mode too
    let pid_path = pid_file();
    if let Some(parent) = pid_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&pid_path, std::process::id().to_string())?;

    // Initialize database pool
    let db_pool = db::create_pool(&config.database_url).await?;

    // Auto-run migrations on startup (idempotent)
    tracing::info!("checking database migrations...");
    if let Err(e) = db::run_migrations(&db_pool).await {
        // Only warn if it's not an "already exists" error
        let err_str = e.to_string();
        if !err_str.contains("already exists") {
            tracing::warn!("migration warning: {}", err_str);
        }
    }

    // Initialize container-based blob manager (handles both chunked and legacy storage)
    let containers_path = format!("{}/containers", config.blob_storage_path);
    let blob_manager = storage::BlobManager::new(&containers_path, db_pool.clone())?;

    // Create shared application state
    let app_state = api::AppState::new(db_pool.clone(), blob_manager, config.clone());

    // Start REST server
    let rest_addr = format!("0.0.0.0:{}", config.rest_port).parse()?;
    let rest_state = app_state.clone();
    let rest_handle = tokio::spawn(async move {
        tracing::info!("REST listening on {}", rest_addr);
        api::rest::serve(rest_addr, rest_state).await
    });

    // Wait for REST server
    rest_handle.await??;

    // Cleanup PID file
    let _ = fs::remove_file(pid_file());

    Ok(())
}

async fn run_migrations(config: &Config) -> anyhow::Result<()> {
    println!("running migrations...");
    let pool = db::create_pool(&config.database_url).await?;
    db::run_migrations(&pool).await?;
    println!("migrations complete");
    Ok(())
}

async fn create_user(config: &Config, username: &str, is_admin: bool) -> anyhow::Result<()> {
    use std::io::{self, Write};
    
    // Validate username
    if username.len() < 3 {
        anyhow::bail!("Username must be at least 3 characters");
    }
    if !username.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
        anyhow::bail!("Username can only contain letters, numbers, underscores, and hyphens");
    }
    
    // SECURITY: Always prompt for password interactively
    print!("Password: ");
    io::stdout().flush()?;
    
    let password = tokio::task::spawn_blocking(|| -> anyhow::Result<String> {
        let pass = rpassword::read_password()?;
        Ok(pass)
    }).await??;
    
    print!("Confirm password: ");
    io::stdout().flush()?;
    
    let confirm = tokio::task::spawn_blocking(|| -> anyhow::Result<String> {
        let pass = rpassword::read_password()?;
        Ok(pass)
    }).await??;
    
    if password != confirm {
        anyhow::bail!("Passwords do not match");
    }
    
    if password.len() < 4 {
        anyhow::bail!("Password must be at least 4 characters");
    }

    println!("Connecting to database...");
    
    let pool = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        db::create_pool(&config.database_url)
    ).await
        .map_err(|_| anyhow::anyhow!("Database connection timed out. Is PostgreSQL running?"))??;
    
    println!("Hashing password...");
    let password_hash = auth::hash_password(&password)?;
    
    println!("Creating user in database...");
    let user = db::users::create_user(&pool, username, &password_hash, is_admin).await?;

    println!("User created: {} (admin: {})", user.id, user.is_admin);

    Ok(())
}

async fn list_users(config: &Config) -> anyhow::Result<()> {
    let pool = db::create_pool(&config.database_url).await?;
    let users = db::users::list_users(&pool).await?;

    if users.is_empty() {
        println!("no users");
    } else {
        for user in users {
            let role = if user.is_admin { "admin" } else { "user" };
            println!("{} - {} ({})", user.id, user.username, role);
        }
    }

    Ok(())
}

async fn reset_database(config: &Config, force: bool) -> anyhow::Result<()> {
    if !force {
        println!("this will DELETE ALL DATA.");
        println!("type 'yes' to confirm: ");
        
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        
        if input.trim() != "yes" {
            println!("aborted");
            return Ok(());
        }
    }

    println!("resetting database...");
    let pool = db::create_pool(&config.database_url).await?;
    
    sqlx::query(
        "DROP TABLE IF EXISTS sync_cursors CASCADE;
         DROP TABLE IF EXISTS versions CASCADE;
         DROP TABLE IF EXISTS files CASCADE;
         DROP TABLE IF EXISTS users CASCADE;",
    )
    .execute(&pool)
    .await?;

    println!("database reset complete");
    
    Ok(())
}

async fn index_folder(config: &Config, path: &str) -> anyhow::Result<()> {
    use std::io::Read;
    
    let pool = db::create_pool(&config.database_url).await?;
    let containers_path = format!("{}/containers", config.blob_storage_path);
    let blob_manager = storage::BlobManager::new(&containers_path, pool.clone())?;
    
    let base_path = std::path::Path::new(path);
    if !base_path.exists() {
        anyhow::bail!("path does not exist: {}", path);
    }
    
    println!("indexing {}...", path);
    
    let mut count = 0;
    for entry in walkdir::WalkDir::new(base_path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let file_path = entry.path();
        
        // Skip hidden files
        if file_path.components().any(|c| {
            c.as_os_str().to_string_lossy().starts_with('.')
        }) {
            continue;
        }
        
        // Compute remote path
        let remote_path = if let Ok(rel) = file_path.strip_prefix(base_path) {
            format!("/{}", rel.to_string_lossy().replace('\\', "/"))
        } else {
            format!("/{}", file_path.file_name().unwrap_or_default().to_string_lossy())
        };
        
        // Read file and compute hash using BLAKE3
        let mut file = std::fs::File::open(file_path)?;
        let mut content = Vec::new();
        file.read_to_end(&mut content)?;
        
        let blob_hash = blake3::hash(&content).to_hex().to_string();
        
        // Store blob if not exists (using legacy format for compatibility)
        if !blob_manager.legacy_exists(&blob_hash)? {
            blob_manager.write_legacy_blob(&blob_hash, &content)?;
        }
        
        // Create file record (no user ownership)
        let file_record = db::files::upsert_file_global(&pool, &remote_path).await?;
        
        // Create version (no user tracking for indexed files)
        let version = db::versions::create_version_global(
            &pool,
            file_record.id,
            &blob_hash,
            content.len() as i64,
        ).await?;
        
        // Update current version
        db::files::set_current_version(&pool, file_record.id, version.id).await?;
        
        println!("  {}", remote_path);
        count += 1;
    }
    
    println!("indexed {} files", count);
    Ok(())
}

/// Export all files from blob storage to plain files (emergency recovery)
async fn export_files(config: &Config, output_path: &str) -> anyhow::Result<()> {
    let pool = db::create_pool(&config.database_url).await?;
    let containers_path = format!("{}/containers", config.blob_storage_path);
    let blob_manager = storage::BlobManager::new(&containers_path, pool.clone())?;
    
    let output_dir = std::path::Path::new(output_path);
    let current_dir = output_dir.join("current");
    let deleted_dir = output_dir.join("deleted");
    
    fs::create_dir_all(&current_dir)?;
    fs::create_dir_all(&deleted_dir)?;
    
    println!("exporting files to {}...", output_path);
    println!();
    
    // Get ALL files with their current versions (including deleted)
    // Now includes version_id and is_chunked flag for chunk reassembly
    let files = sqlx::query_as::<_, (String, Option<uuid::Uuid>, Option<String>, bool, bool)>(
        r#"
        SELECT f.path, f.current_version_id, v.blob_hash, f.is_deleted, COALESCE(v.is_chunked, FALSE)
        FROM files f
        LEFT JOIN versions v ON f.current_version_id = v.id
        WHERE f.current_version_id IS NOT NULL
        ORDER BY f.is_deleted, f.path
        "#
    )
    .fetch_all(&pool)
    .await?;
    
    let mut current_count = 0;
    let mut deleted_count = 0;
    let mut errors = 0;
    
    // Helper function to read file content (handles both chunked and non-chunked)
    async fn read_file_content(
        pool: &db::DbPool,
        blob_manager: &storage::BlobManager,
        version_id: uuid::Uuid,
        blob_hash: &str,
        is_chunked: bool,
    ) -> anyhow::Result<Vec<u8>> {
        if is_chunked {
            // Reassemble from chunks
            let version_chunks = db::chunks::get_version_chunks(pool, version_id).await?;
            let mut content = Vec::new();
            for vc in version_chunks {
                let chunk_data = blob_manager.read_legacy_blob(&vc.chunk_hash)?;
                content.extend_from_slice(&chunk_data);
            }
            Ok(content)
        } else {
            // Read single blob
            Ok(blob_manager.read_legacy_blob(blob_hash)?)
        }
    }
    
    println!("current files:");
    for (path, version_id, blob_hash, is_deleted, is_chunked) in &files {
        if *is_deleted { continue; }
        
        let version_id = match version_id {
            Some(v) => *v,
            None => continue,
        };
        
        let blob_hash = blob_hash.as_deref().unwrap_or("");
        
        let relative_path = path.trim_start_matches('/');
        let file_path = current_dir.join(relative_path);
        
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        match read_file_content(&pool, &blob_manager, version_id, blob_hash, *is_chunked).await {
            Ok(content) => {
                fs::write(&file_path, content)?;
                let chunked_marker = if *is_chunked { " (chunked)" } else { "" };
                println!("  ✓ {}{}", relative_path, chunked_marker);
                current_count += 1;
            }
            Err(e) => {
                println!("  ✗ {} (error: {})", relative_path, e);
                errors += 1;
            }
        }
    }
    
    println!();
    println!("deleted files:");
    for (path, version_id, blob_hash, is_deleted, is_chunked) in &files {
        if !*is_deleted { continue; }
        
        let version_id = match version_id {
            Some(v) => *v,
            None => continue,
        };
        
        let blob_hash = blob_hash.as_deref().unwrap_or("");
        
        let relative_path = path.trim_start_matches('/');
        let file_path = deleted_dir.join(relative_path);
        
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        match read_file_content(&pool, &blob_manager, version_id, blob_hash, *is_chunked).await {
            Ok(content) => {
                fs::write(&file_path, content)?;
                let chunked_marker = if *is_chunked { " (chunked)" } else { "" };
                println!("  ✓ {}{}", relative_path, chunked_marker);
                deleted_count += 1;
            }
            Err(e) => {
                println!("  ✗ {} (error: {})", relative_path, e);
                errors += 1;
            }
        }
    }
    
    println!();
    println!("═══════════════════════════════════");
    println!("exported {} current files", current_count);
    println!("exported {} deleted files", deleted_count);
    if errors > 0 {
        println!("errors: {} (blobs missing)", errors);
    }
    println!();
    println!("current files: {}/current/", output_path);
    println!("deleted files: {}/deleted/", output_path);
    
    Ok(())
}
