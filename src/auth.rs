use crate::error::{AppError, AppResult};
use crate::models::{UserId, User};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: UserId,
    pub username: String,
    pub exp: usize,
}

pub struct Auth {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
}

impl Auth {
    pub fn new(secret: &str) -> Self {
        Self {
            encoding_key: EncodingKey::from_secret(secret.as_bytes()),
            decoding_key: DecodingKey::from_secret(secret.as_bytes()),
        }
    }

    /// Create a JWT token for a user.
    pub fn create_token(&self, user: &User) -> AppResult<String> {
        let claims = Claims {
            sub: user.id,
            username: user.username.clone(),
            exp: (chrono::Utc::now() + chrono::Duration::hours(24)).timestamp() as usize,
        };
        encode(&Header::default(), &claims, &self.encoding_key)
            .map_err(|e| AppError::Auth(format!("Token creation failed: {e}")))
    }

    /// Validate a JWT token and return the claims.
    pub fn validate_token(&self, token: &str) -> AppResult<Claims> {
        let data = decode::<Claims>(token, &self.decoding_key, &Validation::default())
            .map_err(|e| AppError::Auth(format!("Invalid token: {e}")))?;
        Ok(data.claims)
    }
}

/// Authenticate a WebSocket connection.
/// Supports both JWT (`Bearer <token>`) and simple token lookup.
pub async fn authenticate(pool: &PgPool, auth: &Auth, raw_token: &str) -> AppResult<User> {
    let token = raw_token.trim().strip_prefix("Bearer ").unwrap_or(raw_token.trim());

    // Try JWT first
    if let Ok(claims) = auth.validate_token(token) {
        let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
            .bind(claims.sub)
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| AppError::Auth("User not found".into()))?;
        return Ok(user);
    }

    // Fall back to simple token lookup (for development)
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE token = $1")
        .bind(token)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::Auth("Invalid token".into()))?;

    Ok(user)
}
