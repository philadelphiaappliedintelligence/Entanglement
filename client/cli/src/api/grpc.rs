use crate::config::Config;
use std::path::Path;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tonic::Request;

pub mod proto {
    tonic::include_proto!("entanglement.sync");
}

use proto::sync_service_client::SyncServiceClient;
use proto::*;

pub struct GrpcClient {
    client: SyncServiceClient<Channel>,
    token: String,
    user_id: String,
}

impl GrpcClient {
    pub async fn connect(config: &Config) -> anyhow::Result<Self> {
        let grpc_url = config.get_grpc_url()?;
        let channel = Channel::from_shared(grpc_url)?
            .connect()
            .await?;

        let token = config.token.clone()
            .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?;
        let user_id = config.user_id.clone()
            .ok_or_else(|| anyhow::anyhow!("No user ID"))?;

        Ok(Self {
            client: SyncServiceClient::new(channel),
            token,
            user_id,
        })
    }

    fn add_auth<T>(&self, request: &mut Request<T>) {
        // Add user-id for now (JWT validation TODO)
        if let Ok(val) = self.user_id.parse::<MetadataValue<_>>() {
            request.metadata_mut().insert("user-id", val);
        }
        if let Ok(val) = format!("Bearer {}", self.token).parse::<MetadataValue<_>>() {
            request.metadata_mut().insert("authorization", val);
        }
    }

    pub async fn push_file(&mut self, local_path: &Path, remote_path: &str) -> anyhow::Result<PushFileResponse> {
        let mut file = File::open(local_path).await?;
        let metadata = file.metadata().await?;
        let size = metadata.len() as i64;

        // Read file and compute hash
        let mut content = Vec::new();
        file.read_to_end(&mut content).await?;
        let hash = compute_hash(&content);

        // Create stream of messages
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        
        // Send metadata first
        tx.send(PushFileRequest {
            data: Some(push_file_request::Data::Metadata(FileMetadata {
                path: remote_path.to_string(),
                size_bytes: size,
                content_hash: hash,
            })),
        }).await?;

        // Send content in chunks
        const CHUNK_SIZE: usize = 64 * 1024;
        for chunk in content.chunks(CHUNK_SIZE) {
            tx.send(PushFileRequest {
                data: Some(push_file_request::Data::Chunk(chunk.to_vec())),
            }).await?;
        }
        drop(tx);

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        let mut request = Request::new(stream);
        self.add_auth(&mut request);

        let response = self.client.push_file(request).await?;
        Ok(response.into_inner())
    }

    pub async fn pull_file(&mut self, remote_path: &str, local_path: &Path) -> anyhow::Result<()> {
        let mut request = Request::new(GetFileRequest {
            identifier: Some(get_file_request::Identifier::Path(remote_path.to_string())),
            version_id: String::new(),
        });
        self.add_auth(&mut request);

        let response = self.client.get_file(request).await?;
        let mut stream = response.into_inner();

        // Create parent directories
        if let Some(parent) = local_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let mut file = tokio::fs::File::create(local_path).await?;
        
        while let Some(msg) = stream.message().await? {
            match msg.data {
                Some(get_file_response::Data::Info(_info)) => {
                    // File info received
                }
                Some(get_file_response::Data::Chunk(chunk)) => {
                    tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await?;
                }
                None => {}
            }
        }

        Ok(())
    }

    pub async fn list_files(&mut self, prefix: &str) -> anyhow::Result<Vec<FileEntry>> {
        let prefix = if prefix == "/" { String::new() } else { prefix.to_string() };
        
        let mut request = Request::new(ListFilesRequest {
            prefix,
            include_deleted: false,
            limit: 1000,
            offset: 0,
        });
        self.add_auth(&mut request);

        let response = self.client.list_files(request).await?;
        Ok(response.into_inner().files)
    }

    pub async fn list_versions(&mut self, path: &str) -> anyhow::Result<Vec<VersionInfo>> {
        let mut request = Request::new(ListVersionsRequest {
            identifier: Some(list_versions_request::Identifier::Path(path.to_string())),
            limit: 50,
            offset: 0,
        });
        self.add_auth(&mut request);

        let response = self.client.list_versions(request).await?;
        Ok(response.into_inner().versions)
    }

    pub async fn delete_file(&mut self, path: &str) -> anyhow::Result<()> {
        let mut request = Request::new(DeleteFileRequest {
            identifier: Some(delete_file_request::Identifier::Path(path.to_string())),
        });
        self.add_auth(&mut request);

        self.client.delete_file(request).await?;
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn get_changes(&mut self, cursor: Option<&str>) -> anyhow::Result<Vec<ChangeEvent>> {
        let mut request = Request::new(GetChangesRequest {
            cursor: cursor.unwrap_or("").to_string(),
            limit: 100,
        });
        self.add_auth(&mut request);

        let response = self.client.get_changes(request).await?;
        let mut stream = response.into_inner();
        let mut changes = Vec::new();

        while let Some(event) = stream.message().await? {
            changes.push(event);
        }

        Ok(changes)
    }
}

fn compute_hash(content: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content);
    hex::encode(hasher.finalize())
}














