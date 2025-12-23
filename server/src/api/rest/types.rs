//! Shared types for REST API
//!
//! Common request/response structs used across multiple endpoint modules.

use serde::{Deserialize, Serialize};

// ============================================================================
// FILE RESPONSES
// ============================================================================

#[derive(Serialize)]
pub struct FileResponse {
    pub id: String,
    pub path: String,
    pub size_bytes: Option<i64>,
    pub blob_hash: Option<String>,
    pub is_directory: bool,
    pub is_deleted: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Deserialize)]
pub struct ListFilesQuery {
    pub prefix: Option<String>,
    pub include_deleted: Option<bool>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Serialize)]
pub struct ListFilesResponse {
    pub files: Vec<FileResponse>,
    pub total: i64,
}

// ============================================================================
// DIRECTORY RESPONSES
// ============================================================================

#[derive(Serialize)]
pub struct DirectoryEntryResponse {
    pub id: String,
    pub name: String,
    pub path: String,
    pub is_folder: bool,
    pub size_bytes: i64,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_id: Option<String>,
}

#[derive(Serialize)]
pub struct ListDirectoryResponse {
    pub entries: Vec<DirectoryEntryResponse>,
    pub path: String,
}

#[derive(Deserialize)]
pub struct ListDirectoryQuery {
    #[serde(default)]
    pub path: String,
}

// ============================================================================
// UPLOAD RESPONSES
// ============================================================================

#[derive(Serialize)]
pub struct UploadResponse {
    pub id: String,
    pub path: String,
    pub blob_hash: String,
    pub size_bytes: i64,
}
