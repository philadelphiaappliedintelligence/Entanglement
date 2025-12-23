# Requirements Document

## Introduction

This document specifies the requirements for a targeted refactoring of the Entanglement codebase to reduce complexity, eliminate dead code, consolidate duplicate implementations, and harden the production deployment. The refactoring addresses accumulated technical debt from rapid iteration while preserving all existing functionality.

## Glossary

- **Server**: The Rust-based tangled server that provides REST API, WebSocket, and storage services
- **BlobStore**: Legacy standalone blob storage system using 2-character directory sharding
- **BlobManager**: Container-based chunk storage system for chunked file storage
- **gRPC**: Google Remote Procedure Call protocol (currently unused but running)
- **REST_API**: The HTTP-based API used by all clients
- **macOS_Client**: The native macOS application including the main app and FileProvider extension
- **SyncCore**: Shared synchronization logic module in EntanglementCore framework
- **FileProvider_Extension**: macOS system extension for Finder integration
- **Container**: Append-only packfile storing multiple chunks for efficient disk I/O
- **Chunk**: A content-defined piece of a file identified by BLAKE3 hash
- **Transaction**: Database operation that ensures atomicity of multi-step changes

## Requirements

### Requirement 1: Remove Unused gRPC Infrastructure

**User Story:** As a maintainer, I want to remove the unused gRPC server code, so that I can reduce binary size, maintenance burden, and attack surface.

#### Acceptance Criteria

1. WHEN the Server starts, THE Server SHALL NOT spawn a gRPC listener
2. WHEN the Server is built, THE Server SHALL NOT include tonic, prost, or tonic-build dependencies
3. WHEN the Server is built, THE Server SHALL NOT include the sha2 dependency (BLAKE3 is used instead)
4. THE Server SHALL continue to serve REST_API requests after gRPC removal
5. THE Server SHALL continue to serve WebSocket connections after gRPC removal
6. WHEN running `tangled status`, THE Server SHALL NOT display gRPC port information

### Requirement 2: Consolidate File Upsert Functions

**User Story:** As a maintainer, I want to consolidate the multiple file upsert function variants, so that the codebase is easier to understand and maintain.

#### Acceptance Criteria

1. THE Server SHALL provide a single `upsert_file` function with optional parameters for all user-scoped operations
2. THE Server SHALL provide a single `upsert_file_global` function for admin/system operations
3. WHEN a file is upserted, THE Server SHALL use the consolidated function
4. THE Server SHALL maintain all existing file upsert functionality after consolidation

### Requirement 3: Remove Duplicate Docker Compose Configuration

**User Story:** As a developer, I want a single Docker Compose configuration, so that deployment is unambiguous and consistent.

#### Acceptance Criteria

1. THE project SHALL have exactly one docker-compose.yml file at the repository root
2. WHEN Docker Compose is run, THE configuration SHALL NOT expose gRPC ports
3. THE Docker configuration SHALL continue to expose REST and Web ports

### Requirement 4: Consolidate Storage Layer

**User Story:** As a maintainer, I want to eliminate the dual storage systems, so that storage operations are consistent and the codebase is simpler.

#### Acceptance Criteria

1. THE Server SHALL use BlobManager as the sole storage interface for all operations
2. WHEN the `index` command is run, THE Server SHALL use BlobManager for blob storage
3. WHEN the `export` command is run, THE Server SHALL use BlobManager for blob retrieval
4. THE BlobManager SHALL provide backward compatibility for reading legacy standalone blobs
5. WHEN a legacy blob is requested, THE BlobManager SHALL read from the legacy path structure
6. IF a blob exists in both legacy and container storage, THEN THE BlobManager SHALL prefer container storage

### Requirement 5: Consolidate Client Sync Logic

**User Story:** As a maintainer, I want to eliminate duplicate sync code between the macOS app and FileProvider extension, so that bug fixes and improvements apply to both.

#### Acceptance Criteria

1. THE SyncCore module SHALL provide shared chunking logic used by both app and extension
2. THE SyncCore module SHALL provide shared BLAKE3 hashing logic
3. THE SyncCore module SHALL provide shared chunk upload logic
4. THE SyncCore module SHALL provide shared chunk manifest handling
5. WHEN the macOS_Client chunks a file, THE macOS_Client SHALL use SyncCore.ChunkingCore
6. WHEN the FileProvider_Extension chunks a file, THE FileProvider_Extension SHALL use SyncCore.ChunkingCore
7. THE macOS_Client SHALL maintain all existing sync functionality after consolidation

### Requirement 6: Add Transaction Safety

**User Story:** As a user, I want file operations to be atomic, so that partial failures don't leave the system in an inconsistent state.

#### Acceptance Criteria

1. WHEN a file upload involves multiple database operations, THE Server SHALL wrap them in a transaction
2. WHEN a version is created with file update, THE Server SHALL use a single transaction
3. IF a transaction fails, THEN THE Server SHALL rollback all changes in that transaction
4. WHEN a transaction succeeds, THE Server SHALL commit all changes atomically

### Requirement 7: Add Health Check Endpoint

**User Story:** As an operator, I want a health check endpoint, so that container orchestration can monitor service health.

#### Acceptance Criteria

1. THE Server SHALL expose a `/healthz` endpoint
2. WHEN the database is accessible, THE `/healthz` endpoint SHALL return HTTP 200
3. WHEN the storage is accessible, THE `/healthz` endpoint SHALL return HTTP 200
4. IF the database is inaccessible, THEN THE `/healthz` endpoint SHALL return HTTP 503
5. THE `/healthz` response SHALL include status information in JSON format

### Requirement 8: Add Client Retry Logic

**User Story:** As a user, I want the client to automatically retry failed requests, so that transient network issues don't interrupt my workflow.

#### Acceptance Criteria

1. WHEN a transient network error occurs, THE macOS_Client SHALL retry the request
2. THE macOS_Client SHALL use exponential backoff between retry attempts
3. THE macOS_Client SHALL retry up to 3 times by default
4. IF all retries fail, THEN THE macOS_Client SHALL report the error to the user
5. THE macOS_Client SHALL NOT retry non-transient errors (e.g., 401, 403, 404)

### Requirement 9: Update Documentation

**User Story:** As a developer, I want accurate documentation, so that I can understand and work with the codebase effectively.

#### Acceptance Criteria

1. THE README SHALL NOT reference gRPC functionality
2. THE README SHALL include an updated architecture diagram
3. THE README SHALL document the refactoring changes
