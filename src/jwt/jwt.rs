use actix_web::{HttpRequest, HttpResponse};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::env;

// JWT Claims 结构体
#[derive(Serialize, Deserialize)]
pub struct Claims {
    pub sub: String, // 用户名
    pub iat: i64,    // 签发时间
    pub exp: i64,    // 过期时间
    pub jti: String, // Token ID (UUID)
}

// 从环境变量获取 JWT 密钥
pub fn jwt_secret() -> Vec<u8> {
    env::var("JWT_SECRET")
        .unwrap_or_else(|_| "your-secret-key-change-in-production".to_string())
        .into_bytes()
}

// 从请求头中提取并验证 JWT token
pub fn extract_and_validate_token(req: &HttpRequest) -> Result<Claims, HttpResponse> {
    let auth_header = req
        .headers()
        .get("Authorization")
        .ok_or_else(|| HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "Missing Authorization header"
        })))?;

    let auth_str = auth_header
        .to_str()
        .map_err(|_| HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "Invalid Authorization header"
        })))?;

    let jwt_token = auth_str
        .strip_prefix("Bearer ")
        .ok_or_else(|| HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "Invalid Authorization format, expected 'Bearer <token>'"
        })))?;

    let token_data = decode::<Claims>(
        jwt_token,
        &DecodingKey::from_secret(&jwt_secret()),
        &Validation::default(),
    )
    .map_err(|e| HttpResponse::Unauthorized().json(serde_json::json!({
        "error": format!("Invalid token: {}", e)
    })))?;

    Ok(token_data.claims)
}

// 生成 JWT token 的辅助函数
pub fn generate_token(sub: String, iat: i64, exp: i64, jti: String) -> Result<String, String> {
    let claims = Claims {
        sub,
        iat,
        exp,
        jti,
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(&jwt_secret()),
    )
    .map_err(|e| format!("Failed to encode token: {}", e))
}
