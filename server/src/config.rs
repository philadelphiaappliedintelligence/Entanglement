use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    pub server_name: String,
    pub database_url: String,
    pub blob_storage_path: String,
    pub grpc_port: u16,
    pub rest_port: u16,
    pub jwt_secret: String,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Config {
            server_name: std::env::var("SERVER_NAME")
                .unwrap_or_else(|_| "Entanglement".to_string()),
            database_url: std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://entanglement:entanglement@localhost:5432/entanglement".to_string()),
            blob_storage_path: std::env::var("BLOB_STORAGE_PATH")
                .unwrap_or_else(|_| "./data/blobs".to_string()),
            grpc_port: std::env::var("GRPC_PORT")
                .unwrap_or_else(|_| "50051".to_string())
                .parse()?,
            rest_port: std::env::var("REST_PORT")
                .unwrap_or_else(|_| "8080".to_string())
                .parse()?,
            jwt_secret: std::env::var("JWT_SECRET")
                .expect("JWT_SECRET environment variable must be set. Generate with: openssl rand -hex 32"),
        })
    }

    pub fn set_server_name(&mut self, name: String) {
        self.server_name = name;
    }
}

