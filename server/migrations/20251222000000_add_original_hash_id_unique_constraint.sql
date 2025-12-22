-- Add unique constraint on original_hash_id for defense-in-depth
-- This prevents theoretical hash collision attacks on Sticky IDs, though
-- BLAKE3 collisions are cryptographically infeasible

-- Create a partial unique constraint (only on non-null values)
-- This allows multiple NULLs (most files don't have original_hash_id)
CREATE UNIQUE INDEX IF NOT EXISTS idx_files_original_hash_id_unique 
    ON files(original_hash_id) 
    WHERE original_hash_id IS NOT NULL;
