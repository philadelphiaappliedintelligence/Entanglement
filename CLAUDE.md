# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Entanglement is a file synchronization service with content-defined chunking for efficient delta sync. It consists of a Rust server (`tangled`), Rust CLI client (`tangle`), macOS/iOS native clients (Swift/SwiftUI), and a vanilla JS web frontend.

## Build & Run Commands

### Server (binary: `tangled`)
```bash
cd server
export DATABASE_URL=postgres://entanglement:entanglement@localhost:5432/entanglement
export JWT_SECRET=$(openssl rand -hex 32)
export BLOB_STORAGE_PATH=./data/blobs
cargo build                    # Debug build
cargo run -- serve             # Run server (REST on :1975)
cargo test --verbose           # Run all server tests
cargo test test_name           # Run a single test
cargo fmt --check              # Check formatting
cargo clippy -- -D warnings    # Lint
```

### CLI Client (binary: `tangle`)
```bash
cd client/cli
cargo build                    # Debug build
cargo test --verbose           # Run tests
# Commands: setup, start, down, status, ls, history, logout
```

### macOS Client
```bash
cd client/macos/Entanglement
xcodebuild build -project Entanglement.xcodeproj -scheme Entanglement
xcodebuild test -project Entanglement.xcodeproj -scheme Entanglement
```

### Docker (full stack)
```bash
make up       # Start postgres + server (runs scripts/pre-start.sh first)
make down     # Stop services
make logs     # Tail server logs
make clean    # Remove containers and volumes
```

## Architecture

### Data Model
Files are identified by unique path. Each modification creates an immutable **Version** with a BLAKE3 hash. Versions are split into content-addressed **Chunks** via FastCDC, stored in append-only **Blob Containers** (packfiles, 64MB max). Chunks are deduplicated across files by hash and reference-counted.

### Chunking Tiers
File size determines chunk parameters — five tiers from Inline (<4KB, no chunking) through Jumbo (>5GB, 4-16MB chunks). Tier config lives in `server/src/storage/tiering.rs`. The macOS client mirrors this in `ChunkingConfig.swift` and `TierSelector.swift`.

### Server Layout (`server/src/`)
- `api/rest/` — Axum route handlers (auth, files, chunks, blobs, conflicts, sharing, admin, versions)
- `api/ws.rs` — WebSocket push notifications for real-time sync
- `auth/` — Argon2 password hashing + JWT token creation/verification
- `db/` — SQLx queries against PostgreSQL (models, users, files, versions, chunks, containers)
- `storage/` — Blob I/O, content-addressed storage (CAS), FastCDC chunking, tiering
- `config.rs` — Environment-based configuration
- `tui/` — Ratatui-based setup wizard

### CLI Client Layout (`client/cli/src/`)
- `api/rest.rs` — REST client (reqwest) for login, server info
- `api/grpc.rs` — gRPC stubs (tonic, compiled from proto by `build.rs`) — present but REST is the primary transport
- `sync.rs` — Sync engine with file watching and delta sync
- `watch.rs` — Filesystem watcher (notify crate)
- `db.rs` — Local SQLite for tracking sync state
- `config.rs` — TOML config at `~/.config/entanglement/tangle/config.toml`

### macOS Client (`client/macos/Entanglement/`)
Uses a FileProvider extension for virtual filesystem integration. Key files:
- `EntanglementFileProvider/FileProviderExtension.swift` + feature extensions (Create, Modify, Fetch, Delete, Reparent)
- `EntanglementFileProvider/SyncEngine.swift` — Delta sync logic
- `EntanglementFileProvider/ChunkingEngine.swift` — Client-side chunking
- `EntanglementFileProvider/Crypto/BLAKE3.swift` — BLAKE3 hashing

### Web Frontend (`server/web/`)
Vanilla JS single-page app. `app.js` (~91KB) handles file browsing, uploads, admin dashboard, and WebSocket sync. No build step — served directly by darkhttpd in Docker or the Rust server.

### Database
PostgreSQL 16. Migrations in `server/migrations/` (10 files, run automatically). Key tables: `users`, `files`, `versions`, `chunks`, `blob_containers`, `version_chunks`, `sync_cursors`, `conflicts`, `sharing`.

### Communication
- REST API on port 1975 (primary transport for all clients)
- WebSocket at `/ws` for real-time file change notifications
- gRPC stubs exist in CLI client but are unused in practice

### Auth Flow
Username/password login → JWT access token (24h) + refresh token (30d). Admin users can manage other users via `/admin/*` endpoints.

## Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `JWT_SECRET` | auto-generated | JWT signing key (set for production) |
| `DATABASE_URL` | — | PostgreSQL connection string |
| `BLOB_STORAGE_PATH` | `./data/blobs` | Where chunk containers are stored on disk |
| `REST_PORT` | `1975` | API port |
| `WEB_PORT` | `3000` | Web UI port (Docker only, via darkhttpd) |
| `CORS_ORIGINS` | `localhost:3000` | Allowed CORS origins |
| `MAX_UPLOAD_SIZE` | 1GB | Upload size limit |

## CI

GitHub Actions (`.github/workflows/ci.yml`): server tests (with PostgreSQL service), CLI build+test, Docker build. macOS builds triggered on `client/macos/**` changes (`.github/workflows/macos.yml`).
