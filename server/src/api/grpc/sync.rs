use super::proto::*;
use crate::api::AppState;
use crate::db::{files, versions};
use crate::storage::cas;
use std::pin::Pin;
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, Stream, StreamExt};
use tonic::{Request, Response, Status, Streaming};
use uuid::Uuid;

pub struct SyncServiceImpl {
    state: AppState,
}

impl SyncServiceImpl {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }

    // Extract user ID from request metadata (JWT token)
    fn get_user_id(&self, request: &Request<impl std::any::Any>) -> Result<Uuid, Status> {
        // Extract Authorization header with Bearer token
        let auth_header = request
            .metadata()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| Status::unauthenticated("Missing authorization header"))?;

        // Parse Bearer token
        let token = auth_header
            .strip_prefix("Bearer ")
            .or_else(|| auth_header.strip_prefix("bearer "))
            .ok_or_else(|| Status::unauthenticated("Invalid authorization format - expected 'Bearer <token>'"))?;

        // Validate JWT and extract user ID
        crate::auth::verify_token(&self.state.config.jwt_secret, token)
            .map_err(|e| {
                tracing::warn!("JWT validation failed: {}", e);
                Status::unauthenticated("Invalid or expired token")
            })
    }
}

type ResponseStream<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send>>;

#[tonic::async_trait]
impl sync_service_server::SyncService for SyncServiceImpl {
    type GetFileStream = ResponseStream<GetFileResponse>;
    type GetChangesStream = ResponseStream<ChangeEvent>;

    async fn push_file(
        &self,
        request: Request<Streaming<PushFileRequest>>,
    ) -> Result<Response<PushFileResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let mut stream = request.into_inner();

        // First message should contain metadata
        let first_msg = stream
            .next()
            .await
            .ok_or_else(|| Status::invalid_argument("Empty stream"))?
            .map_err(|e| Status::internal(e.to_string()))?;

        let metadata = match first_msg.data {
            Some(push_file_request::Data::Metadata(m)) => m,
            _ => return Err(Status::invalid_argument("First message must be metadata")),
        };

        // Collect all chunks
        let mut content = Vec::new();
        while let Some(msg) = stream.next().await {
            let msg = msg.map_err(|e| Status::internal(e.to_string()))?;
            if let Some(push_file_request::Data::Chunk(chunk)) = msg.data {
                content.extend_from_slice(&chunk);
            }
        }

        // Compute hash and store blob
        let blob_hash = cas::compute_hash(&content);
        let deduplicated = self
            .state
            .blob_store
            .exists(&blob_hash)
            .map_err(|e| Status::internal(e.to_string()))?;

        if !deduplicated {
            self.state
                .blob_store
                .write(&blob_hash, &content)
                .map_err(|e| Status::internal(e.to_string()))?;
        }

        // Create or update file record
        let file = files::upsert_file(&self.state.db, user_id, &metadata.path)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        // Create new version
        let version = versions::create_version(
            &self.state.db,
            file.id,
            &blob_hash,
            content.len() as i64,
            user_id,
        )
        .await
        .map_err(|e| Status::internal(e.to_string()))?;

        // Update file's current version
        files::set_current_version(&self.state.db, file.id, version.id)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(PushFileResponse {
            file_id: file.id.to_string(),
            version_id: version.id.to_string(),
            blob_hash,
            deduplicated,
        }))
    }

    async fn get_file(
        &self,
        request: Request<GetFileRequest>,
    ) -> Result<Response<Self::GetFileStream>, Status> {
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();

        // Find the file
        let file = match req.identifier {
            Some(get_file_request::Identifier::FileId(id)) => {
                let file_id = Uuid::parse_str(&id)
                    .map_err(|_| Status::invalid_argument("Invalid file ID"))?;
                files::get_file_by_id(&self.state.db, file_id, user_id).await
            }
            Some(get_file_request::Identifier::Path(path)) => {
                files::get_file_by_path(&self.state.db, user_id, &path).await
            }
            None => return Err(Status::invalid_argument("Must specify file_id or path")),
        }
        .map_err(|e| Status::internal(e.to_string()))?
        .ok_or_else(|| Status::not_found("File not found"))?;

        // Get the requested version or current version
        let version = if req.version_id.is_empty() {
            let version_id = file
                .current_version_id
                .ok_or_else(|| Status::not_found("File has no versions"))?;
            versions::get_version(&self.state.db, version_id).await
        } else {
            let version_id = Uuid::parse_str(&req.version_id)
                .map_err(|_| Status::invalid_argument("Invalid version ID"))?;
            versions::get_version(&self.state.db, version_id).await
        }
        .map_err(|e| Status::internal(e.to_string()))?
        .ok_or_else(|| Status::not_found("Version not found"))?;

        // Read blob content
        let content = self
            .state
            .blob_store
            .read(&version.blob_hash)
            .map_err(|e| Status::internal(e.to_string()))?;

        // Create response stream
        let (tx, rx) = mpsc::channel(4);

        // Send file info first
        let file_info = FileInfo {
            file_id: file.id.to_string(),
            version_id: version.id.to_string(),
            path: file.path.clone(),
            size_bytes: version.size_bytes,
            blob_hash: version.blob_hash.clone(),
            created_at: version.created_at.timestamp(),
        };

        let tx_clone = tx.clone();
        tokio::spawn(async move {
            // Send file info
            let _ = tx_clone
                .send(Ok(GetFileResponse {
                    data: Some(get_file_response::Data::Info(file_info)),
                }))
                .await;

            // Send content in chunks (64KB each)
            const CHUNK_SIZE: usize = 64 * 1024;
            for chunk in content.chunks(CHUNK_SIZE) {
                let _ = tx_clone
                    .send(Ok(GetFileResponse {
                        data: Some(get_file_response::Data::Chunk(chunk.to_vec())),
                    }))
                    .await;
            }
        });

        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }

    async fn list_versions(
        &self,
        request: Request<ListVersionsRequest>,
    ) -> Result<Response<ListVersionsResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();

        // Find the file
        let file = match req.identifier {
            Some(list_versions_request::Identifier::FileId(id)) => {
                let file_id = Uuid::parse_str(&id)
                    .map_err(|_| Status::invalid_argument("Invalid file ID"))?;
                files::get_file_by_id(&self.state.db, file_id, user_id).await
            }
            Some(list_versions_request::Identifier::Path(path)) => {
                files::get_file_by_path(&self.state.db, user_id, &path).await
            }
            None => return Err(Status::invalid_argument("Must specify file_id or path")),
        }
        .map_err(|e| Status::internal(e.to_string()))?
        .ok_or_else(|| Status::not_found("File not found"))?;

        let limit = if req.limit > 0 { req.limit } else { 50 };
        let offset = req.offset;

        let (vers, total) =
            versions::list_versions(&self.state.db, file.id, limit as i64, offset as i64)
                .await
                .map_err(|e| Status::internal(e.to_string()))?;

        let versions = vers
            .into_iter()
            .map(|v| VersionInfo {
                version_id: v.id.to_string(),
                blob_hash: v.blob_hash,
                size_bytes: v.size_bytes,
                created_at: v.created_at.timestamp(),
                created_by: v.created_by.map(|u| u.to_string()).unwrap_or_default(),
            })
            .collect();

        Ok(Response::new(ListVersionsResponse {
            versions,
            total_count: total as i32,
        }))
    }

    async fn get_changes(
        &self,
        request: Request<GetChangesRequest>,
    ) -> Result<Response<Self::GetChangesStream>, Status> {
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();

        let cursor = if req.cursor.is_empty() {
            None
        } else {
            Some(
                chrono::DateTime::parse_from_rfc3339(&req.cursor)
                    .map_err(|_| Status::invalid_argument("Invalid cursor format"))?
                    .with_timezone(&chrono::Utc),
            )
        };

        let limit = if req.limit > 0 { req.limit } else { 100 };

        let changes = files::get_changes(&self.state.db, user_id, cursor, limit as i64)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let (tx, rx) = mpsc::channel(4);

        tokio::spawn(async move {
            for change in changes {
                let change_type = if change.is_deleted {
                    change_event::ChangeType::Deleted
                } else if change.created_at == change.updated_at {
                    change_event::ChangeType::Created
                } else {
                    change_event::ChangeType::Modified
                };

                let event = ChangeEvent {
                    change_type: change_type as i32,
                    file_id: change.id.to_string(),
                    path: change.path,
                    version_id: change.current_version_id.map(|v| v.to_string()).unwrap_or_default(),
                    size_bytes: change.size_bytes.unwrap_or(0),
                    blob_hash: change.blob_hash.unwrap_or_default(),
                    timestamp: change.updated_at.timestamp(),
                    cursor: change.updated_at.to_rfc3339(),
                };

                if tx.send(Ok(event)).await.is_err() {
                    break;
                }
            }
        });

        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }

    async fn delete_file(
        &self,
        request: Request<DeleteFileRequest>,
    ) -> Result<Response<DeleteFileResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();

        // Find the file
        let file = match req.identifier {
            Some(delete_file_request::Identifier::FileId(id)) => {
                let file_id = Uuid::parse_str(&id)
                    .map_err(|_| Status::invalid_argument("Invalid file ID"))?;
                files::get_file_by_id(&self.state.db, file_id, user_id).await
            }
            Some(delete_file_request::Identifier::Path(path)) => {
                files::get_file_by_path(&self.state.db, user_id, &path).await
            }
            None => return Err(Status::invalid_argument("Must specify file_id or path")),
        }
        .map_err(|e| Status::internal(e.to_string()))?
        .ok_or_else(|| Status::not_found("File not found"))?;

        // Soft delete (recursive for directories)
        files::soft_delete_recursive(&self.state.db, file.id)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(DeleteFileResponse {
            success: true,
            file_id: file.id.to_string(),
        }))
    }

    async fn list_files(
        &self,
        request: Request<ListFilesRequest>,
    ) -> Result<Response<ListFilesResponse>, Status> {
        let user_id = self.get_user_id(&request)?;
        let req = request.into_inner();

        let prefix = if req.prefix.is_empty() {
            None
        } else {
            Some(req.prefix.as_str())
        };

        let limit = if req.limit > 0 { req.limit } else { 100 };
        let offset = req.offset;

        let (file_list, total) = files::list_files(
            &self.state.db,
            user_id,
            prefix,
            req.include_deleted,
            limit as i64,
            offset as i64,
        )
        .await
        .map_err(|e| Status::internal(e.to_string()))?;

        let files = file_list
            .into_iter()
            .map(|f| FileEntry {
                file_id: f.id.to_string(),
                path: f.path,
                size_bytes: f.size_bytes.unwrap_or(0),
                blob_hash: f.blob_hash.unwrap_or_default(),
                is_deleted: f.is_deleted,
                created_at: f.created_at.timestamp(),
                updated_at: f.updated_at.timestamp(),
            })
            .collect();

        Ok(Response::new(ListFilesResponse {
            files,
            total_count: total as i32,
        }))
    }
}

