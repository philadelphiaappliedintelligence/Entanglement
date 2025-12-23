# Design Document: Codebase Refactoring

## Overview

This design document describes the technical approach for refactoring the Entanglement codebase to reduce complexity, eliminate dead code, consolidate duplicate implementations, and harden production deployment. The refactoring is organized into 5 phases that can be executed incrementally with rollback capability.

The current codebase has solid foundations but accumulated technical debt from rapid iteration:
- Unused gRPC server running alongside REST API
- Dual storage systems (BlobStore + BlobManager)
- Duplicate sync logic between macOS app and FileProvider extension
- Missing transaction safety for multi-step database operations
- No health check endpoint for container orchestration

## Architecture

### Current State

```
┌─────────────────────────────────────────────────────────────┐
│                        Server                                │
├─────────────────────────────────────────────────────────────┤
│  ┌──────────┐  ┌──────────┐  ┌──────────┐                   │
│  │  gRPC    │  │  REST    │  │WebSocket │                   │
│  │ (unused) │  │   API    │  │   Hub    │                   │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘                   │
│       │             │             │                          │
│       └─────────────┼─────────────┘                          │
│                     ▼                                        │
│              ┌──────────────┐                                │
│              │   AppState   │                                │
│              └──────┬───────┘                                │
│         ┌───────────┼───────────┐                            │
│         ▼           ▼           ▼                            │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐                     │
│  │BlobStore │ │BlobManager│ │ Database │                     │
│  │ (legacy) │ │(container)│ │  (sqlx)  │                     │
│  └──────────┘ └──────────┘ └──────────┘                     │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│                     macOS Client                             │
├─────────────────────────────────────────────────────────────┤
│  ┌─────────────────────┐    ┌─────────────────────┐         │
│  │     Main App        │    │  FileProvider Ext   │         │
│  ├─────────────────────┤    ├─────────────────────┤         │
│  │ SyncService (757L)  │    │ SyncEngine (699L)   │         │
│  │ Chunking.swift      │    │ ChunkingEngine      │         │
│  │ HashScanner         │    │ (inline hashing)    │         │
│  │ SyncUploader        │    │ (inline upload)     │         │
│  └─────────────────────┘    └─────────────────────┘         │
│              │                        │                      │
│              └────────────┬───────────┘                      │
│                           ▼                                  │
│                  ┌─────────────────┐                         │
│                  │ EntanglementCore│                         │
│                  │   (APIClient)   │                         │
│                  └─────────────────┘                         │
└─────────────────────────────────────────────────────────────┘
```

### Target State

```
┌─────────────────────────────────────────────────────────────┐
│                        Server                                │
├─────────────────────────────────────────────────────────────┤
│  ┌──────────┐  ┌──────────┐                                 │
│  │  REST    │  │WebSocket │                                 │
│  │   API    │  │   Hub    │                                 │
│  └────┬─────┘  └────┬─────┘                                 │
│       │             │                                        │
│       └──────┬──────┘                                        │
│              ▼                                               │
│       ┌──────────────┐                                       │
│       │   AppState   │                                       │
│       └──────┬───────┘                                       │
│              │                                               │
│    ┌─────────┴─────────┐                                    │
│    ▼                   ▼                                     │
│ ┌──────────┐    ┌──────────┐                                │
│ │BlobManager│    │ Database │                                │
│ │ (unified) │    │  (sqlx)  │                                │
│ └──────────┘    └──────────┘                                │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│                     macOS Client                             │
├─────────────────────────────────────────────────────────────┤
│  ┌─────────────────────┐    ┌─────────────────────┐         │
│  │     Main App        │    │  FileProvider Ext   │         │
│  ├─────────────────────┤    ├─────────────────────┤         │
│  │ SyncService         │    │ SyncEngine          │         │
│  │ (orchestration)     │    │ (orchestration)     │         │
│  └─────────┬───────────┘    └─────────┬───────────┘         │
│            │                          │                      │
│            └────────────┬─────────────┘                      │
│                         ▼                                    │
│              ┌─────────────────────┐                         │
│              │  EntanglementCore   │                         │
│              ├─────────────────────┤                         │
│              │ SyncCore/           │                         │
│              │  ├─ ChunkingCore    │                         │
│              │  ├─ HashingCore     │                         │
│              │  ├─ UploadCore      │                         │
│              │  └─ ManifestCore    │                         │
│              │ APIClient (retry)   │                         │
│              └─────────────────────┘                         │
└─────────────────────────────────────────────────────────────┘
```

## Components and Interfaces

### Phase 1: Server gRPC Removal

#### Files to Delete
- `server/src/api/grpc/` directory (mod.rs, sync.rs)
- `server/proto/` directory (sync.proto)
- `server/build.rs` (gRPC code generation)

#### Cargo.toml Changes
```toml
# Remove these dependencies:
# tonic = "0.11"
# prost = "0.12"
# sha2 = "0.10"  # BLAKE3 is used instead

# Remove build-dependencies:
# tonic-build = "0.11"
```

#### main.rs Changes
```rust
// Remove gRPC server startup
async fn run_server(config: Config) -> anyhow::Result<()> {
    // ... initialization ...
    
    // REMOVE: gRPC server spawn
    // let grpc_handle = tokio::spawn(async move { ... });
    
    // Keep only REST server
    let rest_handle = tokio::spawn(async move {
        tracing::info!("REST listening on {}", rest_addr);
        api::rest::serve(rest_addr, rest_state).await
    });
    
    // REMOVE: tokio::select! with grpc_handle
    rest_handle.await??;
    
    Ok(())
}
```

#### api/mod.rs Changes
```rust
// Remove: pub mod grpc;
pub mod rest;
pub mod ws;
```

### Phase 2: Storage Consolidation

#### BlobManager Extension
```rust
impl BlobManager {
    /// Read a legacy standalone blob for backward compatibility
    /// Legacy blobs use 2-char directory sharding: {base}/{hash[0..2]}/{hash}
    pub fn read_legacy_blob(&self, hash: &str) -> Result<Vec<u8>> {
        if hash.len() < 4 {
            return Err(anyhow!("Invalid hash format"));
        }
        
        // Construct legacy path
        let shard = &hash[..2];
        let legacy_path = self.base_path
            .parent()
            .unwrap_or(&self.base_path)
            .join(shard)
            .join(hash);
        
        if legacy_path.exists() {
            let content = std::fs::read(&legacy_path)?;
            Ok(content)
        } else {
            Err(anyhow!("Blob not found: {}", hash))
        }
    }
    
    /// Check if a blob exists in either container or legacy storage
    pub fn blob_exists(&self, hash: &str) -> Result<bool> {
        // Check container storage first (preferred)
        // Then fall back to legacy storage
    }
}
```

#### AppState Simplification
```rust
#[derive(Clone)]
pub struct AppState {
    pub db: DbPool,
    // REMOVE: pub blob_store: Arc<BlobStore>,
    pub blob_manager: Arc<BlobManager>,
    pub config: Config,
    pub sync_hub: SyncHub,
}
```

### Phase 3: Client Sync Consolidation

#### New SyncCore Module Structure
```
EntanglementCore/SyncCore/
├── SyncTypes.swift      # (existing) Shared types
├── SyncProgress.swift   # (existing) Progress reporting
├── ChunkingCore.swift   # NEW: Shared FastCDC + BLAKE3
├── HashingCore.swift    # NEW: Shared file hashing
├── UploadCore.swift     # NEW: Shared chunk upload
└── ManifestCore.swift   # NEW: Shared manifest handling
```

#### ChunkingCore Interface
```swift
/// Shared chunking implementation using FastCDC algorithm
public actor ChunkingCore {
    /// Chunk configuration matching server-side settings
    public struct Config: Sendable {
        public let minChunkSize: Int    // 256 KB
        public let avgChunkSize: Int    // 1 MB
        public let maxChunkSize: Int    // 4 MB
        
        public static let `default` = Config(
            minChunkSize: 256 * 1024,
            avgChunkSize: 1024 * 1024,
            maxChunkSize: 4 * 1024 * 1024
        )
    }
    
    /// Chunk a file into content-defined chunks
    public func chunkFile(
        at url: URL,
        config: Config = .default
    ) async throws -> ChunkManifest
    
    /// Chunk in-memory data
    public func chunkData(
        _ data: Data,
        config: Config = .default
    ) -> [ChunkInfo]
}
```

#### HashingCore Interface
```swift
/// Shared BLAKE3 hashing implementation
public enum HashingCore {
    /// Compute BLAKE3 hash of file at URL
    public static func hashFile(at url: URL) async throws -> String
    
    /// Compute BLAKE3 hash of data
    public static func hashData(_ data: Data) -> String
    
    /// Compute BLAKE3 hash incrementally
    public static func createHasher() -> BLAKE3Hasher
}
```

#### UploadCore Interface
```swift
/// Shared chunk upload logic with deduplication
public actor UploadCore {
    private let apiClient: APIClient
    private let config: SyncConfig
    
    /// Upload chunks with server-side deduplication
    public func uploadChunks(
        manifest: ChunkManifest,
        fileURL: URL,
        progress: SyncProgressReporter?
    ) async throws -> UploadResult
}
```

### Phase 4: Transaction Safety

#### Transaction Wrapper Pattern
```rust
// In v1.rs - file creation with version
pub async fn create_file_version(
    State(state): State<AppState>,
    Json(request): Json<CreateFileRequest>,
) -> Result<Json<CreateFileResponse>, ApiError> {
    // Start transaction
    let mut tx = state.db.begin().await?;
    
    // All operations use the transaction
    let file = db::files::upsert_file(&mut *tx, &request.path, user_id).await?;
    let version = db::versions::create_version(
        &mut *tx,
        file.id,
        &request.content_hash,
        request.size_bytes,
    ).await?;
    db::files::set_current_version(&mut *tx, file.id, version.id).await?;
    
    // Commit atomically
    tx.commit().await?;
    
    Ok(Json(CreateFileResponse { file_id: file.id, version_id: version.id }))
}
```

### Phase 4: Health Check Endpoint

#### health.rs Implementation
```rust
use axum::{extract::State, response::IntoResponse, Json};
use serde::Serialize;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub database: bool,
    pub storage: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub async fn health_check(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let db_ok = sqlx::query("SELECT 1")
        .fetch_one(&state.db)
        .await
        .is_ok();
    
    let storage_ok = state.blob_manager.base_path().exists();
    
    let status = if db_ok && storage_ok { "ok" } else { "degraded" };
    let status_code = if db_ok && storage_ok { 200 } else { 503 };
    
    (
        axum::http::StatusCode::from_u16(status_code).unwrap(),
        Json(HealthResponse {
            status,
            database: db_ok,
            storage: storage_ok,
            error: None,
        }),
    )
}
```

### Phase 4: Client Retry Logic

#### APIClient Retry Extension
```swift
extension APIClient {
    /// Perform request with exponential backoff retry
    private func requestWithRetry<T: Decodable>(
        _ request: URLRequest,
        maxRetries: Int = 3
    ) async throws -> T {
        var lastError: Error?
        
        for attempt in 0..<maxRetries {
            do {
                let (data, response) = try await session.data(for: request)
                
                guard let httpResponse = response as? HTTPURLResponse else {
                    throw APIError.networkError(NSError(domain: "Invalid response", code: 0))
                }
                
                // Don't retry client errors (4xx)
                if httpResponse.statusCode >= 400 && httpResponse.statusCode < 500 {
                    throw APIError.serverError("HTTP \(httpResponse.statusCode)")
                }
                
                guard httpResponse.statusCode == 200 else {
                    throw APIError.serverError("HTTP \(httpResponse.statusCode)")
                }
                
                return try JSONDecoder().decode(T.self, from: data)
                
            } catch {
                lastError = error
                
                // Don't retry non-transient errors
                if !isTransientError(error) {
                    throw error
                }
                
                // Exponential backoff: 1s, 2s, 4s
                if attempt < maxRetries - 1 {
                    let delay = UInt64(pow(2.0, Double(attempt))) * 1_000_000_000
                    try await Task.sleep(nanoseconds: delay)
                    Log.debug("Retry attempt \(attempt + 1) after error: \(error)", category: "API")
                }
            }
        }
        
        throw lastError ?? APIError.networkError(NSError(domain: "Unknown", code: 0))
    }
    
    private func isTransientError(_ error: Error) -> Bool {
        if let apiError = error as? APIError {
            switch apiError {
            case .networkError, .timeout:
                return true
            case .unauthorized, .notConfigured, .invalidURL:
                return false
            default:
                return false
            }
        }
        
        // URLSession errors that are transient
        let nsError = error as NSError
        let transientCodes = [
            NSURLErrorTimedOut,
            NSURLErrorCannotConnectToHost,
            NSURLErrorNetworkConnectionLost,
            NSURLErrorNotConnectedToInternet,
        ]
        return transientCodes.contains(nsError.code)
    }
}
```

## Data Models

### No Schema Changes Required

The refactoring does not modify database schemas. All changes are to application code only.

### Configuration Changes

#### Remove from Config
```rust
// config.rs - remove grpc_port
pub struct Config {
    pub database_url: String,
    pub rest_port: u16,
    // REMOVE: pub grpc_port: u16,
    pub blob_storage_path: String,
    pub jwt_secret: String,
}
```

#### Docker Compose Changes
```yaml
# docker-compose.yml
services:
  server:
    ports:
      - "${REST_PORT:-8080}:8080"
      # REMOVE: - "${GRPC_PORT:-50051}:50051"
      - "${WEB_PORT:-3000}:3000"
```



## Correctness Properties

*A property is a characteristic or behavior that should hold true across all valid executions of a system—essentially, a formal statement about what the system should do. Properties serve as the bridge between human-readable specifications and machine-verifiable correctness guarantees.*

### Property 1: File Upsert Functionality Preservation

*For any* file path and user context, upserting a file using the consolidated `upsert_file` function SHALL produce the same database state as the original multi-variant functions would have produced.

**Validates: Requirements 2.4**

### Property 2: Legacy Blob Backward Compatibility

*For any* blob hash that exists in legacy 2-char sharded storage, the BlobManager SHALL successfully read and return the blob content identical to what BlobStore would have returned.

**Validates: Requirements 4.4**

### Property 3: Chunking Consistency

*For any* file data, chunking via SyncCore.ChunkingCore SHALL produce identical chunk boundaries, hashes, and manifests regardless of whether invoked from the main app or FileProvider extension.

**Validates: Requirements 5.5, 5.6**

### Property 4: Sync Functionality Preservation

*For any* sync operation (upload or download), the consolidated SyncCore-based implementation SHALL produce the same server state and local file state as the original duplicate implementations.

**Validates: Requirements 5.7**

### Property 5: Transaction Atomicity

*For any* multi-step database operation (file creation with version), if any step fails, the database state SHALL be unchanged from before the operation began. If all steps succeed, all changes SHALL be visible atomically.

**Validates: Requirements 6.1, 6.2, 6.3, 6.4**

### Property 6: Transient Error Retry

*For any* API request that fails with a transient network error (timeout, connection lost, etc.), the client SHALL automatically retry the request before reporting failure.

**Validates: Requirements 8.1**

### Property 7: Exponential Backoff Timing

*For any* sequence of retry attempts, the delay between attempt N and attempt N+1 SHALL be approximately 2^N seconds (exponential backoff).

**Validates: Requirements 8.2**

### Property 8: Non-Transient Error No-Retry

*For any* API request that fails with a non-transient error (401 Unauthorized, 403 Forbidden, 404 Not Found), the client SHALL NOT retry and SHALL immediately report the error.

**Validates: Requirements 8.5**

## Error Handling

### Server Error Handling

#### Transaction Rollback
```rust
// All multi-step operations use transactions
let mut tx = pool.begin().await?;

match perform_operations(&mut tx).await {
    Ok(result) => {
        tx.commit().await?;
        Ok(result)
    }
    Err(e) => {
        // Transaction automatically rolls back on drop
        Err(e)
    }
}
```

#### Health Check Degraded State
```rust
// Return 503 with details when degraded
if !db_ok || !storage_ok {
    return (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(HealthResponse {
            status: "degraded",
            database: db_ok,
            storage: storage_ok,
            error: Some("Service dependencies unavailable".into()),
        }),
    );
}
```

### Client Error Handling

#### Retry Classification
```swift
// Transient errors (retry)
- NSURLErrorTimedOut
- NSURLErrorCannotConnectToHost
- NSURLErrorNetworkConnectionLost
- NSURLErrorNotConnectedToInternet
- HTTP 500, 502, 503, 504

// Non-transient errors (no retry)
- HTTP 400 Bad Request
- HTTP 401 Unauthorized
- HTTP 403 Forbidden
- HTTP 404 Not Found
- HTTP 409 Conflict
```

#### User-Facing Error Messages
```swift
// Map technical errors to user-friendly messages
switch error {
case .networkError:
    return "Unable to connect to server. Check your internet connection."
case .unauthorized:
    return "Session expired. Please log in again."
case .serverError(let msg):
    return "Server error: \(msg)"
}
```

## Testing Strategy

### Dual Testing Approach

This refactoring uses both unit tests and property-based tests:

- **Unit tests**: Verify specific examples, edge cases, and error conditions
- **Property tests**: Verify universal properties across all inputs

### Server Tests (Rust)

#### Unit Tests
- Health endpoint returns correct status codes
- Transaction rollback on failure
- Legacy blob path construction
- gRPC removal doesn't break REST

#### Property Tests (using proptest)
```rust
// Property 2: Legacy blob compatibility
proptest! {
    #[test]
    fn legacy_blob_read_matches_original(hash in "[a-f0-9]{64}") {
        // Setup: Create blob in legacy storage
        // Test: Read via BlobManager matches original
    }
}

// Property 5: Transaction atomicity
proptest! {
    #[test]
    fn failed_transaction_leaves_db_unchanged(
        path in "/.+",
        fail_at_step in 0..3usize
    ) {
        // Setup: Record initial DB state
        // Test: Inject failure at step, verify DB unchanged
    }
}
```

### Client Tests (Swift)

#### Unit Tests
- Retry logic respects max attempts
- Exponential backoff timing
- Error classification (transient vs non-transient)
- ChunkingCore produces valid manifests

#### Property Tests (using SwiftCheck)
```swift
// Property 3: Chunking consistency
property("Chunking same data produces same result") <- forAll { (data: Data) in
    let result1 = ChunkingCore.chunkData(data)
    let result2 = ChunkingCore.chunkData(data)
    return result1 == result2
}

// Property 8: Non-transient errors not retried
property("4xx errors not retried") <- forAll(Gen.choose(400, 499)) { statusCode in
    let error = APIError.serverError("HTTP \(statusCode)")
    return !isTransientError(error)
}
```

### Integration Tests

#### Phase 1 Verification
```bash
# Server starts without gRPC
cargo run -- serve --foreground &
sleep 2
! nc -z localhost 50051  # gRPC port should be closed
curl -s http://localhost:8080/server/info | jq .  # REST works
```

#### Phase 2 Verification
```bash
# Export command works with BlobManager
tangled export ./backup
diff -r ./backup/current ./original_files
```

#### Phase 4 Verification
```bash
# Health endpoint
curl -s http://localhost:8080/healthz | jq .
# Should return: {"status":"ok","database":true,"storage":true}
```

### Test Configuration

- Property tests: Minimum 100 iterations per property
- Each property test tagged with: `Feature: codebase-refactoring, Property N: {description}`
- Use proptest (Rust) and SwiftCheck (Swift) for property-based testing
