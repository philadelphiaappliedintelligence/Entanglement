mod sync;

use crate::api::AppState;
use std::net::SocketAddr;
use tonic::transport::Server;

pub use sync::SyncServiceImpl;

// Include the generated protobuf code
pub mod proto {
    tonic::include_proto!("entanglement.sync");
}

pub async fn serve(addr: SocketAddr, state: AppState) -> anyhow::Result<()> {
    let sync_service = SyncServiceImpl::new(state);

    Server::builder()
        .add_service(proto::sync_service_server::SyncServiceServer::new(
            sync_service,
        ))
        .serve(addr)
        .await?;

    Ok(())
}













