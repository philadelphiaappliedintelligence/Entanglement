-- Migration 003: BLAKE3, Dynamic Tiering, and Blob Container Support
-- 
-- Architectural Model: Version-Centric (files -> versions -> chunks)
-- This migration extends the existing schema without breaking compatibility.
--
-- Changes:
-- 1. Create blob_containers table (container-based chunk storage)
-- 2. Alter chunks table (add physical location columns)
-- 3. Alter versions table (add tier_id and blake3_hash)
-- 4. Add performance indexes

-- =============================================================================
-- STEP 1: Create blob_containers table
-- =============================================================================
-- Blob containers are append-only pack files that store multiple chunks.
-- This enables efficient disk I/O by batching small chunks together.

CREATE TABLE IF NOT EXISTS blob_containers (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    disk_path TEXT NOT NULL UNIQUE,          -- e.g., "2024/05/pack_abc.blob"
    total_size BIGINT NOT NULL DEFAULT 0,    -- Current size in bytes
    chunk_count INTEGER NOT NULL DEFAULT 0,  -- Number of chunks in container
    is_sealed BOOLEAN NOT NULL DEFAULT FALSE,-- Sealed = read-only, no more appends
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    sealed_at TIMESTAMPTZ                    -- When the container was sealed
);

-- Index for finding open containers to append to
CREATE INDEX IF NOT EXISTS idx_blob_containers_open 
    ON blob_containers(is_sealed, total_size) 
    WHERE is_sealed = FALSE;

-- =============================================================================
-- STEP 2: Alter chunks table - Add physical location columns
-- =============================================================================
-- Chunks can now be stored inside blob_containers for efficiency.
-- NULL values indicate legacy chunks stored as standalone blob files.

-- Add container_id reference
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns 
        WHERE table_name = 'chunks' AND column_name = 'container_id'
    ) THEN
        ALTER TABLE chunks 
            ADD COLUMN container_id UUID REFERENCES blob_containers(id) ON DELETE RESTRICT;
    END IF;
END $$;

-- Add offset within container
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns 
        WHERE table_name = 'chunks' AND column_name = 'offset_bytes'
    ) THEN
        ALTER TABLE chunks ADD COLUMN offset_bytes BIGINT;
    END IF;
END $$;

-- Add length (explicit, for validation against size_bytes)
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns 
        WHERE table_name = 'chunks' AND column_name = 'length_bytes'
    ) THEN
        ALTER TABLE chunks ADD COLUMN length_bytes INTEGER;
    END IF;
END $$;

-- Index for container-based chunk lookups
CREATE INDEX IF NOT EXISTS idx_chunks_container 
    ON chunks(container_id, offset_bytes) 
    WHERE container_id IS NOT NULL;

-- =============================================================================
-- STEP 3: Alter versions table - Add tier_id and blake3_hash
-- =============================================================================
-- The version is the "real file node" that holds content metadata.

-- Add tier_id column (0=Inline, 1=Granular, 2=Standard, 3=Large, 4=Jumbo)
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns 
        WHERE table_name = 'versions' AND column_name = 'tier_id'
    ) THEN
        ALTER TABLE versions ADD COLUMN tier_id SMALLINT NOT NULL DEFAULT 2;
        COMMENT ON COLUMN versions.tier_id IS 
            'Dynamic chunking tier: 0=Inline(<4KB), 1=Granular(<10MB/code), 2=Standard, 3=Large(>500MB), 4=Jumbo(>5GB/disk)';
    END IF;
END $$;

-- Add blake3_hash column (64-char hex string)
-- Note: blob_hash already exists but may contain SHA-256. We add blake3_hash explicitly.
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns 
        WHERE table_name = 'versions' AND column_name = 'blake3_hash'
    ) THEN
        ALTER TABLE versions ADD COLUMN blake3_hash CHAR(64);
        COMMENT ON COLUMN versions.blake3_hash IS 
            'BLAKE3 hash of the complete file content (64-char lowercase hex)';
    END IF;
END $$;

-- Index for BLAKE3 hash lookups (deduplication)
CREATE INDEX IF NOT EXISTS idx_versions_blake3_hash 
    ON versions(blake3_hash) 
    WHERE blake3_hash IS NOT NULL;

-- =============================================================================
-- STEP 4: Ensure files.path is properly indexed
-- =============================================================================
-- Already exists from migration 001, but ensure uniqueness

-- Path should already be UNIQUE, but add explicit index if needed
CREATE INDEX IF NOT EXISTS idx_files_path_lookup ON files(path);

-- =============================================================================
-- STEP 5: Add tier lookup helper
-- =============================================================================

-- Create an enum-like reference table for tier metadata (optional but useful)
CREATE TABLE IF NOT EXISTS tier_config (
    tier_id SMALLINT PRIMARY KEY,
    name TEXT NOT NULL,
    min_chunk_bytes INTEGER NOT NULL,
    avg_chunk_bytes INTEGER NOT NULL,
    max_chunk_bytes INTEGER NOT NULL
);

-- Insert tier configurations (idempotent)
INSERT INTO tier_config (tier_id, name, min_chunk_bytes, avg_chunk_bytes, max_chunk_bytes)
VALUES 
    (0, 'Inline',   0,        0,        0),          -- No chunking
    (1, 'Granular', 2048,     4096,     8192),       -- 2KB / 4KB / 8KB
    (2, 'Standard', 16384,    32768,    65536),      -- 16KB / 32KB / 64KB
    (3, 'Large',    524288,   1048576,  2097152),    -- 512KB / 1MB / 2MB
    (4, 'Jumbo',    4194304,  8388608,  16777216)    -- 4MB / 8MB / 16MB
ON CONFLICT (tier_id) DO NOTHING;

-- =============================================================================
-- STEP 6: Backfill blake3_hash from blob_hash where possible
-- =============================================================================
-- For existing versions, copy blob_hash to blake3_hash if it looks like a valid hash
-- (This assumes blob_hash contains the content hash, which may be SHA-256 or BLAKE3)

UPDATE versions
SET blake3_hash = blob_hash
WHERE blake3_hash IS NULL 
  AND blob_hash IS NOT NULL 
  AND LENGTH(blob_hash) = 64
  AND blob_hash ~ '^[a-f0-9]{64}$';

-- =============================================================================
-- COMMENTS
-- =============================================================================

COMMENT ON TABLE blob_containers IS 'Append-only pack files for efficient chunk storage';
COMMENT ON COLUMN chunks.container_id IS 'FK to blob_container (NULL = standalone blob file)';
COMMENT ON COLUMN chunks.offset_bytes IS 'Byte offset within the container file';
COMMENT ON COLUMN chunks.length_bytes IS 'Length of chunk data in container (should match size_bytes)';
COMMENT ON TABLE tier_config IS 'Reference table for FastCDC tier parameters';
