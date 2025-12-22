-- Content Defined Chunking (CDC) support for delta sync
-- 
-- Instead of storing files as single blobs, we split them into chunks.
-- This enables:
-- 1. Deduplication across files (shared chunks)
-- 2. Delta sync (only transfer changed chunks)
-- 3. Efficient storage for similar files

-- Chunks table - stores individual content chunks
-- Each chunk is identified by its SHA-256 hash
CREATE TABLE IF NOT EXISTS chunks (
    hash TEXT PRIMARY KEY,              -- SHA-256 hash of chunk content (hex encoded)
    size_bytes INTEGER NOT NULL,        -- Size of this chunk
    ref_count INTEGER NOT NULL DEFAULT 1,  -- Number of versions referencing this chunk
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Version chunks - maps versions to their ordered chunks
-- This is the "chunk manifest" for each file version
CREATE TABLE IF NOT EXISTS version_chunks (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    version_id UUID NOT NULL REFERENCES versions(id) ON DELETE CASCADE,
    chunk_hash TEXT NOT NULL REFERENCES chunks(hash),
    chunk_index INTEGER NOT NULL,       -- Position of chunk in file (0-indexed)
    chunk_offset BIGINT NOT NULL,       -- Byte offset in the file
    UNIQUE(version_id, chunk_index)
);

-- Add index for efficient chunk lookups
CREATE INDEX IF NOT EXISTS idx_version_chunks_version ON version_chunks(version_id, chunk_index);
CREATE INDEX IF NOT EXISTS idx_version_chunks_hash ON version_chunks(chunk_hash);
CREATE INDEX IF NOT EXISTS idx_chunks_ref_count ON chunks(ref_count);

-- Add is_chunked flag to versions to indicate if a version uses chunked storage
-- Default FALSE for backwards compatibility with existing raw blob uploads
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns 
        WHERE table_name = 'versions' AND column_name = 'is_chunked'
    ) THEN
        ALTER TABLE versions ADD COLUMN is_chunked BOOLEAN NOT NULL DEFAULT FALSE;
    END IF;
END $$;

