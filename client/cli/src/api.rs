use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub struct ApiClient {
    base_url: String,
    client: reqwest::Client,
}

// --- Auth types ---

#[derive(Debug, Serialize)]
struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Debug, Deserialize)]
pub struct TokenPair {
    pub token: String,
    pub refresh_token: String,
    pub user_id: String,
    pub username: String,
}

#[derive(Debug, Serialize)]
struct RefreshRequest {
    refresh_token: String,
}

// --- Server info ---

#[derive(Debug, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

// --- File types ---

#[derive(Debug, Deserialize)]
struct FileListResponse {
    files: Vec<FileInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileInfo {
    pub id: Uuid,
    pub path: String,
    pub size_bytes: i64,
    pub blob_hash: Option<String>,
    pub is_directory: bool,
    pub is_deleted: bool,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
struct VersionListResponse {
    versions: Vec<VersionInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VersionInfo {
    pub id: Uuid,
    pub blob_hash: String,
    pub size_bytes: i64,
    pub created_at: String,
}

// --- Chunk types ---

#[derive(Debug, Serialize)]
struct ChunkCheckRequest {
    hashes: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChunkCheckResponse {
    pub existing: Vec<String>,
    pub missing: Vec<String>,
}

// --- V1 file creation ---

#[derive(Debug, Serialize)]
struct CreateFileRequest {
    path: String,
    size_bytes: i64,
    modified_at: String,
    tier_id: u8,
    content_hash: String,
    chunk_hashes: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateFileResponse {
    pub id: Uuid,
    pub version_id: Uuid,
    pub path: String,
}

// --- Changes ---

#[derive(Debug, Deserialize)]
pub struct ChangesResponse {
    pub changes: Vec<FileChange>,
    pub server_time: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileChange {
    pub id: Uuid,
    pub path: String,
    pub action: String,
    pub size_bytes: Option<i64>,
    pub blob_hash: Option<String>,
    pub is_directory: bool,
    pub updated_at: String,
}

// --- Conflicts ---

#[derive(Debug, Deserialize)]
struct ConflictListResponse {
    conflicts: Vec<Conflict>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Conflict {
    pub id: Uuid,
    pub file_id: Uuid,
    pub file_path: String,
    pub conflict_type: String,
    pub detected_at: String,
}

impl ApiClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Check response status; on error, read body for detail message.
    async fn ensure_ok(resp: reqwest::Response) -> anyhow::Result<reqwest::Response> {
        if resp.status().is_success() {
            Ok(resp)
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("API error ({}): {}", status, body)
        }
    }

    pub async fn get_server_info(&self) -> anyhow::Result<ServerInfo> {
        let resp = self
            .client
            .get(format!("{}/server/info", self.base_url))
            .send()
            .await?;
        Ok(Self::ensure_ok(resp).await?.json().await?)
    }

    pub async fn login(&self, username: &str, password: &str) -> anyhow::Result<TokenPair> {
        let resp = self
            .client
            .post(format!("{}/auth/login", self.base_url))
            .json(&LoginRequest {
                username: username.to_string(),
                password: password.to_string(),
            })
            .send()
            .await?;
        Ok(Self::ensure_ok(resp).await?.json().await?)
    }

    pub async fn refresh_token(&self, refresh: &str) -> anyhow::Result<TokenPair> {
        let resp = self
            .client
            .post(format!("{}/auth/refresh", self.base_url))
            .json(&RefreshRequest {
                refresh_token: refresh.to_string(),
            })
            .send()
            .await?;
        Ok(Self::ensure_ok(resp).await?.json().await?)
    }

    pub async fn list_files(&self, token: &str) -> anyhow::Result<Vec<FileInfo>> {
        let resp = self
            .client
            .get(format!("{}/files", self.base_url))
            .bearer_auth(token)
            .send()
            .await?;
        let list: FileListResponse = Self::ensure_ok(resp).await?.json().await?;
        Ok(list.files)
    }

    pub async fn get_file_versions(
        &self,
        token: &str,
        file_id: Uuid,
    ) -> anyhow::Result<Vec<VersionInfo>> {
        let resp = self
            .client
            .get(format!("{}/files/{}/versions", self.base_url, file_id))
            .bearer_auth(token)
            .send()
            .await?;
        let list: VersionListResponse = Self::ensure_ok(resp).await?.json().await?;
        Ok(list.versions)
    }

    pub async fn check_chunks(
        &self,
        token: &str,
        hashes: &[String],
    ) -> anyhow::Result<ChunkCheckResponse> {
        let resp = self
            .client
            .post(format!("{}/v1/chunks/check", self.base_url))
            .bearer_auth(token)
            .json(&ChunkCheckRequest {
                hashes: hashes.to_vec(),
            })
            .send()
            .await?;
        Ok(Self::ensure_ok(resp).await?.json().await?)
    }

    pub async fn upload_chunk(
        &self,
        token: &str,
        hash: &str,
        data: &[u8],
        tier: u8,
    ) -> anyhow::Result<()> {
        let resp = self
            .client
            .put(format!("{}/v1/chunks/{}", self.base_url, hash))
            .bearer_auth(token)
            .header("Content-Type", "application/octet-stream")
            .header("X-Chunk-Tier", tier.to_string())
            .body(data.to_vec())
            .send()
            .await?;
        Self::ensure_ok(resp).await?;
        Ok(())
    }

    pub async fn create_file(
        &self,
        token: &str,
        path: &str,
        size: i64,
        modified_at: &str,
        tier_id: u8,
        content_hash: &str,
        chunk_hashes: Vec<String>,
    ) -> anyhow::Result<CreateFileResponse> {
        let resp = self
            .client
            .post(format!("{}/v1/files", self.base_url))
            .bearer_auth(token)
            .json(&CreateFileRequest {
                path: path.to_string(),
                size_bytes: size,
                modified_at: modified_at.to_string(),
                tier_id,
                content_hash: content_hash.to_string(),
                chunk_hashes,
            })
            .send()
            .await?;
        Ok(Self::ensure_ok(resp).await?.json().await?)
    }

    pub async fn download_file(&self, token: &str, version_id: Uuid) -> anyhow::Result<Vec<u8>> {
        let resp = self
            .client
            .get(format!(
                "{}/v1/files/{}/download",
                self.base_url, version_id
            ))
            .bearer_auth(token)
            .send()
            .await?;
        Ok(Self::ensure_ok(resp).await?.bytes().await?.to_vec())
    }

    pub async fn get_changes(
        &self,
        token: &str,
        since: Option<&str>,
    ) -> anyhow::Result<ChangesResponse> {
        let mut req = self
            .client
            .get(format!("{}/v1/files/changes", self.base_url))
            .bearer_auth(token);
        if let Some(since) = since {
            req = req.query(&[("since", since)]);
        }
        let resp = req.send().await?;
        Ok(Self::ensure_ok(resp).await?.json().await?)
    }

    pub async fn check_conflicts(&self, token: &str) -> anyhow::Result<Vec<Conflict>> {
        let resp = self
            .client
            .get(format!("{}/conflicts", self.base_url))
            .bearer_auth(token)
            .send()
            .await?;
        let list: ConflictListResponse = Self::ensure_ok(resp).await?.json().await?;
        Ok(list.conflicts)
    }
}
