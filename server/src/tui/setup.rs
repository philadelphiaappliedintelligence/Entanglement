use crate::auth;
use crate::config::Config;
use crate::db;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame, Terminal,
};
use std::io::{self, IsTerminal};

#[derive(Clone, PartialEq)]
enum Screen {
    Welcome,
    EnterServerName,
    StartingDocker,
    DockerStarted,
    DockerError(String),
    ConnectingDatabase,
    DatabaseConnected,
    DatabaseError(String),
    RunningMigrations,
    MigrationsComplete,
    MigrationsError(String),
    EnterEmail,
    EnterPassword,
    ConfirmPassword,
    CreatingUser,
    UserCreated(String),
    UserError(String),
    AnotherUser,
    Complete,
    Finished,
}

struct App {
    screen: Screen,
    config: Config,
    server_name_input: String,
    email_input: String,
    password_input: String,
    password_confirm: String,
    error_message: Option<String>,
    db_pool: Option<db::DbPool>,
}

impl App {
    fn new(config: Config) -> Self {
        let server_name_input = config.server_name.clone();
        Self {
            screen: Screen::Welcome,
            config,
            server_name_input,
            email_input: String::new(),
            password_input: String::new(),
            password_confirm: String::new(),
            error_message: None,
            db_pool: None,
        }
    }
}

/// Run setup - automatically detects interactive vs non-interactive mode
pub async fn run_setup(config: Config) -> anyhow::Result<()> {
    // Check if we have a TTY
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return run_non_interactive_setup(config).await;
    }

    // Try TUI, fall back to non-interactive if it fails
    match run_tui_setup(config.clone()).await {
        Ok(()) => Ok(()),
        Err(e) => {
            // If TUI failed, try non-interactive
            eprintln!("TUI failed ({}), running non-interactive setup...", e);
            run_non_interactive_setup(config).await
        }
    }
}

/// Non-interactive setup for scripts/CI
async fn run_non_interactive_setup(config: Config) -> anyhow::Result<()> {
    println!("entanglement server setup (non-interactive)");
    println!();

    // Step 1: Start Docker if compose file exists
    let compose_exists = std::path::Path::new("docker-compose.yml").exists();
    if compose_exists {
        println!("starting database...");
        
        // First try to start existing container
        let start_result = std::process::Command::new("docker")
            .args(["start", "entanglement-db"])
            .output();
        
        let started = match start_result {
            Ok(output) if output.status.success() => true,
            _ => {
                // Try docker compose up
                let compose_result = std::process::Command::new("docker")
                    .args(["compose", "up", "-d"])
                    .output();
                
                match compose_result {
                    Ok(output) if output.status.success() => true,
                    Ok(output) => {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        eprintln!("docker compose failed: {}", stderr);
                        false
                    }
                    Err(e) => {
                        eprintln!("docker not found: {}", e);
                        false
                    }
                }
            }
        };

        if started {
            println!("  waiting for database to be ready...");
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    }

    // Step 2: Connect to database
    println!("connecting to database...");
    let pool = match db::create_pool(&config.database_url).await {
        Ok(p) => {
            println!("  connected");
            p
        }
        Err(e) => {
            anyhow::bail!("database connection failed: {}", e);
        }
    };

    // Step 3: Run migrations
    println!("running migrations...");
    match db::run_migrations(&pool).await {
        Ok(()) => println!("  complete"),
        Err(e) => {
            // Migrations might fail if tables exist - that's ok
            if e.to_string().contains("already exists") {
                println!("  tables already exist, skipping");
            } else {
                anyhow::bail!("migrations failed: {}", e);
            }
        }
    }

    println!();
    println!("setup complete!");
    println!();
    println!("to create a user:");
    println!("  tangled user create --email user@example.com --password yourpassword");
    println!();
    println!("to start the server:");
    println!("  tangled serve");
    println!();

    Ok(())
}

/// Interactive TUI setup
async fn run_tui_setup(config: Config) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(config);
    let result = run_app(&mut terminal, &mut app).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    // Save server name to env file
    if !app.server_name_input.is_empty() && app.server_name_input != "Entanglement" {
        save_server_name(&app.server_name_input)?;
    }

    result
}

fn save_server_name(name: &str) -> anyhow::Result<()> {
    let env_path = std::path::Path::new(".env");
    let mut content = if env_path.exists() {
        std::fs::read_to_string(env_path)?
    } else {
        String::new()
    };

    // Update or add SERVER_NAME
    if content.contains("SERVER_NAME=") {
        let lines: Vec<&str> = content.lines().collect();
        let new_lines: Vec<String> = lines
            .into_iter()
            .map(|l| {
                if l.starts_with("SERVER_NAME=") {
                    format!("SERVER_NAME={}", name)
                } else {
                    l.to_string()
                }
            })
            .collect();
        content = new_lines.join("\n");
    } else {
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&format!("SERVER_NAME={}\n", name));
    }

    std::fs::write(env_path, content)?;
    Ok(())
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> anyhow::Result<()> {
    loop {
        terminal.draw(|f| ui(f, app))?;

        // Handle async operations - these auto-advance
        match &app.screen {
            Screen::StartingDocker => {
                // First try to start existing container
                let start_result = std::process::Command::new("docker")
                    .args(["start", "entanglement-db"])
                    .output();
                
                if let Ok(output) = start_result {
                    if output.status.success() {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        app.screen = Screen::DockerStarted;
                        continue;
                    }
                }
                
                // Container doesn't exist, try docker compose up
                let result = std::process::Command::new("docker")
                    .args(["compose", "up", "-d"])
                    .output();
                
                match result {
                    Ok(output) => {
                        if output.status.success() {
                            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                            app.screen = Screen::DockerStarted;
                        } else {
                            let stderr = String::from_utf8_lossy(&output.stderr);
                            let stdout = String::from_utf8_lossy(&output.stdout);
                            let error = if stderr.is_empty() { stdout } else { stderr };
                            app.screen = Screen::DockerError(error.trim().to_string());
                        }
                    }
                    Err(e) => {
                        app.screen = Screen::DockerError(format!("docker not found: {}", e));
                    }
                }
                continue;
            }
            Screen::DockerStarted => {
                // Auto-advance to database connection
                app.screen = Screen::ConnectingDatabase;
                continue;
            }
            Screen::ConnectingDatabase => {
                match db::create_pool(&app.config.database_url).await {
                    Ok(pool) => {
                        app.db_pool = Some(pool);
                        app.screen = Screen::DatabaseConnected;
                    }
                    Err(e) => {
                        app.screen = Screen::DatabaseError(e.to_string());
                    }
                }
                continue;
            }
            Screen::DatabaseConnected => {
                // Auto-advance to migrations
                app.screen = Screen::RunningMigrations;
                continue;
            }
            Screen::RunningMigrations => {
                if let Some(pool) = &app.db_pool {
                    match db::run_migrations(pool).await {
                        Ok(_) => app.screen = Screen::MigrationsComplete,
                        Err(e) => {
                            // Check if it's just "already exists" error
                            let err_str = e.to_string();
                            if err_str.contains("already exists") {
                                app.screen = Screen::MigrationsComplete;
                            } else {
                                app.screen = Screen::MigrationsError(err_str);
                            }
                        }
                    }
                }
                continue;
            }
            Screen::CreatingUser => {
                if let Some(pool) = &app.db_pool {
                    match auth::hash_password(&app.password_input) {
                        Ok(hash) => {
                            match db::users::create_user(pool, &app.email_input, &hash).await {
                                Ok(user) => app.screen = Screen::UserCreated(user.id.to_string()),
                                Err(e) => app.screen = Screen::UserError(e.to_string()),
                            }
                        }
                        Err(e) => app.screen = Screen::UserError(e.to_string()),
                    }
                }
                continue;
            }
            Screen::Finished => return Ok(()),
            _ => {}
        }

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                // Global quit
                if key.code == KeyCode::Esc {
                    app.screen = Screen::Finished;
                    continue;
                }

                match &app.screen {
                    Screen::Welcome => {
                        if key.code == KeyCode::Enter {
                            app.screen = Screen::EnterServerName;
                        }
                    }
                    Screen::EnterServerName => {
                        app.error_message = None;
                        match key.code {
                            KeyCode::Char(c) => app.server_name_input.push(c),
                            KeyCode::Backspace => { app.server_name_input.pop(); }
                            KeyCode::Enter => {
                                if app.server_name_input.is_empty() {
                                    app.error_message = Some("name required".to_string());
                                } else {
                                    app.config.set_server_name(app.server_name_input.clone());
                                    // Check if compose file exists
                                    if std::path::Path::new("docker-compose.yml").exists() {
                                        app.screen = Screen::StartingDocker;
                                    } else {
                                        app.screen = Screen::ConnectingDatabase;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    Screen::DockerError(_) => match key.code {
                        KeyCode::Char('r') => {
                            app.screen = Screen::StartingDocker;
                        }
                        KeyCode::Char('s') => {
                            app.screen = Screen::ConnectingDatabase;
                        }
                        _ => {}
                    }
                    Screen::DatabaseError(_) => {
                        if key.code == KeyCode::Char('r') {
                            app.screen = Screen::ConnectingDatabase;
                        }
                    }
                    Screen::MigrationsComplete => {
                        if key.code == KeyCode::Enter {
                            app.screen = Screen::EnterEmail;
                        }
                    }
                    Screen::MigrationsError(_) => match key.code {
                        KeyCode::Char('r') => {
                            app.screen = Screen::RunningMigrations;
                        }
                        KeyCode::Char('s') => {
                            app.screen = Screen::EnterEmail;
                        }
                        _ => {}
                    }
                    Screen::EnterEmail => {
                        app.error_message = None;
                        match key.code {
                            KeyCode::Char(c) => app.email_input.push(c),
                            KeyCode::Backspace => { app.email_input.pop(); }
                            KeyCode::Enter => {
                                if app.email_input.is_empty() {
                                    app.error_message = Some("email required".to_string());
                                } else if !app.email_input.contains('@') {
                                    app.error_message = Some("invalid email".to_string());
                                } else {
                                    app.screen = Screen::EnterPassword;
                                }
                            }
                            _ => {}
                        }
                    }
                    Screen::EnterPassword => {
                        app.error_message = None;
                        match key.code {
                            KeyCode::Char(c) => app.password_input.push(c),
                            KeyCode::Backspace => { app.password_input.pop(); }
                            KeyCode::Enter => {
                                if app.password_input.len() < 8 {
                                    app.error_message = Some("min 8 characters".to_string());
                                } else {
                                    app.screen = Screen::ConfirmPassword;
                                }
                            }
                            _ => {}
                        }
                    }
                    Screen::ConfirmPassword => {
                        app.error_message = None;
                        match key.code {
                            KeyCode::Char(c) => app.password_confirm.push(c),
                            KeyCode::Backspace => { app.password_confirm.pop(); }
                            KeyCode::Enter => {
                                if app.password_confirm != app.password_input {
                                    app.error_message = Some("passwords don't match".to_string());
                                    app.password_confirm.clear();
                                } else {
                                    app.screen = Screen::CreatingUser;
                                }
                            }
                            _ => {}
                        }
                    }
                    Screen::UserCreated(_) => {
                        if key.code == KeyCode::Enter {
                            app.screen = Screen::AnotherUser;
                        }
                    }
                    Screen::UserError(_) => match key.code {
                        KeyCode::Char('r') => {
                            app.password_input.clear();
                            app.password_confirm.clear();
                            app.screen = Screen::EnterEmail;
                        }
                        KeyCode::Enter => app.screen = Screen::Complete,
                        _ => {}
                    },
                    Screen::AnotherUser => match key.code {
                        KeyCode::Char('y') => {
                            app.email_input.clear();
                            app.password_input.clear();
                            app.password_confirm.clear();
                            app.screen = Screen::EnterEmail;
                        }
                        KeyCode::Char('n') | KeyCode::Enter => {
                            app.screen = Screen::Complete;
                        }
                        _ => {}
                    },
                    Screen::Complete => {
                        if key.code == KeyCode::Enter {
                            app.screen = Screen::Finished;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

fn ui(f: &mut Frame, app: &App) {
    let area = f.area();
    
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Min(0)])
        .split(area);

    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled("entanglement server", Style::default().add_modifier(Modifier::BOLD))),
        Line::from(""),
    ];

    match &app.screen {
        Screen::Welcome => {
            lines.push(Line::from("setup wizard"));
            lines.push(Line::from(""));
            lines.push(Line::from("this will:"));
            lines.push(Line::from("  - name your server"));
            lines.push(Line::from("  - start database (docker)"));
            lines.push(Line::from("  - run migrations"));
            lines.push(Line::from("  - create user account"));
            lines.push(Line::from(""));
            lines.push(Line::from("[enter] continue  [esc] quit"));
        }
        Screen::EnterServerName => {
            lines.push(Line::from("server name"));
            lines.push(Line::from(""));
            lines.push(Line::from("this name appears on connected clients"));
            lines.push(Line::from(""));
            lines.push(Line::from(format!("> {}_", app.server_name_input)));
            if let Some(err) = &app.error_message {
                lines.push(Line::from(format!("  ! {}", err)));
            }
        }
        Screen::StartingDocker => {
            lines.push(Line::from(format!("* {}", app.config.server_name)));
            lines.push(Line::from(""));
            lines.push(Line::from("starting database..."));
        }
        Screen::DockerStarted => {
            lines.push(Line::from(format!("* {}", app.config.server_name)));
            lines.push(Line::from("* database started"));
            lines.push(Line::from(""));
            lines.push(Line::from("connecting..."));
        }
        Screen::DockerError(e) => {
            lines.push(Line::from(format!("* {}", app.config.server_name)));
            lines.push(Line::from(""));
            lines.push(Line::from("! docker error"));
            for line in e.lines().take(4) {
                lines.push(Line::from(format!("  {}", line)));
            }
            lines.push(Line::from(""));
            lines.push(Line::from("[r] retry  [s] skip  [esc] quit"));
        }
        Screen::ConnectingDatabase => {
            lines.push(Line::from(format!("* {}", app.config.server_name)));
            lines.push(Line::from("* database started"));
            lines.push(Line::from(""));
            lines.push(Line::from("connecting to database..."));
        }
        Screen::DatabaseConnected => {
            lines.push(Line::from(format!("* {}", app.config.server_name)));
            lines.push(Line::from("* database started"));
            lines.push(Line::from("* database connected"));
            lines.push(Line::from(""));
            lines.push(Line::from("running migrations..."));
        }
        Screen::DatabaseError(e) => {
            lines.push(Line::from(format!("* {}", app.config.server_name)));
            lines.push(Line::from(""));
            lines.push(Line::from("! database error"));
            lines.push(Line::from(""));
            for line in e.lines().take(3) {
                lines.push(Line::from(format!("  {}", line)));
            }
            lines.push(Line::from(""));
            lines.push(Line::from("[r] retry  [esc] quit"));
        }
        Screen::RunningMigrations => {
            lines.push(Line::from(format!("* {}", app.config.server_name)));
            lines.push(Line::from("* database started"));
            lines.push(Line::from("* database connected"));
            lines.push(Line::from(""));
            lines.push(Line::from("running migrations..."));
        }
        Screen::MigrationsComplete => {
            lines.push(Line::from(format!("* {}", app.config.server_name)));
            lines.push(Line::from("* database started"));
            lines.push(Line::from("* database connected"));
            lines.push(Line::from("* migrations complete"));
            lines.push(Line::from(""));
            lines.push(Line::from("[enter] create user"));
        }
        Screen::MigrationsError(e) => {
            lines.push(Line::from(format!("* {}", app.config.server_name)));
            lines.push(Line::from("* database started"));
            lines.push(Line::from("* database connected"));
            lines.push(Line::from(""));
            lines.push(Line::from("! migration error"));
            for line in e.lines().take(3) {
                lines.push(Line::from(format!("  {}", line)));
            }
            lines.push(Line::from(""));
            lines.push(Line::from("[r] retry  [s] skip  [esc] quit"));
        }
        Screen::EnterEmail => {
            lines.push(Line::from(format!("* {}", app.config.server_name)));
            lines.push(Line::from("* database ready"));
            lines.push(Line::from("* migrations complete"));
            lines.push(Line::from(""));
            lines.push(Line::from("create user"));
            lines.push(Line::from(""));
            lines.push(Line::from(format!("> email: {}_", app.email_input)));
            if let Some(err) = &app.error_message {
                lines.push(Line::from(format!("  ! {}", err)));
            }
        }
        Screen::EnterPassword => {
            lines.push(Line::from(format!("* {}", app.config.server_name)));
            lines.push(Line::from("* database ready"));
            lines.push(Line::from("* migrations complete"));
            lines.push(Line::from(""));
            lines.push(Line::from("create user"));
            lines.push(Line::from(""));
            lines.push(Line::from(format!("  email: {}", app.email_input)));
            lines.push(Line::from(format!("> password: {}_", "*".repeat(app.password_input.len()))));
            if let Some(err) = &app.error_message {
                lines.push(Line::from(format!("  ! {}", err)));
            }
        }
        Screen::ConfirmPassword => {
            lines.push(Line::from(format!("* {}", app.config.server_name)));
            lines.push(Line::from("* database ready"));
            lines.push(Line::from("* migrations complete"));
            lines.push(Line::from(""));
            lines.push(Line::from("create user"));
            lines.push(Line::from(""));
            lines.push(Line::from(format!("  email: {}", app.email_input)));
            lines.push(Line::from(format!("  password: {}", "*".repeat(app.password_input.len()))));
            lines.push(Line::from(format!("> confirm: {}_", "*".repeat(app.password_confirm.len()))));
            if let Some(err) = &app.error_message {
                lines.push(Line::from(format!("  ! {}", err)));
            }
        }
        Screen::CreatingUser => {
            lines.push(Line::from(format!("* {}", app.config.server_name)));
            lines.push(Line::from("* database ready"));
            lines.push(Line::from("* migrations complete"));
            lines.push(Line::from(""));
            lines.push(Line::from("creating user..."));
        }
        Screen::UserCreated(id) => {
            lines.push(Line::from(format!("* {}", app.config.server_name)));
            lines.push(Line::from("* database ready"));
            lines.push(Line::from("* migrations complete"));
            lines.push(Line::from(format!("* user: {}", &id[..8])));
            lines.push(Line::from(""));
            lines.push(Line::from("[enter] continue"));
        }
        Screen::UserError(e) => {
            lines.push(Line::from(format!("* {}", app.config.server_name)));
            lines.push(Line::from("* database ready"));
            lines.push(Line::from("* migrations complete"));
            lines.push(Line::from(""));
            lines.push(Line::from("! user creation failed"));
            lines.push(Line::from(format!("  {}", e)));
            lines.push(Line::from(""));
            lines.push(Line::from("[r] retry  [enter] skip"));
        }
        Screen::AnotherUser => {
            lines.push(Line::from(format!("* {}", app.config.server_name)));
            lines.push(Line::from("* database ready"));
            lines.push(Line::from("* migrations complete"));
            lines.push(Line::from("* user created"));
            lines.push(Line::from(""));
            lines.push(Line::from("create another user? [y/n]"));
        }
        Screen::Complete => {
            lines.push(Line::from(format!("* {}", app.config.server_name)));
            lines.push(Line::from("* database ready"));
            lines.push(Line::from("* migrations complete"));
            lines.push(Line::from("* user created"));
            lines.push(Line::from(""));
            lines.push(Line::from("setup complete!"));
            lines.push(Line::from(""));
            lines.push(Line::from("to start:"));
            lines.push(Line::from("  tangled serve"));
            lines.push(Line::from(""));
            lines.push(Line::from(format!("rest: localhost:{}", app.config.rest_port)));
            lines.push(Line::from(format!("grpc: localhost:{}", app.config.grpc_port)));
            lines.push(Line::from(""));
            lines.push(Line::from("[enter] exit"));
        }
        Screen::Finished => {}
    }

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, layout[0]);
}
