use super::DbPool;
use chrono::{DateTime, Utc};
use uuid::Uuid;
use serde::Serialize;

#[allow(dead_code)]
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct File {
    pub id: Uuid,
    pub path: String,
    pub current_version_id: Option<Uuid>,
    pub is_deleted: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub owner_id: Option<Uuid>,
    pub original_hash_id: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct FileWithVersion {
    pub id: Uuid,
    pub path: String,
    pub current_version_id: Option<Uuid>,
    pub is_deleted: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub size_bytes: Option<i64>,
    pub blob_hash: Option<String>,
    pub original_hash_id: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct FileChange {
    pub id: Uuid,
    pub path: String,
    pub current_version_id: Option<Uuid>,
    pub is_deleted: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub size_bytes: Option<i64>,
    pub blob_hash: Option<String>,
    pub original_hash_id: Option<String>,
}

/// Create or update a file record (upsert) - global (no owner)
pub async fn upsert_file_global(pool: &DbPool, path: &str) -> anyhow::Result<File> {
    let file = sqlx::query_as::<_, File>(
        r#"
        INSERT INTO files (path)
        VALUES ($1)
        ON CONFLICT (path)
        DO UPDATE SET updated_at = NOW(), is_deleted = FALSE
        RETURNING id, path, current_version_id, is_deleted, created_at, updated_at, owner_id, original_hash_id
        "#,
    )
    .bind(path)
    .fetch_one(pool)
    .await?;

    Ok(file)
}

/// Create or update a file record with client-provided dates
/// Uses the provided dates if available, otherwise preserves existing dates, or falls back to NOW()
pub async fn upsert_file_with_dates(
    pool: &DbPool, 
    path: &str,
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
) -> anyhow::Result<File> {
    let file = sqlx::query_as::<_, File>(
        r#"
        INSERT INTO files (path, created_at, updated_at)
        VALUES ($1, COALESCE($2, NOW()), COALESCE($3, NOW()))
        ON CONFLICT (path)
        DO UPDATE SET 
            updated_at = COALESCE($3, files.updated_at),
            is_deleted = FALSE
        RETURNING id, path, current_version_id, is_deleted, created_at, updated_at, owner_id, original_hash_id
        "#,
    )
    .bind(path)
    .bind(created_at)
    .bind(updated_at)
    .fetch_one(pool)
    .await?;

    Ok(file)
}

/// Create or update a file record with owner and client-provided dates (secure version)
pub async fn upsert_file_with_owner_and_dates(
    pool: &DbPool, 
    path: &str,
    owner_id: Uuid,
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
) -> anyhow::Result<File> {
    let file = sqlx::query_as::<_, File>(
        r#"
        INSERT INTO files (path, owner_id, created_at, updated_at)
        VALUES ($1, $2, COALESCE($3, NOW()), COALESCE($4, NOW()))
        ON CONFLICT (path)
        DO UPDATE SET 
            updated_at = COALESCE($4, files.updated_at),
            is_deleted = FALSE
        WHERE files.owner_id = $2 OR files.owner_id IS NULL
        RETURNING id, path, current_version_id, is_deleted, created_at, updated_at, owner_id, original_hash_id
        "#,
    )
    .bind(path)
    .bind(owner_id)
    .bind(created_at)
    .bind(updated_at)
    .fetch_one(pool)
    .await?;

    Ok(file)
}

/// Create or update a file record with owner
pub async fn upsert_file_with_owner(
    pool: &DbPool,
    path: &str,
    owner_id: Uuid,
) -> anyhow::Result<File> {
    let file = sqlx::query_as::<_, File>(
        r#"
        INSERT INTO files (path, owner_id)
        VALUES ($1, $2)
        ON CONFLICT (path)
        DO UPDATE SET updated_at = NOW(), is_deleted = FALSE
        WHERE files.owner_id = $2 OR files.owner_id IS NULL
        RETURNING id, path, current_version_id, is_deleted, created_at, updated_at, owner_id, original_hash_id
        "#,
    )
    .bind(path)
    .bind(owner_id)
    .fetch_one(pool)
    .await?;

    Ok(file)
}

/// Create or update a file record with owner and optional original hash ID
/// Used when materializing virtual folders to preserve ID continuity
pub async fn upsert_file_with_owner_and_hash(
    pool: &DbPool,
    path: &str,
    owner_id: Uuid,
    original_hash_id: Option<String>,
) -> anyhow::Result<File> {
    let file = sqlx::query_as::<_, File>(
        r#"
        INSERT INTO files (path, owner_id, original_hash_id)
        VALUES ($1, $2, $3)
        ON CONFLICT (path)
        DO UPDATE SET
            updated_at = NOW(),
            is_deleted = FALSE,
            original_hash_id = COALESCE($3, files.original_hash_id)
        WHERE files.owner_id = $2 OR files.owner_id IS NULL
        RETURNING id, path, current_version_id, is_deleted, created_at, updated_at, owner_id, original_hash_id
        "#,
    )
    .bind(path)
    .bind(owner_id)
    .bind(&original_hash_id)
    .fetch_one(pool)
    .await?;

    Ok(file)
}

/// Legacy: Create or update a file record with user (for API compatibility)
#[allow(dead_code)]
pub async fn upsert_file(pool: &DbPool, _user_id: Uuid, path: &str) -> anyhow::Result<File> {
    upsert_file_global(pool, path).await
}

/// Get a file by ID with ownership check
/// Returns the file only if the user owns it or if the file has no owner (legacy)
pub async fn get_file_by_id(
    pool: &DbPool,
    file_id: Uuid,
    user_id: Uuid,
) -> anyhow::Result<Option<File>> {
    let file = sqlx::query_as::<_, File>(
        r#"
        SELECT id, path, current_version_id, is_deleted, created_at, updated_at, owner_id, original_hash_id
        FROM files
        WHERE id = $1 AND (owner_id = $2 OR owner_id IS NULL)
        "#,
    )
    .bind(file_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    Ok(file)
}

/// Get a file by ID with version info (global - for admin operations only)
/// WARNING: This bypasses ownership checks. Use get_file_by_id_with_owner for user-facing operations.
pub async fn get_file_by_id_global(
    pool: &DbPool,
    file_id: Uuid,
) -> anyhow::Result<Option<FileWithVersion>> {
    let file = sqlx::query_as::<_, FileWithVersion>(
        r#"
        SELECT f.id, f.path, f.current_version_id, f.is_deleted,
               f.created_at, f.updated_at, v.size_bytes, v.blob_hash,
               f.original_hash_id
        FROM files f
        LEFT JOIN versions v ON f.current_version_id = v.id
        WHERE f.id = $1
        "#,
    )
    .bind(file_id)
    .fetch_optional(pool)
    .await?;

    Ok(file)
}

/// Get a file by its original hash ID (Sticky ID lookup)
/// This is used to resolve materialized folders after they have been moved
pub async fn get_file_by_original_hash(
    pool: &DbPool,
    original_hash: &str,
) -> anyhow::Result<Option<File>> {
    let file = sqlx::query_as::<_, File>(
        r#"
        SELECT id, path, current_version_id, is_deleted, created_at, updated_at, owner_id, original_hash_id
        FROM files
        WHERE original_hash_id = $1 AND is_deleted = FALSE
        "#,
    )
    .bind(original_hash)
    .fetch_optional(pool)
    .await?;

    Ok(file)
}

/// Get a file by ID with version info and ownership check
pub async fn get_file_by_id_with_owner(
    pool: &DbPool,
    file_id: Uuid,
    user_id: Uuid,
) -> anyhow::Result<Option<FileWithVersion>> {
    let file = sqlx::query_as::<_, FileWithVersion>(
        r#"
        SELECT f.id, f.path, f.current_version_id, f.is_deleted,
               f.created_at, f.updated_at, v.size_bytes, v.blob_hash,
               f.original_hash_id
        FROM files f
        LEFT JOIN versions v ON f.current_version_id = v.id
        WHERE f.id = $1 AND (f.owner_id = $2 OR f.owner_id IS NULL)
        "#,
    )
    .bind(file_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    Ok(file)
}

/// Get a file by path with ownership check
pub async fn get_file_by_path(
    pool: &DbPool,
    user_id: Uuid,
    path: &str,
) -> anyhow::Result<Option<File>> {
    let file = sqlx::query_as::<_, File>(
        r#"
        SELECT id, path, current_version_id, is_deleted, created_at, updated_at, owner_id, original_hash_id
        FROM files
        WHERE path = $1 AND (owner_id = $2 OR owner_id IS NULL)
        "#,
    )
    .bind(path)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    Ok(file)
}

/// Set the current version of a file
/// NOTE: Does NOT update `updated_at` to preserve the original file modification date
pub async fn set_current_version(
    pool: &DbPool,
    file_id: Uuid,
    version_id: Uuid,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        UPDATE files
        SET current_version_id = $2
        WHERE id = $1
        "#,
    )
    .bind(file_id)
    .bind(version_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Soft delete a file
pub async fn soft_delete(pool: &DbPool, file_id: Uuid) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        UPDATE files
        SET is_deleted = TRUE, updated_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(file_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Soft delete a file and all children (recursive)
/// Used for directory deletion
pub async fn soft_delete_recursive(pool: &DbPool, file_id: Uuid) -> anyhow::Result<()> {
    // 1. Get the file path
    let file = get_file_by_id_global(pool, file_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("File not found"))?;

    // 2. If it's a directory (path ends in /), delete all children AND the directory itself
    if file.path.ends_with('/') {
        let prefix_pattern = format!("{}%", file.path);
        
        // Delete children matching the prefix AND the directory record itself
        sqlx::query(
            r#"
            UPDATE files
            SET is_deleted = TRUE, updated_at = NOW()
            WHERE path LIKE $1 OR id = $2
            "#,
        )
        .bind(prefix_pattern)
        .bind(file_id)
        .execute(pool)
        .await?;
    } else {
        // Just delete the single file
        soft_delete(pool, file_id).await?;
    }

    Ok(())
}

/// Soft delete a file with ownership check
pub async fn soft_delete_with_owner(pool: &DbPool, file_id: Uuid, user_id: Uuid) -> anyhow::Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE files
        SET is_deleted = TRUE, updated_at = NOW()
        WHERE id = $1 AND (owner_id = $2 OR owner_id IS NULL)
        "#,
    )
    .bind(file_id)
    .bind(user_id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Soft delete a file and all children (recursive) with ownership check
pub async fn soft_delete_recursive_with_owner(pool: &DbPool, file_id: Uuid, user_id: Uuid) -> anyhow::Result<bool> {
    // 1. Get the file with ownership check
    let file = get_file_by_id_with_owner(pool, file_id, user_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("File not found or access denied"))?;

    // 2. If it's a directory (path ends in /), delete all children AND the directory itself
    if file.path.ends_with('/') {
        let prefix_pattern = format!("{}%", file.path);
        
        // Delete children matching the prefix AND the directory record itself (with ownership check)
        let result = sqlx::query(
            r#"
            UPDATE files
            SET is_deleted = TRUE, updated_at = NOW()
            WHERE (path LIKE $1 OR id = $2) AND (owner_id = $3 OR owner_id IS NULL)
            "#
        )
        .bind(prefix_pattern)
        .bind(file_id)
        .bind(user_id)
        .execute(pool)
        .await?;
        
        Ok(result.rows_affected() > 0)
    } else {
        // Just delete the single file with ownership check
        soft_delete_with_owner(pool, file_id, user_id).await
    }
}

/// Move or rename a file (and its children if it's a directory)
pub async fn move_file(pool: &DbPool, file_id: Uuid, new_path: &str, user_id: Uuid) -> anyhow::Result<File> {
    tracing::info!("DEBUG: move_file entry. ID: {}, Target: {}", file_id, new_path);
    
    // 1. Get the original file to check permissions and get old path
    let file = get_file_by_id_with_owner(pool, file_id, user_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("File not found or access denied"))?;

    let old_path = file.path;
    tracing::info!("DEBUG: move_file resolving to move_path. Old: '{}', New: '{}'", old_path, new_path);

    // Delegate to path-based move logic
    move_path(pool, &old_path, new_path, user_id).await
}

/// Core move logic working on paths (handles both real and virtual folders)
pub async fn move_path(pool: &DbPool, old_path: &str, new_path: &str, user_id: Uuid) -> anyhow::Result<File> {
    tracing::info!("DEBUG: move_path start. '{}' -> '{}'", old_path, new_path);

    // Smart Root Handling:
    // If user tries to move to "/", they probably mean "Move INTO root", not "Rename TO root".
    // We should infer the new path from the old filename.
    let mut resolved_new_path = new_path.to_string();
    
    if new_path == "/" {
        // Extract basename from old_path
        let _path_obj = std::path::Path::new(old_path);
        
        // Handle trailing slash for directories (e.g. "/music/ppooll/")
        // Path::new("/music/ppooll/").file_name() is None or empty depending on impl, 
        // usually it ignores trailing slash if component based, but safe to trim.
        let trimmed = old_path.trim_end_matches('/');
        let name = std::path::Path::new(trimmed)
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default();

        if !name.is_empty() {
            // Reconstruct path at root
            if old_path.ends_with('/') {
                resolved_new_path = format!("/{}/", name);
            } else {
                resolved_new_path = format!("/{}", name);
            }
            tracing::info!("DEBUG: inferred move to root -> '{}'", resolved_new_path);
        } else {
             // Fallback if we can't parse name (shouldn't happen for valid paths)
             return Err(anyhow::anyhow!("Cannot move root to root"));
        }
    } else if new_path == "" {
        // Handle empty path as root too just in case
        return Err(anyhow::anyhow!("Invalid empty target path"));
    }

    // Use the resolved path for existence check
    let new_path_str = resolved_new_path.as_str();

    // 1. Check if target already exists
    // CRITICAL: We need to normalize new_path check depending on whether it's a dir or file move? 
    // Actually, SQL exact match is fine initially.
    let target_exists = sqlx::query(
        "SELECT 1 FROM files WHERE path = $1 AND is_deleted = FALSE"
    )
    .bind(new_path_str)
    .fetch_optional(pool)
    .await?;

    if target_exists.is_some() {
        tracing::error!("DEBUG: Target path already exists: {}", new_path_str);
        return Err(anyhow::anyhow!("Target path already exists"));
    }

    // 2. Perform the move
    
    // Self-Healing Check: Does this path have children?
    // If so, it's a directory, even if missing trailing slash (Legacy Data Fix)
    // We check `old_path + /%` to ensure we don't match `foo_bar` for `foo`
    let check_path = if old_path.ends_with('/') {
        format!("{}%", old_path)
    } else {
        format!("{}/%", old_path)
    };

    let has_children = sqlx::query("SELECT 1 FROM files WHERE path LIKE $1 AND is_deleted = FALSE LIMIT 1")
        .bind(&check_path)
        .fetch_optional(pool)
        .await?
        .is_some();

    // Also check if the directory itself exists as a record
    let _dir_exists = sqlx::query("SELECT 1 FROM files WHERE path = $1 AND is_deleted = FALSE LIMIT 1")
        .bind(old_path)
        .fetch_optional(pool)
        .await?
        .is_some();

    if old_path.ends_with('/') || has_children {
        tracing::info!("DEBUG: Detected Directory Move (Slash: {}, Children: {})", old_path.ends_with('/'), has_children);
        // It's a directory: Move the directory itself AND all children
        
        // CRITICAL: Enforce trailing slash on new_path if we are moving a directory
        // This prevents accidental conversion to a file if client omits the slash
        let clean_new_path = if new_path_str.ends_with('/') {
            new_path_str.to_string()
        } else {
            format!("{}/", new_path_str)
        };
        tracing::info!("DEBUG: Directory move - clean_new_path enforced: {}", clean_new_path);

        // Transaction since we are updating multiple rows potentially
        let mut tx = pool.begin().await?;

        tracing::info!("DEBUG: Moving directory. Old: '{}', New: '{}', Prefix: '{}'", old_path, clean_new_path, check_path);

        // Normalize old_path - handle both with and without trailing slash
        let old_with_slash = if old_path.ends_with('/') {
            old_path.to_string()
        } else {
            format!("{}/", old_path)
        };
        let old_without_slash = old_with_slash.trim_end_matches('/').to_string();

        tracing::info!("DEBUG: old_with_slash='{}', old_without_slash='{}'", old_with_slash, old_without_slash);

        // Update the directory record itself - match EITHER with or without trailing slash
        // This handles legacy data inconsistencies
        let dir_result = sqlx::query(
            r#"
            UPDATE files
            SET path = $1, updated_at = NOW()
            WHERE (path = $2 OR path = $3) AND (owner_id = $4 OR owner_id IS NULL)
            "#
        )
        .bind(&clean_new_path)          // New path: "/MUSIC/ppooll/"
        .bind(&old_with_slash)          // Match: "/ppooll/"
        .bind(&old_without_slash)       // Also match: "/ppooll"
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        
        tracing::info!("DEBUG: Directory record update affected: {} rows", dir_result.rows_affected());

        // Update all children (paths that START WITH the old directory path)
        // Children are stored WITH the parent path prefix, e.g., "/ppooll/file.txt"
        let children_result = sqlx::query(
            r#"
            UPDATE files
            SET path = $1 || SUBSTRING(path, $2 + 1), updated_at = NOW()
            WHERE path LIKE $3 
              AND path != $4 
              AND path != $5
              AND (owner_id = $6 OR owner_id IS NULL)
            "#
        )
        .bind(&clean_new_path)                      // New parent: "/MUSIC/ppooll/"
        .bind(old_with_slash.len() as i32)          // Strip length (includes trailing slash)
        .bind(format!("{}%", &old_with_slash))      // Match: "/ppooll/%"
        .bind(&old_with_slash)                      // Exclude: "/ppooll/"
        .bind(&old_without_slash)                   // Exclude: "/ppooll"
        .bind(user_id)
        .execute(&mut *tx)
        .await?;

        tracing::info!("DEBUG: Children update affected: {} rows", children_result.rows_affected());

        // Check if the directory record itself exists and was updated
        let updated_file = sqlx::query_as::<_, File>(
            r#"
            SELECT id, path, current_version_id, is_deleted, created_at, updated_at, owner_id, original_hash_id
            FROM files
            WHERE path = $1
            "#
        )
        .bind(&clean_new_path)
        .fetch_optional(&mut *tx)
        .await?;
        
        tx.commit().await?;

        if let Some(f) = updated_file {
            tracing::info!("DEBUG: Directory record found at new path: '{}' with ID: {}", f.path, f.id);
            tracing::info!("DEBUG: Original hash ID: {:?}", f.original_hash_id);
            Ok(f)
        } else {
            tracing::warn!("DEBUG: No directory record found at new path '{}' - checking if we need to create it", clean_new_path);
            // Check if we need to update the directory record itself
            // Sometimes the UPDATE doesn't match the directory itself if it doesn't have children

            // Try to find and update the directory record directly (handle both path variants)
            let dir_result = sqlx::query_as::<_, File>(
                r#"
                UPDATE files
                SET path = $1, updated_at = NOW()
                WHERE (path = $2 OR path = $3) AND (owner_id = $4 OR owner_id IS NULL)
                RETURNING id, path, current_version_id, is_deleted, created_at, updated_at, owner_id, original_hash_id
                "#
            )
            .bind(&clean_new_path)
            .bind(&old_with_slash)
            .bind(&old_without_slash)
            .bind(user_id)
            .fetch_optional(pool)
            .await?;

            if let Some(f) = dir_result {
                tracing::info!("DEBUG: Directory record updated directly. ID: {}", f.id);
                Ok(f)
            } else {
                tracing::info!("DEBUG: Virtual folder move - upserting new record");
                // If we moved a virtual folder (no directory record), we should technically Create one now
                // so the client has a real object to reference for the new path.
            
            // CRITICAL: STICKY ID SUPPORT
            // Since this was a virtual folder, the client knows it by the Hash of its old path.
            // We must save this Hash in `original_hash_id` so the client can continue to access it by the old ID.
            
            // 1. Calculate the old hash (virtual ID)
            // old_path is like "/music/ppooll/" - check if it ends with / (it does in this block)
            let virtual_id = blake3::hash(old_path.as_bytes()).to_hex().to_string();
            
            upsert_file_with_owner_and_hash(pool, &clean_new_path, user_id, Some(virtual_id)).await
        }
    }

    } else {
        tracing::info!("DEBUG: Detected File Move");
        // Single file move
        let updated_file = sqlx::query_as::<_, File>(
            r#"
            UPDATE files
            SET path = $1, updated_at = NOW()
            WHERE path = $2 AND (owner_id = $3 OR owner_id IS NULL)
            RETURNING id, path, current_version_id, is_deleted, created_at, updated_at, owner_id, original_hash_id
            "#
        )
        .bind(new_path_str) // Use resolved new_path_str here
        .bind(old_path)
        .bind(user_id)
        .fetch_one(pool)
        .await?;
        
        tracing::info!("DEBUG: File move success. ID: {}", updated_file.id);

        Ok(updated_file)
    }
}

/// Undelete a file
pub async fn undelete(pool: &DbPool, file_id: Uuid) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        UPDATE files
        SET is_deleted = FALSE, updated_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(file_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// List all files for a user (with ownership check)
pub async fn list_files(
    pool: &DbPool,
    user_id: Uuid,
    prefix: Option<&str>,
    include_deleted: bool,
    limit: i64,
    offset: i64,
) -> anyhow::Result<(Vec<FileWithVersion>, i64)> {
    let prefix_pattern = prefix.map(|p| format!("{}%", p));

    let files = sqlx::query_as::<_, FileWithVersion>(
        r#"
        SELECT f.id, f.path, f.current_version_id, f.is_deleted,
               f.created_at, f.updated_at, v.size_bytes, v.blob_hash,
               f.original_hash_id
        FROM files f
        LEFT JOIN versions v ON f.current_version_id = v.id
        WHERE ($1::text IS NULL OR f.path LIKE $1)
          AND ($2 OR f.is_deleted = FALSE)
          AND (f.owner_id = $5 OR f.owner_id IS NULL)
        ORDER BY f.path
        LIMIT $3 OFFSET $4
        "#,
    )
    .bind(&prefix_pattern)
    .bind(include_deleted)
    .bind(limit)
    .bind(offset)
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let total: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*)
        FROM files
        WHERE ($1::text IS NULL OR path LIKE $1)
          AND ($2 OR is_deleted = FALSE)
          AND (owner_id = $3 OR owner_id IS NULL)
        "#,
    )
    .bind(&prefix_pattern)
    .bind(include_deleted)
    .bind(user_id)
    .fetch_one(pool)
    .await?;

    Ok((files, total.0))
}

/// Get a file by its version ID (looks up version -> file relationship)
pub async fn get_file_by_version_id(
    pool: &DbPool,
    version_id: Uuid,
) -> anyhow::Result<Option<File>> {
    let file = sqlx::query_as::<_, File>(
        r#"
        SELECT f.id, f.path, f.current_version_id, f.is_deleted, f.created_at, f.updated_at, f.owner_id, f.original_hash_id
        FROM files f
        JOIN versions v ON v.file_id = f.id
        WHERE v.id = $1
        "#,
    )
    .bind(version_id)
    .fetch_optional(pool)
    .await?;

    Ok(file)
}

/// Get file changes since a cursor (for delta sync) with ownership check
pub async fn get_changes(
    pool: &DbPool,
    user_id: Uuid,
    cursor: Option<DateTime<Utc>>,
    limit: i64,
) -> anyhow::Result<Vec<FileChange>> {
    let changes = sqlx::query_as::<_, FileChange>(
        r#"
        SELECT f.id, f.path, f.current_version_id, f.is_deleted, 
               f.created_at, f.updated_at, v.size_bytes, v.blob_hash
        FROM files f
        LEFT JOIN versions v ON f.current_version_id = v.id
        WHERE ($1::timestamptz IS NULL OR f.updated_at > $1)
          AND (f.owner_id = $3 OR f.owner_id IS NULL)
        ORDER BY f.updated_at ASC
        LIMIT $2
        "#,
    )
    .bind(cursor)
    .bind(limit)
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    Ok(changes)
}

// =============================================================================
// Directory Listing (Virtual Folders)
// =============================================================================

/// An entry in a directory listing (file or virtual folder)
#[derive(Debug, Clone)]
pub struct DirectoryEntry {
    /// File ID (real UUID for files, BLAKE3 hash for folders)
    pub id: String,
    /// Entry name (just the filename/folder name, not full path)
    pub name: String,
    /// Full path (for folders: "documents/work/")
    pub path: String,
    /// True if this is a virtual folder
    pub is_folder: bool,
    /// Size in bytes (0 for folders)
    pub size_bytes: i64,
    /// Last updated timestamp
    pub updated_at: DateTime<Utc>,
    /// Current version ID (None for folders)
    pub version_id: Option<Uuid>,
}

/// List directory contents with virtual folder support.
///
/// Given a prefix like "documents/", returns:
/// - Direct child files (e.g., "documents/notes.txt")
/// - Virtual folders for subdirectories (e.g., "documents/work/")
///
/// Folders are deduplicated and get deterministic IDs via BLAKE3 hash.
pub async fn list_directory(
    pool: &DbPool,
    prefix: &str,
) -> anyhow::Result<Vec<DirectoryEntry>> {
    use std::collections::{HashMap, HashSet};
    
    // Normalize prefix: empty string for root, otherwise ensure trailing slash
    let normalized_prefix = if prefix.is_empty() || prefix == "/" {
        String::new()
    } else {
        let p = prefix.trim_start_matches('/');
        if p.ends_with('/') {
            p.to_string()
        } else {
            format!("{}/", p)
        }
    };
    
    // Query all files under this prefix
    // Note: DB paths start with "/" so we match "/prefix%"
    let prefix_pattern = format!("/{}%", normalized_prefix);
    
    let files = sqlx::query_as::<_, FileWithVersion>(
        r#"
        SELECT f.id, f.path, f.current_version_id, f.is_deleted,
               f.created_at, f.updated_at, v.size_bytes, v.blob_hash,
               f.original_hash_id
        FROM files f
        LEFT JOIN versions v ON f.current_version_id = v.id
        WHERE f.path LIKE $1 AND f.is_deleted = FALSE
        ORDER BY f.path
        "#,
    )
    .bind(&prefix_pattern)
    .fetch_all(pool)
    .await?;
    
    let mut entries: Vec<DirectoryEntry> = Vec::new();
    let mut seen_folders: HashSet<String> = HashSet::new();
    let mut folder_times: HashMap<String, DateTime<Utc>> = HashMap::new();
    
    let prefix_len = normalized_prefix.len();
    
    for file in files {
        // Strip the prefix to get the relative path
        // Also handle paths that start with "/" (common in our DB)
        let file_path = file.path.trim_start_matches('/');
        
        // Skip if the file path doesn't start with our prefix (shouldn't happen, but be safe)
        if !file_path.starts_with(&normalized_prefix) {
            continue;
        }
        
        let relative_path = &file_path[prefix_len..];
        
        // Skip empty relative paths (the prefix itself)
        if relative_path.is_empty() {
            continue;
        }
        
        // Check if this is a direct child or nested
        if let Some(slash_pos) = relative_path.find('/') {
            // Nested path - extract folder name
            let folder_name = &relative_path[..slash_pos];
            
            if !seen_folders.contains(folder_name) {
                seen_folders.insert(folder_name.to_string());
                folder_times.insert(folder_name.to_string(), file.updated_at);
            } else {
                // Update folder time to most recent file
                if let Some(existing_time) = folder_times.get_mut(folder_name) {
                    if file.updated_at > *existing_time {
                        *existing_time = file.updated_at;
                    }
                }
            }
        } else {
            // Direct child file
            entries.push(DirectoryEntry {
                id: file.id.to_string(),
                name: relative_path.to_string(),
                path: file.path.clone(),
                is_folder: false,
                size_bytes: file.size_bytes.unwrap_or(0),
                updated_at: file.updated_at,
                version_id: file.current_version_id,
            });
        }
    }
    
    // Add virtual folder entries
    // First, look up any existing folder records to get their real UUIDs
    let folder_paths: Vec<String> = seen_folders
        .iter()
        .map(|name| format!("/{}{}/", normalized_prefix, name))
        .collect();
    
    // Query for existing folder records
    // Query for existing folder records
    // CRITICAL: Fetch original_hash_id to support Sticky IDs
    let existing_folders: std::collections::HashMap<String, String> = if !folder_paths.is_empty() {
        sqlx::query_as::<_, (uuid::Uuid, String, Option<String>)>(
            r#"
            SELECT id, path, original_hash_id FROM files 
            WHERE path = ANY($1) AND is_deleted = FALSE
            "#
        )
        .bind(&folder_paths)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|(id, path, original_hash)| {
            // Prefer Sticky ID if available, otherwise use UUID
            let effective_id = original_hash.unwrap_or(id.to_string());
            (path, effective_id)
        })
        .collect()
    } else {
        std::collections::HashMap::new()
    };
    
    for folder_name in seen_folders {
        let full_folder_path = format!("/{}{}/", normalized_prefix, folder_name);
        
        // Use real UUID/Sticky ID if folder exists in DB, otherwise generate BLAKE3 hash
        let folder_id = if let Some(effective_id) = existing_folders.get(&full_folder_path) {
            effective_id.clone()
        } else {
            // Fallback to deterministic hash for truly virtual folders
            blake3::hash(full_folder_path.as_bytes()).to_hex().to_string()
        };
        
        let updated_at = folder_times
            .get(&folder_name)
            .copied()
            .unwrap_or_else(Utc::now);
        
        entries.push(DirectoryEntry {
            id: folder_id,
            name: folder_name,
            path: full_folder_path,
            is_folder: true,
            size_bytes: 0,
            updated_at,
            version_id: None,
        });
    }
    
    // Sort by name (folders and files mixed, alphabetically)
    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    
    Ok(entries)
}
