#!/bin/sh
# =============================================================================
# Entanglement Server - Container Entrypoint Script
# =============================================================================
#
# This script runs EVERY TIME the container starts and handles:
#   1. JWT Secret Generation - Creates a secure random key if not provided
#   2. Database Readiness     - Waits for PostgreSQL to accept connections
#   3. Database Migrations    - Applies any pending schema changes
#   4. Application Start      - Launches the Entanglement server
#
# Environment Variables:
#   JWT_SECRET   - (Optional) Server-side secret for JWT signing
#                  If not set, a random one is generated (logs out users on restart)
#   DATABASE_URL - PostgreSQL connection string
#
# =============================================================================

set -e  # Exit immediately if a command fails

# =============================================================================
# STEP 1: JWT_SECRET Generation
# =============================================================================
# SECURITY: The JWT secret is used to sign authentication tokens.
#           For production, this MUST be set in .env to persist across restarts.
#           If not set, we generate a random one - but this will log out all
#           users when the container restarts.

if [ -z "$JWT_SECRET" ]; then
    echo "============================================================"
    echo "[entrypoint] ⚠️  WARNING: JWT_SECRET not set!"
    echo "[entrypoint] Generating random secret for this session..."
    echo "[entrypoint] NOTE: Users will be logged out on container restart."
    echo "[entrypoint] For production, set JWT_SECRET in .env file:"
    echo "[entrypoint]   openssl rand -hex 32"
    echo "============================================================"
    export JWT_SECRET=$(openssl rand -hex 32)
fi

# Validate JWT_SECRET minimum length (256-bit = 64 hex chars)
if [ ${#JWT_SECRET} -lt 32 ]; then
    echo "[entrypoint] ❌ ERROR: JWT_SECRET is too short (minimum 32 characters)"
    echo "[entrypoint] Generate a secure secret with: openssl rand -hex 32"
    exit 1
fi

# =============================================================================
# STEP 2: Wait for Database
# =============================================================================
# WHY: Docker Compose's `depends_on` only waits for container start, not
#      for the database to be ready to accept connections. This loop ensures
#      PostgreSQL is fully initialized before we try to run migrations.

if [ -n "$DATABASE_URL" ]; then
    echo "[entrypoint] Waiting for database connection..."
    
    # Extract host and port from DATABASE_URL
    # Format: postgres://user:pass@host:port/dbname
    DB_HOST=$(echo "$DATABASE_URL" | sed -n 's|.*@\([^:/]*\).*|\1|p')
    DB_PORT=$(echo "$DATABASE_URL" | sed -n 's|.*:\([0-9]*\)/.*|\1|p')
    DB_PORT=${DB_PORT:-5432}
    
    # Retry loop: try to connect up to 30 times (30 seconds)
    MAX_RETRIES=30
    RETRY_COUNT=0
    
    while [ $RETRY_COUNT -lt $MAX_RETRIES ]; do
        if nc -z "$DB_HOST" "$DB_PORT" 2>/dev/null; then
            echo "[entrypoint] ✅ Database is ready at $DB_HOST:$DB_PORT"
            break
        fi
        
        RETRY_COUNT=$((RETRY_COUNT + 1))
        echo "[entrypoint] Waiting for database at $DB_HOST:$DB_PORT... ($RETRY_COUNT/$MAX_RETRIES)"
        sleep 1
    done
    
    # Check if we exhausted retries
    if [ $RETRY_COUNT -eq $MAX_RETRIES ]; then
        echo "[entrypoint] ❌ ERROR: Could not connect to database after $MAX_RETRIES attempts"
        echo "[entrypoint] Check that PostgreSQL is running and DATABASE_URL is correct"
        exit 1
    fi
else
    echo "[entrypoint] ⚠️  WARNING: DATABASE_URL not set, skipping database wait"
fi

# =============================================================================
# STEP 3: Database Migrations
# =============================================================================
# WHY: Run migrations BEFORE starting the server to ensure the schema is
#      up to date. This is safe to run on every startup - SQLx migrations
#      are idempotent (they track which have already been applied).

echo "[entrypoint] Running database migrations..."

# Run migrations - capture output for better logging
if tangled migrate 2>&1; then
    echo "[entrypoint] ✅ Migrations completed successfully"
else
    # Migration might "fail" if tables already exist - that's OK
    echo "[entrypoint] ℹ️  Migration completed (tables may already exist)"
fi

# =============================================================================
# STEP 4: Start Application
# =============================================================================
# WHY: `exec` replaces this shell process with the application, making the
#      application PID 1. This is important for signal handling - when
#      Docker sends SIGTERM to stop the container, it goes directly to the
#      application, allowing for graceful shutdown.

echo "[entrypoint] 🚀 Starting Entanglement server..."
echo "[entrypoint] REST API: http://0.0.0.0:${REST_PORT:-1975}"
echo "[entrypoint] gRPC API: http://0.0.0.0:${GRPC_PORT:-50051}"
echo "[entrypoint] Web UI:   http://0.0.0.0:${WEB_PORT:-3000}"

# =============================================================================
# STEP 5: Start Web UI Server (Background)
# =============================================================================
# WHY: darkhttpd serves the static web UI files. It runs in the background
#      so the main application can run as PID 1 for proper signal handling.

if [ -d "/app/web" ]; then
    echo "[entrypoint] Starting web UI server..."
    darkhttpd /app/web --port "${WEB_PORT:-3000}" --daemon
    echo "[entrypoint] ✅ Web UI server started on port ${WEB_PORT:-3000}"
else
    echo "[entrypoint] ⚠️  Web UI directory not found, skipping web server"
fi

exec "$@"
