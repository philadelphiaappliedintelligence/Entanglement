use super::DbPool;
use chrono::{DateTime, Utc};
use uuid::Uuid;

#[allow(dead_code)]
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub password_hash: String,
    pub is_admin: bool,
    pub created_at: DateTime<Utc>,
}

/// Create a new user
pub async fn create_user(pool: &DbPool, username: &str, password_hash: &str, is_admin: bool) -> anyhow::Result<User> {
    let user = sqlx::query_as::<_, User>(
        r#"
        INSERT INTO users (username, email, password_hash, is_admin)
        VALUES ($1, $1 || '@localhost', $2, $3)
        RETURNING id, username, password_hash, is_admin, created_at
        "#,
    )
    .bind(username)
    .bind(password_hash)
    .bind(is_admin)
    .fetch_one(pool)
    .await?;

    Ok(user)
}

/// Get a user by username
pub async fn get_user_by_username(pool: &DbPool, username: &str) -> anyhow::Result<Option<User>> {
    let user = sqlx::query_as::<_, User>(
        r#"
        SELECT id, username, password_hash, is_admin, created_at
        FROM users
        WHERE username = $1
        "#,
    )
    .bind(username)
    .fetch_optional(pool)
    .await?;

    Ok(user)
}

/// Get a user by ID
#[allow(dead_code)]
pub async fn get_user_by_id(pool: &DbPool, id: Uuid) -> anyhow::Result<Option<User>> {
    let user = sqlx::query_as::<_, User>(
        r#"
        SELECT id, username, password_hash, is_admin, created_at
        FROM users
        WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(user)
}

/// List all users
pub async fn list_users(pool: &DbPool) -> anyhow::Result<Vec<User>> {
    let users = sqlx::query_as::<_, User>(
        r#"
        SELECT id, username, password_hash, is_admin, created_at
        FROM users
        ORDER BY created_at DESC
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(users)
}

/// Update a user's password
pub async fn update_password(pool: &DbPool, user_id: Uuid, password_hash: &str) -> anyhow::Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE users SET password_hash = $2 WHERE id = $1
        "#,
    )
    .bind(user_id)
    .bind(password_hash)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Delete a user
pub async fn delete_user(pool: &DbPool, user_id: Uuid) -> anyhow::Result<bool> {
    let result = sqlx::query(
        r#"
        DELETE FROM users WHERE id = $1
        "#,
    )
    .bind(user_id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Set admin status for a user
pub async fn set_admin(pool: &DbPool, user_id: Uuid, is_admin: bool) -> anyhow::Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE users SET is_admin = $2 WHERE id = $1
        "#,
    )
    .bind(user_id)
    .bind(is_admin)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}
