# Entanglement

**Content-addressed file sync with intelligent chunking.**

<!-- Badges -->
<!-- [![CI](https://github.com/philadelphiaappliedintelligence/Entanglement/actions/workflows/ci.yml/badge.svg)](https://github.com/philadelphiaappliedintelligence/Entanglement/actions/workflows/ci.yml) -->
<!-- [![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE) -->

Entanglement is a self-hosted file synchronization service that uses content-defined chunking for efficient delta sync. Files are split into variable-size chunks, deduplicated by hash, and stored in append-only packfiles — so only the bytes that actually changed get transferred.

It ships with a Rust server, Rust CLI client, macOS native client (with FileProvider), and a web UI.

---

## Features

- **Content-defined chunking** — FastCDC splits files at natural boundaries so insertions don't cascade into full re-uploads
- **Cross-file deduplication** — chunks are stored once by BLAKE3 hash and reference-counted across all files
- **Immutable versioning** — every save creates a new version; restore any previous version instantly
- **Tiered chunk sizing** — five tiers from Inline (<4 KB, no chunking) to Jumbo (>5 GB, 4–16 MB chunks) adapt to file size and type
- **Append-only packfiles** — chunks are packed into 64 MB blob containers with zstd compression
- **Real-time sync** — WebSocket push notifications trigger immediate sync across connected clients
- **Multi-platform clients** — CLI daemon, macOS FileProvider extension, and web browser
- **File sharing** — generate share links with optional expiry for files and folders
- **Conflict resolution** — automatic detection with manual resolution workflow
- **Selective sync** — per-device rules to control which paths sync
- **Admin dashboard** — web-based user management, server stats, and monitoring
- **Zero-config Docker start** — `make up` handles secrets, migrations, and admin user creation automatically

## Architecture

```
┌──────────────┐   ┌──────────────┐   ┌──────────────┐
│   CLI Client │   │ macOS Client │   │    Web UI    │
│   (tangle)   │   │ FileProvider │   │  (vanilla JS)│
└──────┬───────┘   └──────┬───────┘   └──────┬───────┘
       │                  │                   │
       │         REST API (port 1975)         │
       │           + WebSocket /ws            │
       └──────────────┬──┴───────────────────┘
                      │
              ┌───────▼────────┐
              │  Server (tangled) │
              │     Axum + Tokio  │
              └──┬──────────┬──┘
                 │          │
        ┌────────▼──┐  ┌───▼──────────┐
        │ PostgreSQL │  │ Blob Storage │
        │   (state)  │  │ (packfiles)  │
        └───────────┘  └──────────────┘
```

### Components

| Component | Language | Description |
|-----------|----------|-------------|
| `server/` — **tangled** | Rust | REST API server, WebSocket notifications, PostgreSQL backend |
| `client/cli/` — **tangle** | Rust | CLI sync client with background daemon and file watcher |
| **macOS client** | Swift/SwiftUI | Native macOS app with FileProvider — [available separately](https://github.com/philadelphiaappliedintelligence/Entanglement-macOS) |
| `server/web/` | Vanilla JS | Single-page web UI for file browsing, uploads, sharing, and admin |

### Data Model

Files are identified by path. Each modification creates an immutable **Version** with a BLAKE3 hash. Versions are split into content-addressed **Chunks** via FastCDC, stored in append-only **Blob Containers** (packfiles, 64 MB max). Chunks are deduplicated across all files by hash and reference-counted.

### Chunking Tiers

| Tier | File Size | Chunk Size (min/avg/max) | Notes |
|------|-----------|--------------------------|-------|
| T0 Inline | < 4 KB | — | Stored whole, no chunking |
| T1 Granular | < 10 MB | 2 / 4 / 8 KB | Also used for source code files |
| T2 Standard | 10–500 MB | 16 / 32 / 64 KB | Default tier |
| T3 Large | 500 MB – 5 GB | 512 KB / 1 / 2 MB | |
| T4 Jumbo | > 5 GB | 4 / 8 / 16 MB | Also forced for disk images (.iso, .vmdk, .dmg) |

---

## Quick Start

### Docker (recommended)

```bash
git clone https://github.com/philadelphiaappliedintelligence/Entanglement.git
cd Entanglement
make up
```

That's it. `make up` will:
1. Generate a `JWT_SECRET` if one doesn't exist
2. Start PostgreSQL and the server
3. Run database migrations automatically
4. Create a default **admin** user and print the password to the logs

Grab the generated admin password:

```bash
make logs   # look for "[entrypoint] Password: ..."
```

The server is now available at:
- **REST API:** http://localhost:1975
- **Web UI:** http://localhost:3000

### From Source

Requires Rust 1.75+, PostgreSQL 16.

```bash
cd server

# Start PostgreSQL (if using Docker for just the database)
docker compose up -d postgres

# First-run setup — generates .env, runs migrations, creates admin user
cargo run -- init

# Start the server
cargo run -- serve              # background (daemon)
cargo run -- serve --foreground # foreground with logs
```

### CLI Client

```bash
cd client/cli
cargo build --release

# Configure and connect
./target/release/tangle setup      # server URL + login

# Sync
./target/release/tangle start      # start background sync daemon
./target/release/tangle status     # check sync status
./target/release/tangle ls         # list synced files
./target/release/tangle history    # view file history
./target/release/tangle down       # stop daemon
```

### macOS Client

The native macOS app with Finder integration via FileProvider is available separately. See [Entanglement for macOS](https://github.com/philadelphiaappliedintelligence/Entanglement-macOS).

---

## Server CLI Reference

The server binary is `tangled`. When run with no arguments, it shows status if already configured or runs `init` on first use.

```
tangled init                              First-run setup (generate .env, migrate, create admin)
tangled setup                             Interactive TUI setup wizard
tangled serve [--foreground]              Start server (daemon by default)
tangled down                              Stop server
tangled status                            Show server status
tangled migrate                           Run database migrations
tangled reset [--force]                   Drop all tables (requires confirmation)
tangled index <path>                      Import files from a folder into the server
tangled export <path>                     Export all files to plain folder (emergency recovery)
tangled user create --username <name> [--admin] [--password <pw>]
tangled user list                         List all users
```

### `tangled init`

Designed for first-run setup. Steps through:
1. Generates `.env` with a random `JWT_SECRET` if missing
2. Connects to PostgreSQL (waits up to 10s)
3. Runs all pending migrations
4. Prompts to create an admin user (interactive) or prints instructions (non-interactive)

### `tangled setup`

A full TUI wizard (powered by Ratatui) that walks through server naming, Docker/database startup, migrations, and user creation. Falls back to a non-interactive mode when no TTY is detected.

---

## Configuration

All configuration is via environment variables, loaded from `.env` in the working directory.

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_URL` | `postgres://entanglement:entanglement@localhost:5432/entanglement` | PostgreSQL connection string |
| `JWT_SECRET` | *auto-generated* | JWT signing key. Generate: `openssl rand -hex 32`. **Set this for production** — an ephemeral secret logs out all users on restart. |
| `BLOB_STORAGE_PATH` | `./data/blobs` | Directory for blob container (packfile) storage |
| `REST_PORT` | `1975` | REST API listen port |
| `SERVER_NAME` | `Entanglement` | Server display name shown to clients |
| `CORS_ORIGINS` | `http://localhost:3000,http://127.0.0.1:3000` | Allowed CORS origins (comma-separated) |
| `MAX_UPLOAD_SIZE` | `1073741824` (1 GB) | Maximum upload size in bytes |
| `AUTH_RATE_LIMIT` | `5` | Auth endpoint requests per second (Docker) |
| `AUTH_RATE_BURST` | `10` | Auth rate limit burst size (Docker) |
| `ACCESS_TOKEN_HOURS` | `24` | JWT access token lifetime |
| `REFRESH_TOKEN_DAYS` | `30` | JWT refresh token lifetime |
| `WEB_PORT` | `3000` | Web UI port (Docker only, served by darkhttpd) |

See [`.env.example`](.env.example) for a ready-to-use template.

---

## API Overview

All endpoints are served on the REST port (default 1975). Authentication is via `Authorization: Bearer <token>` header.

### Auth

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/auth/login` | Login with username/password, returns JWT tokens |
| `POST` | `/auth/refresh` | Refresh access token |
| `GET` | `/auth/me` | Get current user info |

### Files (Legacy)

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/files` | List files (paginated) |
| `POST` | `/files` | Upload file (base64 body) |
| `GET` | `/files/:id` | File metadata |
| `PATCH` | `/files/:id` | Move/rename file |
| `DELETE` | `/files/:id` | Soft-delete file |
| `GET` | `/files/:id/download` | Download file content |
| `GET` | `/files/:id/versions` | List file versions |
| `POST` | `/files/:id/restore/:version_id` | Restore a previous version |
| `GET` | `/files/:id/chunks` | Get chunk manifest |
| `POST` | `/files/chunked` | Create file from uploaded chunks |

### Files (V1 — Container-Based)

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/v1/files` | Finalize file upload from chunks |
| `POST` | `/v1/files/directory` | Create virtual directory |
| `GET` | `/v1/files/list` | List directory contents with virtual folders |
| `GET` | `/v1/files/changes` | Incremental sync (changes since timestamp) |
| `GET` | `/v1/files/download-zip` | Download folder as ZIP |
| `GET` | `/v1/files/:version_id/download` | Download file by version ID |
| `GET` | `/v1/files/:id` | File metadata |

### Chunks & Blobs

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/chunks/check` | Check which chunks already exist |
| `PUT` | `/chunks/:hash` | Upload a chunk |
| `GET` | `/chunks/:hash` | Download a chunk |
| `POST` | `/v1/chunks/check` | Check chunks (container storage) |
| `PUT` | `/v1/chunks/:hash` | Upload chunk to container |
| `GET` | `/v1/chunks/:hash` | Download chunk from container |
| `PUT` | `/blobs/:hash` | Upload raw blob |
| `GET` | `/blobs/:hash` | Download blob by hash |
| `POST` | `/metadata` | Create file metadata after blob upload |

### Sharing

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/shares` | List share links |
| `POST` | `/shares` | Create share link |
| `GET` | `/shares/:id` | Share details |
| `DELETE` | `/shares/:id` | Revoke share link |
| `GET` | `/share/:token` | Access shared file (public) |
| `GET` | `/share/:token/download` | Download shared file |
| `GET` | `/share/:token/download-zip` | Download shared folder as ZIP |
| `GET` | `/share/:token/contents` | List shared folder contents |

### Conflicts

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/conflicts` | List sync conflicts |
| `GET` | `/conflicts/:id` | Conflict details |
| `POST` | `/conflicts/:id/resolve` | Resolve conflict |
| `POST` | `/conflicts/detect` | Detect conflicts for files |

### Selective Sync

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/sync/rules` | List sync rules |
| `POST` | `/sync/rules` | Create sync rule |
| `GET` | `/sync/rules/:id` | Rule details |
| `PUT` | `/sync/rules/:id` | Update rule |
| `DELETE` | `/sync/rules/:id` | Delete rule |
| `POST` | `/sync/check` | Check if paths should sync |
| `GET` | `/sync/devices` | List devices |
| `PUT` | `/sync/devices/:device_id` | Update device |
| `DELETE` | `/sync/devices/:device_id` | Remove device |

### Admin

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/admin/users` | List all users |
| `POST` | `/admin/users` | Create user |
| `DELETE` | `/admin/users/:id` | Delete user |
| `PUT` | `/admin/users/:id/password` | Reset user password |
| `PUT` | `/admin/users/:id/admin` | Toggle admin status |
| `GET` | `/admin/stats` | Server statistics |

### Health & Info

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/health` | Combined health check |
| `GET` | `/health/ready` | Readiness probe |
| `GET` | `/health/live` | Liveness probe |
| `GET` | `/server/info` | Server name, version, capabilities |

### WebSocket

| Path | Description |
|------|-------------|
| `/ws/sync` | Real-time file change notifications |

---

## Security

- **Argon2id password hashing** with per-user salts
- **JWT authentication** (HS256) — 24h access tokens with 30d refresh tokens and token rotation
- **Rate limiting** on auth and upload endpoints via tower_governor
- **Path traversal prevention** — normalization and character whitelisting on all file paths
- **File ownership enforcement** on every user-facing endpoint
- **SQL injection protection** — parameterized queries throughout, escaped LIKE patterns
- **Security headers** — CORS, CSP, X-Frame-Options, X-Content-Type-Options on all responses
- **Sanitized error responses** — no internal details leaked to clients
- **HTTPS-ready** — designed to run behind a reverse proxy (nginx, Caddy) for TLS termination

---

## Project Structure

```
Entanglement/
├── server/                          # Rust server (tangled)
│   ├── src/
│   │   ├── main.rs                  # CLI entry point (init, serve, user, etc.)
│   │   ├── config.rs                # Environment-based configuration
│   │   ├── api/
│   │   │   ├── rest/                # Axum route handlers
│   │   │   │   ├── mod.rs           # Router setup and middleware
│   │   │   │   ├── v1.rs            # V1 API (container-based storage)
│   │   │   │   ├── auth.rs          # Login, refresh, user management
│   │   │   │   ├── files.rs         # File CRUD and downloads
│   │   │   │   ├── chunks.rs        # Chunk upload/download
│   │   │   │   ├── blobs.rs         # Raw blob storage
│   │   │   │   ├── versions.rs      # Version history and restore
│   │   │   │   ├── conflicts.rs     # Conflict detection and resolution
│   │   │   │   ├── sharing.rs       # Share link management
│   │   │   │   ├── selective_sync.rs# Per-device sync rules
│   │   │   │   └── admin.rs         # Admin endpoints and health checks
│   │   │   └── ws.rs                # WebSocket sync notifications
│   │   ├── auth/                    # Argon2 hashing + JWT signing
│   │   ├── db/                      # SQLx queries (users, files, versions, chunks)
│   │   ├── storage/                 # Blob I/O, CAS, FastCDC chunking, tiering
│   │   └── tui/                     # Ratatui interactive setup wizard
│   ├── migrations/                  # PostgreSQL schema migrations (SQLx)
│   ├── web/                         # Web UI (vanilla JS SPA)
│   │   ├── index.html
│   │   ├── app.js                   # Application logic (~91 KB)
│   │   └── style.css
│   ├── docker-compose.yml
│   ├── Dockerfile
│   └── entrypoint.sh                # Docker entrypoint (migrations + auto-admin)
│
├── client/
│   ├── cli/                         # Rust CLI client (tangle)
│   │   └── src/
│   │       ├── main.rs              # CLI commands (setup, start, down, ls, etc.)
│   │       ├── api.rs               # REST client (reqwest)
│   │       ├── sync.rs              # Sync engine with delta transfers
│   │       ├── watch.rs             # Filesystem watcher (notify crate)
│   │       ├── chunking.rs          # Client-side FastCDC
│   │       ├── db.rs                # Local SQLite for sync state
│   │       ├── config.rs            # TOML config (~/.config/entanglement/)
│   │       └── daemon.rs            # Background daemon mode
│
├── scripts/                         # Build and setup scripts
├── Cargo.toml                       # Workspace manifest
├── Makefile                         # Docker shortcuts (up, down, logs, clean)
└── .env.example                     # Environment variable template
```

---

## Development

### Prerequisites

- Rust 1.75+ (2021 edition)
- PostgreSQL 16
- Docker & Docker Compose (optional, for containerized setup)

### Build & Test

```bash
# Full workspace
cargo build --workspace
cargo test --workspace

# Server only
cd server
cargo build
cargo test --verbose
cargo fmt --check
cargo clippy -- -D warnings

# CLI client only
cd client/cli
cargo build
cargo test --verbose
```

### Docker

```bash
make up       # Start PostgreSQL + server (auto-generates secrets, runs migrations)
make down     # Stop services
make logs     # Tail server logs
make clean    # Remove containers, volumes, and prune
```

### Database

Migrations live in `server/migrations/` and run automatically on server start. To run manually:

```bash
cd server
cargo run -- migrate          # apply pending migrations
cargo run -- reset --force    # drop all tables (destructive!)
```

### CI

GitHub Actions runs on every push and PR to `main`:

| Job | What it does |
|-----|-------------|
| **server-tests** | `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` with PostgreSQL |
| **cli-tests** | Build and test the CLI client |
| **workspace-check** | Full workspace build and test |
| **docker** | Build Docker image, start stack, health check |

---

## Contributing

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/my-feature`)
3. Make your changes
4. Run tests (`cargo test --workspace`)
5. Check formatting and lints (`cargo fmt --check && cargo clippy -- -D warnings`)
6. Commit and open a pull request

Please keep PRs focused — one feature or fix per PR.

---

## License

[MIT](LICENSE)
