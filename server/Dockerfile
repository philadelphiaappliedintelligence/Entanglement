# =============================================================================
# Entanglement Server - Production Dockerfile
# =============================================================================
#
# This Dockerfile uses a multi-stage build for optimization:
#   1. Builder stage: Compiles with all dev dependencies (large)
#   2. Runner stage:  Minimal Alpine image with only runtime deps (small)
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
# STAGE 1: Builder
# =============================================================================
# WHY: Rust nightly is required for edition2024 support
# WHY: Alpine for smaller base image and static musl builds
FROM rustlang/rust:nightly-alpine AS builder

# WHY: These packages are needed for:
#   - musl-dev: C compiler for native dependencies
#   - openssl-dev/openssl-libs-static: TLS support for network libs
#   - pkgconfig: Locating system libraries during build
RUN apk add --no-cache \
    musl-dev \
    openssl-dev \
    openssl-libs-static \
    pkgconfig

WORKDIR /app

# =============================================================================
# DEPENDENCY CACHING OPTIMIZATION
# =============================================================================
# WHY: Docker caches each layer. By copying ONLY dependency files first,
#      we cache the expensive `cargo build` step. Source code changes won't
#      invalidate this cache layer.
COPY Cargo.toml Cargo.lock ./

# WHY: Cargo needs a src/main.rs to build dependencies. We create a dummy
#      that will be replaced later. This builds all crate dependencies.
RUN mkdir -p src && echo "fn main() {}" > src/main.rs

# WHY: `--locked` ensures Cargo.lock is honored exactly. The `|| true`
#      allows this dummy build to fail gracefully (some features may require
#      the real code). The key is it downloads/compiles dependencies.
RUN cargo build --release --locked 2>/dev/null || true

# =============================================================================
# SOURCE CODE BUILD
# =============================================================================
# WHY: Now we copy the actual source. Only this layer and below will
#      rebuild when code changes - dependencies are already cached above.
COPY src/ src/
COPY migrations/ migrations/

# WHY: Touch main.rs to ensure Cargo sees it as "changed" and rebuilds
#      (the dummy main.rs had the same mtime as the cached layer)
RUN touch src/main.rs

# WHY: Final release build with locked dependencies
RUN cargo build --release --locked


# =============================================================================
# STAGE 2: Runtime
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
#   - 8080: REST API
#   - 3000: Web UI (served by darkhttpd)
EXPOSE 8080 3000

# WHY: entrypoint.sh handles:
#   1. JWT_SECRET generation if not provided
#   2. Waiting for database to be ready
#   3. Running database migrations
#   4. Starting the application
ENTRYPOINT ["/entrypoint.sh"]

# WHY: Default command passed to entrypoint
#   --foreground prevents daemonization (required for Docker)
CMD ["tangled", "serve", "--foreground"]
