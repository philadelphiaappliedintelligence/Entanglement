use crate::api::RestClient;
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
use std::io;
use std::path::PathBuf;

#[derive(Clone, PartialEq)]
enum Screen {
    EnterServer,
    ConnectingServer,
    ServerConnected(String, String), // name, grpc_port
    ServerError(String),
    EnterEmail,
    EnterPassword,
    LoggingIn,
    LoggedIn,
    LoginError(String),
    EnterFolderPath,
    CreatingFolder,
    FolderReady,
    FolderError(String),
    Complete,
    Finished,
}

struct App {
    screen: Screen,
    server_url: String,
    server_name: Option<String>,
    grpc_url: Option<String>,
    email: String,
    password: String,
    folder_path: String,
    token: Option<String>,
    user_id: Option<String>,
    error_message: Option<String>,
}

impl App {
    fn new() -> Self {
        let default_folder = dirs::home_dir()
            .map(|h| h.join("Sync").to_string_lossy().to_string())
            .unwrap_or_else(|| "~/Sync".to_string());
        
        Self {
            screen: Screen::EnterServer,
            server_url: String::new(),
            server_name: None,
            grpc_url: None,
            email: String::new(),
            password: String::new(),
            folder_path: default_folder,
            token: None,
            user_id: None,
            error_message: None,
        }
    }
}

pub async fn run_setup() -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();
    let result = run_app(&mut terminal, &mut app).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> anyhow::Result<()> {
    loop {
        terminal.draw(|f| ui(f, app))?;

        // Handle async operations
        match &app.screen {
            Screen::ConnectingServer => {
                let client = RestClient::new(&app.server_url);
                match client.get_server_info().await {
                    Ok(info) => {
                        app.server_name = Some(info.name.clone());
                        // Derive gRPC URL from REST URL
                        let grpc_url = app.server_url
                            .replace(":8080", &format!(":{}", info.grpc_port));
                        app.grpc_url = Some(grpc_url);
                        app.screen = Screen::ServerConnected(info.name, info.grpc_port.to_string());
                    }
                    Err(e) => {
                        app.screen = Screen::ServerError(e.to_string());
                    }
                }
                continue;
            }
            Screen::LoggingIn => {
                let client = RestClient::new(&app.server_url);
                match client.login(&app.email, &app.password).await {
                    Ok(auth) => {
                        app.token = Some(auth.token);
                        app.user_id = Some(auth.user_id);
                        app.screen = Screen::LoggedIn;
                    }
                    Err(e) => {
                        app.screen = Screen::LoginError(e.to_string());
                    }
                }
                continue;
            }
            Screen::CreatingFolder => {
                let path = PathBuf::from(&app.folder_path);
                
                // Create folder with server name
                let folder_name = app.server_name.as_deref().unwrap_or("Entanglement");
                let sync_path = if path.ends_with(folder_name) {
                    path.clone()
                } else {
                    path.join(folder_name)
                };
                
                match std::fs::create_dir_all(&sync_path) {
                    Ok(_) => {
                        // Initialize local database
                        let db_path = sync_path.join(".entanglement");
                        if let Err(e) = std::fs::create_dir_all(&db_path) {
                            app.screen = Screen::FolderError(e.to_string());
                            continue;
                        }
                        if let Err(e) = db::init_local_db(&db_path.join("sync.db")) {
                            app.screen = Screen::FolderError(e.to_string());
                            continue;
                        }
                        
                        // Update folder path to include server name
                        app.folder_path = sync_path.to_string_lossy().to_string();
                        
                        // Save config
                        let config = Config {
                            server_url: Some(app.server_url.clone()),
                            grpc_url: app.grpc_url.clone(),
                            server_name: app.server_name.clone(),
                            token: app.token.clone(),
                            user_id: app.user_id.clone(),
                            sync_root: Some(app.folder_path.clone()),
                        };
                        if let Err(e) = config.save() {
                            app.screen = Screen::FolderError(e.to_string());
                            continue;
                        }
                        
                        app.screen = Screen::FolderReady;
                    }
                    Err(e) => {
                        app.screen = Screen::FolderError(e.to_string());
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

                if key.code == KeyCode::Esc {
                    app.screen = Screen::Finished;
                    continue;
                }

                match &app.screen {
                    Screen::EnterServer => {
                        app.error_message = None;
                        match key.code {
                            KeyCode::Char(c) => app.server_url.push(c),
                            KeyCode::Backspace => { app.server_url.pop(); }
                            KeyCode::Enter => {
                                if app.server_url.is_empty() {
                                    app.error_message = Some("server url required".to_string());
                                } else {
                                    // Add http:// if missing
                                    if !app.server_url.starts_with("http") {
                                        app.server_url = format!("http://{}", app.server_url);
                                    }
                                    app.screen = Screen::ConnectingServer;
                                }
                            }
                            _ => {}
                        }
                    }
                    Screen::ServerConnected(_, _) => {
                        if key.code == KeyCode::Enter {
                            app.screen = Screen::EnterEmail;
                        }
                    }
                    Screen::ServerError(_) => match key.code {
                        KeyCode::Char('r') => {
                            app.screen = Screen::EnterServer;
                        }
                        _ => {}
                    },
                    Screen::EnterEmail => {
                        app.error_message = None;
                        match key.code {
                            KeyCode::Char(c) => app.email.push(c),
                            KeyCode::Backspace => { app.email.pop(); }
                            KeyCode::Enter => {
                                if app.email.is_empty() {
                                    app.error_message = Some("email required".to_string());
                                } else if !app.email.contains('@') {
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
                            KeyCode::Char(c) => app.password.push(c),
                            KeyCode::Backspace => { app.password.pop(); }
                            KeyCode::Enter => {
                                if app.password.is_empty() {
                                    app.error_message = Some("password required".to_string());
                                } else {
                                    app.screen = Screen::LoggingIn;
                                }
                            }
                            _ => {}
                        }
                    }
                    Screen::LoggedIn => {
                        if key.code == KeyCode::Enter {
                            app.screen = Screen::EnterFolderPath;
                        }
                    }
                    Screen::LoginError(_) => match key.code {
                        KeyCode::Char('r') => {
                            app.password.clear();
                            app.screen = Screen::EnterPassword;
                        }
                        _ => {}
                    },
                    Screen::EnterFolderPath => {
                        app.error_message = None;
                        match key.code {
                            KeyCode::Char(c) => app.folder_path.push(c),
                            KeyCode::Backspace => { app.folder_path.pop(); }
                            KeyCode::Enter => {
                                if app.folder_path.is_empty() {
                                    app.error_message = Some("folder path required".to_string());
                                } else {
                                    // Expand ~ to home directory
                                    if app.folder_path.starts_with("~/") {
                                        if let Some(home) = dirs::home_dir() {
                                            app.folder_path = home.join(&app.folder_path[2..])
                                                .to_string_lossy().to_string();
                                        }
                                    }
                                    app.screen = Screen::CreatingFolder;
                                }
                            }
                            _ => {}
                        }
                    }
                    Screen::FolderReady => {
                        if key.code == KeyCode::Enter {
                            app.screen = Screen::Complete;
                        }
                    }
                    Screen::FolderError(_) => match key.code {
                        KeyCode::Char('r') => {
                            app.screen = Screen::EnterFolderPath;
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
        Line::from(Span::styled("entanglement", Style::default().add_modifier(Modifier::BOLD))),
        Line::from(""),
    ];

    match &app.screen {
        Screen::EnterServer => {
            lines.push(Line::from("connect to server"));
            lines.push(Line::from(""));
            lines.push(Line::from(format!("> server: {}_", app.server_url)));
            lines.push(Line::from(""));
            lines.push(Line::from("example: localhost:8080 or myserver.com:8080"));
            if let Some(err) = &app.error_message {
                lines.push(Line::from(""));
                lines.push(Line::from(format!("! {}", err)));
            }
        }
        Screen::ConnectingServer => {
            lines.push(Line::from("connecting to server..."));
        }
        Screen::ServerConnected(name, _) => {
            lines.push(Line::from(format!("* connected to: {}", name)));
            lines.push(Line::from(""));
            lines.push(Line::from("[enter] continue"));
        }
        Screen::ServerError(e) => {
            lines.push(Line::from("! connection failed"));
            lines.push(Line::from(""));
            lines.push(Line::from(e.as_str()));
            lines.push(Line::from(""));
            lines.push(Line::from("[r] retry  [esc] quit"));
        }
        Screen::EnterEmail => {
            lines.push(Line::from(format!("* server: {}", app.server_name.as_deref().unwrap_or("unknown"))));
            lines.push(Line::from(""));
            lines.push(Line::from("login"));
            lines.push(Line::from(""));
            lines.push(Line::from(format!("> email: {}_", app.email)));
            if let Some(err) = &app.error_message {
                lines.push(Line::from(format!("  ! {}", err)));
            }
        }
        Screen::EnterPassword => {
            lines.push(Line::from(format!("* server: {}", app.server_name.as_deref().unwrap_or("unknown"))));
            lines.push(Line::from(""));
            lines.push(Line::from("login"));
            lines.push(Line::from(""));
            lines.push(Line::from(format!("  email: {}", app.email)));
            lines.push(Line::from(format!("> password: {}_", "*".repeat(app.password.len()))));
            if let Some(err) = &app.error_message {
                lines.push(Line::from(format!("  ! {}", err)));
            }
        }
        Screen::LoggingIn => {
            lines.push(Line::from(format!("* server: {}", app.server_name.as_deref().unwrap_or("unknown"))));
            lines.push(Line::from(""));
            lines.push(Line::from("logging in..."));
        }
        Screen::LoggedIn => {
            lines.push(Line::from(format!("* server: {}", app.server_name.as_deref().unwrap_or("unknown"))));
            lines.push(Line::from(format!("* logged in as: {}", app.email)));
            lines.push(Line::from(""));
            lines.push(Line::from("[enter] continue"));
        }
        Screen::LoginError(e) => {
            lines.push(Line::from(format!("* server: {}", app.server_name.as_deref().unwrap_or("unknown"))));
            lines.push(Line::from("! login failed"));
            lines.push(Line::from(""));
            lines.push(Line::from(e.as_str()));
            lines.push(Line::from(""));
            lines.push(Line::from("[r] retry  [esc] quit"));
        }
        Screen::EnterFolderPath => {
            let server_name = app.server_name.as_deref().unwrap_or("Entanglement");
            lines.push(Line::from(format!("* server: {}", server_name)));
            lines.push(Line::from(format!("* logged in as: {}", app.email)));
            lines.push(Line::from(""));
            lines.push(Line::from("choose sync location"));
            lines.push(Line::from(""));
            lines.push(Line::from(format!("a folder named '{}' will be created here:", server_name)));
            lines.push(Line::from(""));
            lines.push(Line::from(format!("> path: {}_", app.folder_path)));
            if let Some(err) = &app.error_message {
                lines.push(Line::from(format!("  ! {}", err)));
            }
        }
        Screen::CreatingFolder => {
            lines.push(Line::from(format!("* server: {}", app.server_name.as_deref().unwrap_or("unknown"))));
            lines.push(Line::from(format!("* logged in as: {}", app.email)));
            lines.push(Line::from(""));
            lines.push(Line::from("creating folder..."));
        }
        Screen::FolderReady => {
            lines.push(Line::from(format!("* server: {}", app.server_name.as_deref().unwrap_or("unknown"))));
            lines.push(Line::from(format!("* logged in as: {}", app.email)));
            lines.push(Line::from(format!("* folder: {}", app.folder_path)));
            lines.push(Line::from(""));
            lines.push(Line::from("[enter] continue"));
        }
        Screen::FolderError(e) => {
            lines.push(Line::from(format!("* server: {}", app.server_name.as_deref().unwrap_or("unknown"))));
            lines.push(Line::from(format!("* logged in as: {}", app.email)));
            lines.push(Line::from("! folder creation failed"));
            lines.push(Line::from(""));
            lines.push(Line::from(e.as_str()));
            lines.push(Line::from(""));
            lines.push(Line::from("[r] retry  [esc] quit"));
        }
        Screen::Complete => {
            lines.push(Line::from(format!("* server: {}", app.server_name.as_deref().unwrap_or("unknown"))));
            lines.push(Line::from(format!("* logged in as: {}", app.email)));
            lines.push(Line::from(format!("* folder: {}", app.folder_path)));
            lines.push(Line::from(""));
            lines.push(Line::from("setup complete!"));
            lines.push(Line::from(""));
            lines.push(Line::from("to start syncing:"));
            lines.push(Line::from("  tangle start"));
            lines.push(Line::from(""));
            lines.push(Line::from("or just run:"));
            lines.push(Line::from("  tangle"));
            lines.push(Line::from(""));
            lines.push(Line::from("[enter] exit"));
        }
        Screen::Finished => {
            lines.push(Line::from("goodbye"));
        }
    }

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, layout[0]);
}

