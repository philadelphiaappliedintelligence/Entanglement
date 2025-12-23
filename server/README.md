# Entanglement Server

A file sync server with deduplication, delta sync, and chunked transfers.

## Quick Start (Docker)

```bash
# Start the server
docker compose up -d

# Create a user
docker exec -it entanglement-server tangled user create \
  --email user@example.com \
  --password yourpassword

# View logs
docker compose logs -f server
```

The server will be available at:
- **REST API**: http://localhost:1975

## Configuration

Copy `.env.example` to `.env` and configure:

```bash
cp .env.example .env
```

| Variable | Required | Description |
|----------|----------|-------------|
| `JWT_SECRET` | **Yes** (prod) | Secret key for JWT tokens |
| `SERVER_NAME` | No | Display name for your server |
| `REST_PORT` | No | API port (default: 1975) |

## API Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/auth/register` | POST | Create new user |
| `/auth/login` | POST | Login and get token |
| `/server/info` | GET | Server info |
| `/v1/files/list` | GET | List directory |
| `/files/{id}` | GET/DELETE | Get or delete file |
| `/health` | GET | Health check (with DB status) |
| `/health/ready` | GET | Readiness probe |
| `/health/live` | GET | Liveness probe |

## Development

```bash
# Install dependencies and run locally
cargo run -- serve

# Or install the binary
cargo install --path .
tangled serve
```

## Data Persistence

Docker volumes:
- `postgres_data` - Database
- `entanglement_data` - Uploaded files (blobs)

To backup:
```bash
docker compose down
docker run --rm -v server_entanglement_data:/data -v $(pwd):/backup alpine tar czf /backup/data-backup.tar.gz /data
```

