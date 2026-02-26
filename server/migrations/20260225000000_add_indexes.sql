-- Add missing indexes for query performance
CREATE INDEX IF NOT EXISTS idx_files_owner_id ON files(owner_id);
CREATE INDEX IF NOT EXISTS idx_versions_created_by ON versions(created_by);
