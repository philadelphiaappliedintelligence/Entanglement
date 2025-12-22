use serde::{Deserialize, Serialize};

pub struct RestClient {
    base_url: String,
    client: reqwest::Client,
}

#[derive(Debug, Serialize)]
struct LoginRequest {
    email: String,
    password: String,
}

#[derive(Debug, Deserialize)]
pub struct AuthResponse {
    pub token: String,
    pub user_id: String,
}

#[derive(Debug, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    pub grpc_port: u16,
}

impl RestClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
        }
    }

    pub async fn login(&self, email: &str, password: &str) -> anyhow::Result<AuthResponse> {
        let url = format!("{}/auth/login", self.base_url);
        
        let resp = self.client
            .post(&url)
            .json(&LoginRequest {
                email: email.to_string(),
                password: password.to_string(),
            })
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Login failed ({}): {}", status, body);
        }

        let auth: AuthResponse = resp.json().await?;
        Ok(auth)
    }

    #[allow(dead_code)]
    pub async fn register(&self, email: &str, password: &str) -> anyhow::Result<AuthResponse> {
        let url = format!("{}/auth/register", self.base_url);
        
        let resp = self.client
            .post(&url)
            .json(&LoginRequest {
                email: email.to_string(),
                password: password.to_string(),
            })
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Registration failed ({}): {}", status, body);
        }

        let auth: AuthResponse = resp.json().await?;
        Ok(auth)
    }

    pub async fn get_server_info(&self) -> anyhow::Result<ServerInfo> {
        let url = format!("{}/server/info", self.base_url);
        
        let resp = self.client
            .get(&url)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Failed to get server info ({}): {}", status, body);
        }

        let info: ServerInfo = resp.json().await?;
        Ok(info)
    }
}

