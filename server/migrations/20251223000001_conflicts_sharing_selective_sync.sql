-- Conflicts, Sharing, and Selective Sync Tables
-- Migration adds support for:
-- 1. Conflict Detection and Resolution
-- 2. File/Folder Sharing with links
-- 3. Selective Sync preferences

-- =============================================================================
-- CONFLICTS
-- =============================================================================

-- Conflicts table - tracks file sync conflicts
CREATE TABLE IF NOT EXISTS sync_conflicts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    file_id UUID NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    
    -- Conflict details
    local_version_id UUID REFERENCES versions(id) ON DELETE SET NULL,
    remote_version_id UUID REFERENCES versions(id) ON DELETE SET NULL,
    conflict_type VARCHAR(32) NOT NULL, -- 'edit_edit', 'delete_edit', 'edit_delete'
    
    -- Resolution
    resolved_at TIMESTAMPTZ,
    resolution VARCHAR(32), -- 'keep_local', 'keep_remote', 'keep_both', 'manual'
    resolved_by UUID REFERENCES users(id) ON DELETE SET NULL,
    
    -- Timestamps
    detected_at TIMESTAMPTZ DEFAULT NOW(),
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- Index for finding user's unresolved conflicts
CREATE INDEX IF NOT EXISTS idx_conflicts_user_unresolved 
    ON sync_conflicts(user_id, resolved_at) 
    WHERE resolved_at IS NULL;

-- Index for finding conflicts by file
CREATE INDEX IF NOT EXISTS idx_conflicts_file 
    ON sync_conflicts(file_id);

-- =============================================================================
-- SHARING
-- =============================================================================

-- Share links - public or protected links to files/folders
CREATE TABLE IF NOT EXISTS share_links (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    file_id UUID NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    created_by UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    
    -- Link configuration
    token VARCHAR(64) NOT NULL UNIQUE, -- Random token for URL (e.g., /share/abc123)
    password_hash VARCHAR(255), -- Optional password protection (bcrypt)
    
    -- Permissions
    can_view BOOLEAN DEFAULT TRUE,
    can_download BOOLEAN DEFAULT TRUE,
    can_edit BOOLEAN DEFAULT FALSE,
    
    -- Limits
    expires_at TIMESTAMPTZ, -- Optional expiration
    max_downloads INTEGER, -- Optional download limit
    download_count INTEGER DEFAULT 0,
    
    -- Status
    is_active BOOLEAN DEFAULT TRUE,
    
    -- Timestamps
    created_at TIMESTAMPTZ DEFAULT NOW(),
    last_accessed_at TIMESTAMPTZ
);

-- Index for token lookup (most common operation)
CREATE INDEX IF NOT EXISTS idx_share_links_token ON share_links(token);

-- Index for finding user's shares
CREATE INDEX IF NOT EXISTS idx_share_links_created_by ON share_links(created_by);

-- Index for finding shares by file
CREATE INDEX IF NOT EXISTS idx_share_links_file ON share_links(file_id);

-- Shared users - direct sharing with specific users
CREATE TABLE IF NOT EXISTS shared_users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    file_id UUID NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    owner_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    shared_with_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    
    -- Permissions
    can_view BOOLEAN DEFAULT TRUE,
    can_download BOOLEAN DEFAULT TRUE,
    can_edit BOOLEAN DEFAULT FALSE,
    can_delete BOOLEAN DEFAULT FALSE,
    can_share BOOLEAN DEFAULT FALSE,
    
    -- Timestamps
    created_at TIMESTAMPTZ DEFAULT NOW(),
    
    -- Prevent duplicate shares
    UNIQUE(file_id, shared_with_id)
);

-- Index for finding what's shared with a user
CREATE INDEX IF NOT EXISTS idx_shared_users_shared_with ON shared_users(shared_with_id);

-- Index for finding what a user has shared
CREATE INDEX IF NOT EXISTS idx_shared_users_owner ON shared_users(owner_id);

-- =============================================================================
-- SELECTIVE SYNC
-- =============================================================================

-- Selective sync rules - per-user sync preferences
CREATE TABLE IF NOT EXISTS selective_sync_rules (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    
    -- Rule type: 'include' or 'exclude'
    rule_type VARCHAR(16) NOT NULL,
    
    -- Path pattern (supports wildcards)
    -- Examples: "/work/", "/photos/*.raw", "*.tmp"
    path_pattern VARCHAR(1024) NOT NULL,
    
    -- Priority (higher = evaluated first)
    priority INTEGER DEFAULT 0,
    
    -- Status
    is_active BOOLEAN DEFAULT TRUE,
    
    -- Timestamps
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

-- Index for finding user's sync rules
CREATE INDEX IF NOT EXISTS idx_selective_sync_user ON selective_sync_rules(user_id, is_active);

-- Device sync state - tracks what's synced per device
CREATE TABLE IF NOT EXISTS device_sync_state (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    device_id VARCHAR(255) NOT NULL, -- Client-generated device identifier
    device_name VARCHAR(255), -- Human-readable device name
    
    -- Sync cursor for delta sync
    last_sync_cursor TIMESTAMPTZ,
    
    -- Storage quota tracking
    synced_bytes BIGINT DEFAULT 0,
    max_sync_bytes BIGINT, -- NULL = unlimited
    
    -- Status
    is_active BOOLEAN DEFAULT TRUE,
    last_seen_at TIMESTAMPTZ DEFAULT NOW(),
    
    -- Timestamps
    created_at TIMESTAMPTZ DEFAULT NOW(),
    
    -- One device per user
    UNIQUE(user_id, device_id)
);

-- Index for device lookup
CREATE INDEX IF NOT EXISTS idx_device_sync_user ON device_sync_state(user_id, is_active);

