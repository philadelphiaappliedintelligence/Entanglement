use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Access token expiration time in hours
/// SECURITY: Reduced from 30 days to 24 hours for better security
const ACCESS_TOKEN_HOURS: i64 = 24;

/// Refresh token expiration time in days
const REFRESH_TOKEN_DAYS: i64 = 30;

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,       // User ID
    exp: i64,          // Expiration time
    iat: i64,          // Issued at
    token_type: String, // "access" or "refresh"
}

/// Create an access JWT token for a user (short-lived)
pub fn create_token(secret: &str, user_id: Uuid) -> anyhow::Result<String> {
    create_access_token(secret, user_id)
}

/// Create an access token (short-lived, for API requests)
pub fn create_access_token(secret: &str, user_id: Uuid) -> anyhow::Result<String> {
    let now = Utc::now();
    let exp = now + Duration::hours(ACCESS_TOKEN_HOURS);

    let claims = Claims {
        sub: user_id.to_string(),
        exp: exp.timestamp(),
        iat: now.timestamp(),
        token_type: "access".to_string(),
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )?;

    Ok(token)
}

/// Create a refresh token (long-lived, for obtaining new access tokens)
pub fn create_refresh_token(secret: &str, user_id: Uuid) -> anyhow::Result<String> {
    let now = Utc::now();
    let exp = now + Duration::days(REFRESH_TOKEN_DAYS);

    let claims = Claims {
        sub: user_id.to_string(),
        exp: exp.timestamp(),
        iat: now.timestamp(),
        token_type: "refresh".to_string(),
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )?;

    Ok(token)
}

/// Verify a JWT token and extract the user ID
pub fn verify_token(secret: &str, token: &str) -> anyhow::Result<Uuid> {
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )?;

    let user_id = Uuid::parse_str(&token_data.claims.sub)?;
    Ok(user_id)
}

/// Verify a refresh token and extract the user ID
/// Returns an error if the token is not a refresh token
pub fn verify_refresh_token(secret: &str, token: &str) -> anyhow::Result<Uuid> {
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )?;

    if token_data.claims.token_type != "refresh" {
        anyhow::bail!("Not a refresh token");
    }

    let user_id = Uuid::parse_str(&token_data.claims.sub)?;
    Ok(user_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_roundtrip() {
        let secret = "test_secret";
        let user_id = Uuid::new_v4();

        let token = create_token(secret, user_id).unwrap();
        let extracted_id = verify_token(secret, &token).unwrap();

        assert_eq!(user_id, extracted_id);
    }

    #[test]
    fn test_invalid_token() {
        let secret = "test_secret";
        let result = verify_token(secret, "invalid_token");
        assert!(result.is_err());
    }
}








