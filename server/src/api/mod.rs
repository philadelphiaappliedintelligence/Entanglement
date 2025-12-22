pub mod grpc;
pub mod rest;
pub mod ws;

use crate::config::Config;
use crate::db::DbPool;
use crate::storage::{BlobManager, BlobStore};
use std::sync::Arc;

pub use ws::SyncHub;

#[derive(Clone)]
pub struct AppState {
    pub db: DbPool,
    /// Legacy blob store (for standalone blobs)
    pub blob_store: Arc<BlobStore>,
    /// New container-based blob manager (for chunked storage)
    pub blob_manager: Arc<BlobManager>,
    pub config: Config,
    /// WebSocket sync hub for real-time notifications
    pub sync_hub: SyncHub,
}

impl AppState {
    pub fn new(
        db: DbPool,
        blob_store: BlobStore,
        blob_manager: BlobManager,
        config: Config,
    ) -> Self {
        Self {
            db,
            blob_store: Arc::new(blob_store),
            blob_manager: Arc::new(blob_manager),
            config,
            sync_hub: SyncHub::default(),
        }
    }
}


