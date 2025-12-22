# Entanglement Server - Installation Guide

## Prerequisites

- Docker & Docker Compose
- Linux server (Ubuntu/Debian recommended)

## Quick Start

```bash
# 1. Create environment file
cat > .env << EOF
REST_PORT=1975
SERVER_NAME=My Server
MAX_UPLOAD_SIZE=10737418240
JWT_SECRET=$(openssl rand -hex 32)
POSTGRES_PASSWORD=$(openssl rand -hex 16)
EOF

# 2. Start services
docker compose up -d --build

# 3. Create first user
docker exec -it entanglement-server tangled user create --email you@example.com
```

## Verify Installation

```bash
curl http://localhost:1975/server/info
```

## Management Commands

| Command | Description |
|---------|-------------|
| `docker compose logs -f server` | View server logs |
| `docker compose down` | Stop services |
| `docker compose up -d --build` | Rebuild & restart |
| `docker compose down -v` | **Delete all data** |

## Configuration Options (.env)

| Variable | Default | Description |
|----------|---------|-------------|
| `REST_PORT` | 8080 | HTTP API port |
| `GRPC_PORT` | 50051 | gRPC sync port |
| `SERVER_NAME` | Entanglement | Display name |
| `MAX_UPLOAD_SIZE` | 1073741824 | Max file size (bytes) |
| `JWT_SECRET` | *generated* | **Required** - auth signing key |
| `POSTGRES_PASSWORD` | entanglement | Database password |

## Firewall

Open these ports:
- `REST_PORT` (default 8080) - REST API
- `GRPC_PORT` (default 50051) - Sync protocol
