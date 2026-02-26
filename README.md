# Entanglement

A file synchronization service with content-defined chunking for efficient delta sync. Built with a Rust server, Rust CLI client, macOS native client, and vanilla JS web UI.

## Architecture

- **Content-addressed storage** with BLAKE3 hashing and FastCDC chunking
- **Immutable versioning** — every file modification creates a new version
- **Tiered chunking** — 5 tiers from Inline (<4KB) to Jumbo (>5GB) for optimal deduplication
- **Append-only blob containers** (packfiles, 64MB max) with zstd compression
- **Cross-chunk deduplication** by hash and reference counting

### Components

| Component | Language | Description |
|-----------|----------|-------------|
| `server/` (tangled) | Rust | REST API server with PostgreSQL backend |
| `client/cli/` (tangle) | Rust | CLI sync client with background daemon |
| `client/macos/` | Swift/SwiftUI | macOS native client with FileProvider extension |
| `server/web/` | Vanilla JS | Web UI for file browsing, uploads, and admin |

## Quick Start

### Docker (recommended)

```bash
git clone https://github.com/philadelphiaappliedintelligence/Entanglement.git
cd Entanglement

cp .env.example .env
# Edit .env and set JWT_SECRET:
#   JWT_SECRET=$(openssl rand -hex 32)

make up          # Start PostgreSQL + server
make logs        # Tail server logs
```

The server will be available at:
- **REST API:** http://localhost:1975
- **Web UI:** http://localhost:3000

### Create your first user

```bash
# Via CLI (if running locally)
cargo run -p tangled -- user create --username admin --admin

# Or use the TUI setup wizard
cargo run -p tangled -- setup
```

### From source

```bash
# Server
cd server
export JWT_SECRET=$(openssl rand -hex 32)
export DATABASE_URL=postgres://entanglement:entanglement@localhost:5432/entanglement
cargo run -- serve --foreground

# CLI client
cd client/cli
cargo build --release
./target/release/tangle setup    # Configure server URL + login
./target/release/tangle start    # Start background sync daemon
./target/release/tangle status   # Check sync status
./target/release/tangle ls       # List synced files
./target/release/tangle stop     # Stop daemon
```

### macOS client

Open `client/macos/Entanglement/Entanglement.xcodeproj` in Xcode and build.

## Configuration

### Environment Variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `JWT_SECRET` | **Yes** | — | JWT signing key. Generate: `openssl rand -hex 32` |
| `DATABASE_URL` | **Yes** | — | PostgreSQL connection string |
| `BLOB_STORAGE_PATH` | No | `./data/blobs` | Chunk container storage path |
| `REST_PORT` | No | `1975` | REST API port |
| `SERVER_NAME` | No | `Entanglement` | Server display name |
| `CORS_ORIGINS` | No | `localhost:3000` | Allowed CORS origins (comma-separated) |
| `MAX_UPLOAD_SIZE` | No | 1GB | Maximum upload size |

See `.env.example` for a complete template.

## Security

- JWT authentication with HS256 (24h access tokens, 30d refresh tokens)
- Refresh token rotation
- Rate limiting on auth and upload endpoints (tower_governor)
- Path traversal prevention with normalization and character whitelisting
- File ownership enforcement on all user-facing endpoints
- SQL injection protection (escaped LIKE queries)
- CORS, CSP, X-Frame-Options, X-Content-Type-Options headers
- Sanitized error responses (no internal details leaked to clients)
- Argon2 password hashing

## Development

### Build & test

```bash
# Workspace (server + CLI)
cargo build --workspace
cargo test --workspace

# Server only
cd server && cargo test --verbose

# CLI only
cd client/cli && cargo test --verbose

# macOS
cd client/macos/Entanglement
xcodebuild build -project Entanglement.xcodeproj -scheme Entanglement
```

### Docker

```bash
make up       # Start services
make down     # Stop services
make logs     # Tail logs
make clean    # Remove containers and volumes
```

## Contributing

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/my-feature`)
3. Run tests (`cargo test --workspace`)
4. Commit your changes
5. Open a pull request

## License

[MIT](LICENSE)
