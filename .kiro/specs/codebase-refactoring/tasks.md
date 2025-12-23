# Implementation Plan: Codebase Refactoring

## Overview

This implementation plan breaks down the refactoring into discrete, incremental tasks organized by phase. Each phase can be completed independently and merged separately for easy rollback if issues arise.

## Tasks

- [ ] 1. Phase 1: Prune Dead Code (Server)
  - Remove unused gRPC infrastructure and consolidate file operations
  - _Requirements: 1.1-1.6, 2.1-2.4, 3.1-3.3_

  - [ ] 1.1 Remove gRPC dependencies from Cargo.toml
    - Remove `tonic`, `prost`, `sha2` from dependencies
    - Remove `tonic-build` from build-dependencies
    - Run `cargo build` to verify compilation
    - _Requirements: 1.2, 1.3_

  - [ ] 1.2 Delete gRPC source files
    - Delete `server/src/api/grpc/` directory
    - Delete `server/proto/` directory if it exists
    - Delete `server/build.rs` if it only contains gRPC codegen
    - _Requirements: 1.2_

  - [ ] 1.3 Update api/mod.rs to remove gRPC module
    - Remove `pub mod grpc;` line
    - Verify REST and WebSocket modules still exported
    - _Requirements: 1.4, 1.5_

  - [ ] 1.4 Update main.rs to remove gRPC server startup
    - Remove gRPC server spawn in `run_server()`
    - Remove gRPC from `tokio::select!` block
    - Keep only REST server handle
    - _Requirements: 1.1_

  - [ ] 1.5 Update config.rs to remove gRPC port
    - Remove `grpc_port` field from Config struct
    - Update `show_status()` to not display gRPC port
    - _Requirements: 1.6_

  - [ ] 1.6 Update Docker configuration
    - Remove gRPC port mapping from root `docker-compose.yml`
    - Delete `server/docker-compose.yml` if it exists
    - _Requirements: 3.1, 3.2, 3.3_

  - [ ] 1.7 Write unit tests for Phase 1 changes
    - Test server starts without gRPC listener
    - Test REST API responds correctly
    - Test `tangled status` output format
    - _Requirements: 1.1, 1.4, 1.6_

- [ ] 2. Checkpoint - Phase 1 Verification
  - Ensure server builds and starts correctly
  - Verify REST API and WebSocket still functional
  - Run `cargo test` to ensure no regressions

- [ ] 3. Phase 2: Consolidate Storage Layer
  - Eliminate dual storage systems, use BlobManager exclusively
  - _Requirements: 4.1-4.6_

  - [ ] 3.1 Extend BlobManager with legacy blob support
    - Add `read_legacy_blob()` method to BlobManager
    - Add `blob_exists()` method checking both storage locations
    - Implement legacy path construction (2-char sharding)
    - _Requirements: 4.4, 4.5_

  - [ ] 3.2 Write property test for legacy blob compatibility
    - **Property 2: Legacy Blob Backward Compatibility**
    - **Validates: Requirements 4.4**

  - [ ] 3.3 Update AppState to remove BlobStore
    - Remove `blob_store` field from AppState struct
    - Update `AppState::new()` to not require BlobStore
    - _Requirements: 4.1_

  - [ ] 3.4 Update index command to use BlobManager
    - Modify `index_folder()` to use BlobManager for writes
    - Remove BlobStore instantiation
    - _Requirements: 4.2_

  - [ ] 3.5 Update export command to use BlobManager
    - Modify `export_files()` to use BlobManager for reads
    - Use `read_legacy_blob()` for backward compatibility
    - _Requirements: 4.3_

  - [ ] 3.6 Remove BlobStore module
    - Delete `server/src/storage/blob.rs`
    - Update `server/src/storage/mod.rs` to remove BlobStore export
    - _Requirements: 4.1_

  - [ ] 3.7 Write integration tests for storage consolidation
    - Test index command creates blobs via BlobManager
    - Test export command reads both legacy and container blobs
    - _Requirements: 4.2, 4.3_

- [ ] 4. Checkpoint - Phase 2 Verification
  - Run `tangled index` on test folder
  - Run `tangled export` and verify files recovered
  - Ensure all tests pass

- [ ] 5. Phase 3: Consolidate Client Sync Logic
  - Create shared SyncCore modules for app and extension
  - _Requirements: 5.1-5.7_

  - [ ] 5.1 Create ChunkingCore in SyncCore module
    - Create `EntanglementCore/SyncCore/ChunkingCore.swift`
    - Implement FastCDC chunking algorithm
    - Implement BLAKE3 hash computation for chunks
    - Define ChunkManifest and ChunkInfo types
    - _Requirements: 5.1, 5.2_

  - [ ] 5.2 Write property test for chunking consistency
    - **Property 3: Chunking Consistency**
    - **Validates: Requirements 5.5, 5.6**

  - [ ] 5.3 Create HashingCore in SyncCore module
    - Create `EntanglementCore/SyncCore/HashingCore.swift`
    - Implement file hashing with BLAKE3
    - Implement incremental hasher for large files
    - _Requirements: 5.2_

  - [ ] 5.4 Create UploadCore in SyncCore module
    - Create `EntanglementCore/SyncCore/UploadCore.swift`
    - Implement chunk deduplication check
    - Implement concurrent chunk upload with progress
    - _Requirements: 5.3_

  - [ ] 5.5 Create ManifestCore in SyncCore module
    - Create `EntanglementCore/SyncCore/ManifestCore.swift`
    - Implement manifest serialization/deserialization
    - Implement manifest comparison for delta sync
    - _Requirements: 5.4_

  - [ ] 5.6 Refactor ChunkingEngine to use ChunkingCore
    - Update `EntanglementFileProvider/ChunkingEngine.swift`
    - Delegate to SyncCore.ChunkingCore
    - Remove duplicate FastCDC implementation
    - _Requirements: 5.6_

  - [ ] 5.7 Refactor SyncService to use SyncCore modules
    - Update `Entanglement/Services/SyncService.swift`
    - Use ChunkingCore, HashingCore, UploadCore
    - Remove duplicate implementations
    - _Requirements: 5.5_

  - [ ] 5.8 Refactor SyncEngine to use SyncCore modules
    - Update `EntanglementFileProvider/SyncEngine.swift`
    - Use shared SyncCore modules
    - Keep only FileProvider-specific orchestration
    - _Requirements: 5.6_

  - [ ] 5.9 Write property test for sync functionality preservation
    - **Property 4: Sync Functionality Preservation**
    - **Validates: Requirements 5.7**

- [ ] 6. Checkpoint - Phase 3 Verification
  - Build Entanglement.app in Xcode
  - Test file upload via main app
  - Test file upload via FileProvider
  - Verify both produce identical server state

- [ ] 7. Phase 4: Harden & Observability
  - Add transaction safety, health checks, and retry logic
  - _Requirements: 6.1-6.4, 7.1-7.5, 8.1-8.5_

  - [ ] 7.1 Add transaction wrapper to v1.rs file operations
    - Wrap `create_file_version` in transaction
    - Wrap file update operations in transaction
    - Ensure rollback on any failure
    - _Requirements: 6.1, 6.2_

  - [ ] 7.2 Write property test for transaction atomicity
    - **Property 5: Transaction Atomicity**
    - **Validates: Requirements 6.1, 6.2, 6.3, 6.4**

  - [ ] 7.3 Create health.rs endpoint
    - Create `server/src/api/rest/health.rs`
    - Implement `/healthz` endpoint
    - Check database connectivity
    - Check storage accessibility
    - Return appropriate status codes
    - _Requirements: 7.1, 7.2, 7.3, 7.4, 7.5_

  - [ ] 7.4 Register health endpoint in REST router
    - Add health route to `server/src/api/rest/mod.rs`
    - Ensure no authentication required for health check
    - _Requirements: 7.1_

  - [ ] 7.5 Write unit tests for health endpoint
    - Test returns 200 when healthy
    - Test returns 503 when database unavailable
    - Test JSON response format
    - _Requirements: 7.2, 7.3, 7.4, 7.5_

  - [ ] 7.6 Add retry logic to APIClient
    - Implement `requestWithRetry()` method
    - Implement `isTransientError()` classification
    - Add exponential backoff delays
    - _Requirements: 8.1, 8.2, 8.3_

  - [ ] 7.7 Write property test for transient error retry
    - **Property 6: Transient Error Retry**
    - **Validates: Requirements 8.1**

  - [ ] 7.8 Write property test for exponential backoff
    - **Property 7: Exponential Backoff Timing**
    - **Validates: Requirements 8.2**

  - [ ] 7.9 Write property test for non-transient error handling
    - **Property 8: Non-Transient Error No-Retry**
    - **Validates: Requirements 8.5**

  - [ ] 7.10 Update existing API methods to use retry logic
    - Update `get()` method to use `requestWithRetry()`
    - Update chunk upload to use retry
    - Ensure non-transient errors propagate immediately
    - _Requirements: 8.1, 8.4, 8.5_

- [ ] 8. Checkpoint - Phase 4 Verification
  - Test `/healthz` endpoint returns correct status
  - Test retry behavior with simulated network failures
  - Verify transaction rollback on failures

- [ ] 9. Phase 5: Cleanup & Documentation
  - Final polish and documentation updates
  - _Requirements: 9.1, 9.2, 9.3_

  - [ ] 9.1 Update README.md
    - Remove all gRPC references
    - Update architecture diagram
    - Add refactoring changelog section
    - _Requirements: 9.1, 9.2, 9.3_

  - [ ] 9.2 Remove orphaned files
    - Delete any unused Swift files from Services/Sync
    - Delete any orphaned test files
    - Clean up any temporary files
    - _Requirements: 9.1_

  - [ ] 9.3 Write final integration test suite
    - End-to-end upload test
    - End-to-end download test
    - Health check verification
    - _Requirements: 1.4, 4.3, 7.1_

- [ ] 10. Final Checkpoint - Complete Verification
  - Run full test suite: `cargo test` and Xcode tests
  - Verify Docker deployment works
  - Manual smoke test of all functionality

## Notes

- Tasks marked with `*` are optional and can be skipped for faster MVP
- Each phase should be done in a separate Git branch for easy rollback
- Checkpoints ensure incremental validation before proceeding
- Property tests validate universal correctness properties
- Unit tests validate specific examples and edge cases
