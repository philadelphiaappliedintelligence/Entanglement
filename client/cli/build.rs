fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .build_server(false)
        .build_client(true)
        .compile(&["../../server/proto/sync.proto"], &["../../server/proto/"])?;
    Ok(())
}















