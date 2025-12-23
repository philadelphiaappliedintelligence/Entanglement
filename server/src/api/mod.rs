pub mod rest;
pub mod ws;

use crate::config::Config;
use crate::db::DbPool;
use crate::storage::BlobManager;
use std::sync::Arc;

pub use ws::SyncHub;

#[derive(Clone)]
pub struct AppState {
    pub db: DbPool,
    /// Unified blob manager (handles both chunked storage and legacy blobs)
    pub blob_manager: Arc<BlobManager>,
    pub config: Config,
    /// WebSocket sync hub for real-time notifications
    pub sync_hub: SyncHub,
}

impl AppState {
    pub fn new(
        db: DbPool,
        blob_manager: BlobManager,
        config: Config,
    ) -> Self {
        Self {
            db,
            blob_manager: Arc::new(blob_manager),
            config,
            sync_hub: SyncHub::default(),
        }
    }
}
