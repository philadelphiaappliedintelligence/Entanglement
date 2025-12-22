# Entanglement

A secure file synchronization service with a macOS client and Rust server.

## Quick Start with Docker

```bash
# Clone the repository
git clone https://github.com/your-org/entanglement.git
cd entanglement

# Copy and configure environment (recommended for production)
cp env.example .env
# Edit .env and set JWT_SECRET for production:
# JWT_SECRET=$(openssl rand -hex 32)

# Start the services
docker compose up -d

# Check status
docker compose ps
docker compose logs -f server
```

The server will be available at:
- **Web UI:** http://localhost:3000
- **REST API:** http://localhost:1975
- **gRPC:** localhost:50051

## Configuration

### Environment Variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `JWT_SECRET` | **Yes** (production) | Auto-generated | Secret for signing JWT tokens. Generate with `openssl rand -hex 32` |
| `POSTGRES_PASSWORD` | No | `entanglement` | Database password |
| `REST_PORT` | No | `1975` | REST API port |
| `GRPC_PORT` | No | `50051` | gRPC port |
| `WEB_PORT` | No | `3000` | Web UI port |

> ⚠️ **Security Note:** If `JWT_SECRET` is not set, a random secret is generated on each container restart, logging out all users. Set a persistent secret in `.env` for production.

## Development Setup

### Server (Rust)

```bash
cd server

# Set required environment variable
export JWT_SECRET=$(openssl rand -hex 32)
export DATABASE_URL=postgres://entanglement:entanglement@localhost:5432/entanglement

# Start postgres (if not using Docker)
docker compose up -d postgres

# Run the server
cargo run -- serve
```

### Client (macOS)

Open `client/macos/Entanglement/Entanglement.xcodeproj` in Xcode and build.

## Security

This project follows security best practices:

- ✅ JWT authentication with configurable expiration
- ✅ Refresh token rotation
- ✅ Rate limiting on authentication endpoints
- ✅ CORS restrictions
- ✅ Path traversal prevention
- ✅ File ownership enforcement
- ✅ Secure credential storage (Keychain on macOS)
- ✅ Sanitized error messages (no internal details leaked)

## License

MIT
