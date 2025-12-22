-- Migration: Fix corrupted folder paths and restore children
-- This fixes data corruption caused by a bug in move_path

-- 1. Find files that should be directories (have children but no trailing slash)
-- and add the trailing slash
UPDATE files f
SET path = f.path || '/'
WHERE 
    NOT f.path LIKE '%/'
    AND f.is_deleted = FALSE
    AND EXISTS (
        SELECT 1 FROM files child 
        WHERE child.path LIKE f.path || '/%' 
        AND child.is_deleted = FALSE
    );

-- 2. For the specific ppool issue: if /ppool exists without trailing slash
-- and there are orphaned files in root that should be in ppool
-- First, check if we need to restore the ppool folder
UPDATE files
SET path = '/ppool/'
WHERE path = '/ppool' AND is_deleted = FALSE;

-- 3. Show current state for manual review (run this SELECT separately to see results)
-- SELECT id, path, is_deleted FROM files WHERE path LIKE '/ppool%' OR path LIKE '/%' ORDER BY path;


