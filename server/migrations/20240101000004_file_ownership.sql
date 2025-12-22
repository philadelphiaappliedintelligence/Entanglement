-- Add file ownership for security
-- Each file must have an owner for authorization checks

-- Add owner_id column to files table
ALTER TABLE files ADD COLUMN IF NOT EXISTS owner_id UUID REFERENCES users(id);

-- Create index for efficient ownership queries
CREATE INDEX IF NOT EXISTS idx_files_owner_id ON files(owner_id);

-- For existing files without an owner, they remain accessible only to admins
-- New files will require an owner_id to be set

-- Add a comment explaining the security model
COMMENT ON COLUMN files.owner_id IS 'User who owns this file. NULL for legacy/system files.';
