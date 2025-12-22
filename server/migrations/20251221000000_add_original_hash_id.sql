-- Add column to store the original virtual hash ID of a folder
-- This allows us to maintain ID stability when a virtual folder is materialized into a real record

-- PostgreSQL doesn't have ADD COLUMN IF NOT EXISTS, so we use a DO block
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns 
        WHERE table_name = 'files' AND column_name = 'original_hash_id'
    ) THEN
        ALTER TABLE files ADD COLUMN original_hash_id TEXT;
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS idx_files_original_hash_id ON files(original_hash_id);
