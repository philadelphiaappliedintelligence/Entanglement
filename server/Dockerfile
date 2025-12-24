# =============================================================================
# Entanglement Server - Production Dockerfile
# =============================================================================
#
# This Dockerfile uses cargo-chef for optimal dependency caching:
#   1. Chef stage:   Installs cargo-chef
#   2. Planner stage: Analyzes dependencies and creates a "recipe"
#   3. Builder stage: Builds dependencies (cached), then builds app
#   4. Runner stage:  Minimal Alpine image with only runtime deps
#
# Security Features:
#   - Non-root user (entanglement:entanglement, uid/gid 1000)
#   - Minimal attack surface (Alpine Linux)
#   - No secrets baked into image (injected at runtime via entrypoint.sh)
#
# Build: docker build -t entanglement-server .
# Run:   docker run -e JWT_SECRET=xxx -e DATABASE_URL=xxx entanglement-server
#
# =============================================================================

# =============================================================================
# STAGE 1: Chef (install cargo-chef)
# =============================================================================
FROM rustlang/rust:nightly-alpine AS chef

RUN apk add --no-cache musl-dev openssl-dev openssl-libs-static pkgconfig
RUN cargo install cargo-chef --locked

WORKDIR /app

# =============================================================================
# STAGE 2: Planner (analyze dependencies)
# =============================================================================
FROM chef AS planner

COPY Cargo.toml Cargo.lock ./
COPY src/ src/

# Create a "recipe" file that captures all dependency information
RUN cargo chef prepare --recipe-path recipe.json

# =============================================================================
# STAGE 3: Builder (build dependencies, then app)
# =============================================================================
FROM chef AS builder

# Copy the recipe from planner stage
COPY --from=planner /app/recipe.json recipe.json

# Build ONLY dependencies - this layer is cached unless Cargo.toml/Cargo.lock change
RUN cargo chef cook --release --recipe-path recipe.json

# Now copy source code and build the actual application
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY migrations/ migrations/

# Build the application (dependencies are already compiled and cached)
RUN cargo build --release --locked

# =============================================================================
# STAGE 4: Runtime
# =============================================================================
# WHY: Start fresh with a minimal base image. The builder stage adds ~1GB
#      of Rust/build dependencies we don't need at runtime.
FROM alpine:3.20

# WHY: Minimal runtime dependencies:
#   - ca-certificates: HTTPS certificate validation
#   - openssl: For any TLS operations at runtime
#   - libgcc: Required by some Rust binaries
#   - netcat-openbsd: For database connectivity checks in entrypoint
#   - darkhttpd: Lightweight static file server for web UI
RUN apk add --no-cache \
    ca-certificates \
    openssl \
    libgcc \
    netcat-openbsd \
    darkhttpd

# =============================================================================
# SECURITY: Non-Root User
# =============================================================================
# WHY: Running as root inside containers is a security anti-pattern.
#      If the container is compromised, the attacker has root access
#      to the container filesystem. Using a non-root user limits damage.
# NOTE: User must be created BEFORE COPY --chown can use it
RUN addgroup -g 1000 entanglement && \
    adduser -u 1000 -G entanglement -s /bin/sh -D entanglement && \
    mkdir -p /data/blobs && \
    chown -R entanglement:entanglement /data

# WHY: Copy ONLY the compiled binary from builder (not source/dependencies)
COPY --from=builder /app/target/release/tangled /usr/local/bin/tangled

# WHY: Copy entrypoint script with executable permissions
COPY --chmod=755 entrypoint.sh /entrypoint.sh

# WHY: Copy web UI files for serving via darkhttpd
COPY --chown=entanglement:entanglement web /app/web

# WHY: Switch to non-root user for all subsequent commands and runtime
USER entanglement

WORKDIR /app

# WHY: Expose the ports the server listens on
#   - 1975: REST API
#   - 3000: Web UI (served by darkhttpd)
EXPOSE 1975 3000

# WHY: entrypoint.sh handles:
#   1. JWT_SECRET generation if not provided
#   2. Waiting for database to be ready
#   3. Running database migrations
#   4. Starting the application
ENTRYPOINT ["/entrypoint.sh"]

# WHY: Default command passed to entrypoint
#   --foreground prevents daemonization (required for Docker)
CMD ["tangled", "serve", "--foreground"]
