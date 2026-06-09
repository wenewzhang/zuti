use actix_web::{http::header::ContentType, web, HttpRequest, HttpResponse};
use diesel::prelude::*;
use serde::Serialize;

use crate::jwt::extract_and_validate_token;
use crate::models::User;
use crate::schema::users;
use crate::schema::users::dsl::*;
use crate::utils::token_cache::TOKEN_CACHE;

/// 通用错误响应结构体
#[derive(Serialize)]
pub struct ErrorResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

/// 检查用户 type_ 是否为 share 或 admin
///
/// # Arguments
/// * `pool` - 数据库连接池
/// * `username` - 用户名
///
/// # Returns
/// * `Ok(true)` - 用户是 share 或 admin
/// * `Ok(false)` - 用户不是 share 也不是 admin
/// * `Err(AdminCheckError::UserNotFound)` - 用户不存在
/// * `Err(AdminCheckError::DatabaseError)` - 数据库查询错误
pub fn is_share_or_read(pool: &crate::DbPool, username: &str) -> Result<bool, AdminCheckError> {
    let mut conn = pool.get().map_err(|_| AdminCheckError::DatabaseError)?;

    let user_result: Result<Option<User>, _> = users
        .filter(name.eq(username))
        .first::<User>(&mut conn)
        .optional();

    match user_result {
        Ok(Some(user)) => Ok(user.type_ == crate::utils::consts::USER_TYPE_SHARE || user.type_ == crate::utils::consts::USER_TYPE_READ),
        Ok(None) => Err(AdminCheckError::UserNotFound),
        Err(_) => Err(AdminCheckError::DatabaseError),
    }
}

/// 检查用户 type_ 是否为 share 或 admin
///
/// # Arguments
/// * `pool` - 数据库连接池
/// * `username` - 用户名
///
/// # Returns
/// * `Ok(true)` - 用户是 share 或 admin
/// * `Ok(false)` - 用户不是 share 也不是 admin
/// * `Err(AdminCheckError::UserNotFound)` - 用户不存在
/// * `Err(AdminCheckError::DatabaseError)` - 数据库查询错误
pub fn is_share_or_admin(pool: &crate::DbPool, username: &str) -> Result<bool, AdminCheckError> {
    let mut conn = pool.get().map_err(|_| AdminCheckError::DatabaseError)?;

    let user_result: Result<Option<User>, _> = users
        .filter(name.eq(username))
        .first::<User>(&mut conn)
        .optional();

    match user_result {
        Ok(Some(user)) => Ok(user.type_ == crate::utils::consts::USER_TYPE_SHARE || user.type_ == crate::utils::consts::USER_TYPE_ADMIN),
        Ok(None) => Err(AdminCheckError::UserNotFound),
        Err(_) => Err(AdminCheckError::DatabaseError),
    }
}

/// 检查用户是否为 admin
///
/// # Arguments
/// * `pool` - 数据库连接池
/// * `username` - 用户名
///
/// # Returns
/// * `Ok(true)` - 用户是 admin
/// * `Ok(false)` - 用户不是 admin
/// * `Err(AdminCheckError::UserNotFound)` - 用户不存在
/// * `Err(AdminCheckError::DatabaseError)` - 数据库查询错误
pub fn is_admin(pool: &crate::DbPool, username: &str) -> Result<bool, AdminCheckError> {
    let mut conn = pool.get().map_err(|_| AdminCheckError::DatabaseError)?;

    let user_result: Result<Option<User>, _> = users
        .filter(name.eq(username))
        .first::<User>(&mut conn)
        .optional();

    match user_result {
        Ok(Some(user)) => Ok(user.type_ == crate::utils::consts::USER_TYPE_ADMIN),
        Ok(None) => Err(AdminCheckError::UserNotFound),
        Err(_) => Err(AdminCheckError::DatabaseError),
    }
}

/// 管理员检查错误类型
#[derive(Debug)]
pub enum AdminCheckError {
    UserNotFound,
    DatabaseError,
}

impl std::fmt::Display for AdminCheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdminCheckError::UserNotFound => write!(f, "User not found"),
            AdminCheckError::DatabaseError => write!(f, "Database error"),
        }
    }
}

impl std::error::Error for AdminCheckError {}

/// 验证 JWT token 并检查其与数据库中存储的 token 是否匹配
/// 使用内存缓存减少数据库查询
/// 返回 Ok(claims) 如果验证成功，否则返回 Err(HttpResponse)
pub async fn validate_token_with_db(
    req: &HttpRequest,
    pool: &web::Data<crate::DbPool>,
) -> Result<crate::jwt::Claims, HttpResponse> {
    // 1. 验证 JWT token
    let claims = match extract_and_validate_token(req) {
        Ok(claims) => claims,
        Err(response) => return Err(response),
    };

    let username = claims.sub.clone();
    let token_jti = claims.jti.clone();

    // 2. 检查内存缓存
    if let Some((cached_user, _cached_type)) = TOKEN_CACHE.get(&token_jti) {
        // 缓存命中，验证用户名匹配
        if cached_user == username {
            // 可以选择将 user_type 存入 claims 的扩展字段
            log::debug!("Token cache hit for user: {}", username);
            return Ok(claims);
        }
    }

    // 3. 缓存未命中，查询数据库验证 token
    let pool_clone = pool.get_ref().clone();
    let jti_for_cache = token_jti.clone();
    let username_for_db = username.clone();
    
    let validation_result: Result<(bool, String), String> = web::block(move || {
        let mut conn = pool_clone.get().map_err(|e| format!("Database connection error: {}", e))?;
        
        let user: Option<User> = users::table
            .filter(users::name.eq(&username_for_db))
            .first::<User>(&mut conn)
            .optional()
            .map_err(|e| format!("Database query error: {}", e))?;
        
        match user {
            Some(u) => {
                let valid = u.token.as_ref() == Some(&jti_for_cache);
                Ok((valid, u.type_))
            }
            None => Ok((false, String::new())),
        }
    })
    .await
    .unwrap();

    match validation_result {
        Ok((true, user_type)) => {
            // 验证成功，写入缓存
            TOKEN_CACHE.insert(token_jti, username, user_type);
            Ok(claims)
        }
        Ok((false, _)) => {
            Err(HttpResponse::Unauthorized()
                .content_type(ContentType::json())
                .body(serde_json::to_string(&ErrorResponse {
                    success: false,
                    message: "Invalid token".to_string(),
                    error: Some("Token validation failed".to_string()),
                }).unwrap()))
        }
        Err(e) => {
            Err(HttpResponse::InternalServerError()
                .content_type(ContentType::json())
                .body(serde_json::to_string(&ErrorResponse {
                    success: false,
                    message: "Database error".to_string(),
                    error: Some(e),
                }).unwrap()))
        }
    }
}

/// 验证 JWT token 并检查用户是否为 admin
/// 
/// # Arguments
/// * `req` - HTTP 请求
/// * `pool` - 数据库连接池
/// 
/// # Returns
/// * `Ok(String)` - 验证通过，返回用户名
/// * `Err(HttpResponse)` - 验证失败，返回错误响应
pub async fn verify_admin_access(
    req: &HttpRequest,
    pool: &web::Data<crate::DbPool>,
) -> Result<String, HttpResponse> {
    // 1. 验证 JWT token（包括数据库验证）
    let claims = match validate_token_with_db(req, pool).await {
        Ok(claims) => claims,
        Err(response) => return Err(response),
    };

    // 2. 检查用户是否为 admin
    let username = claims.sub;
    
    match is_admin(pool.get_ref(), &username) {
        Ok(true) => Ok(username),
        Ok(false) => {
            Err(HttpResponse::Forbidden()
                .content_type(ContentType::json())
                .body(serde_json::to_string(&ErrorResponse {
                    success: false,
                    message: "Permission denied".to_string(),
                    error: Some("Only admin users can perform this operation".to_string()),
                }).unwrap()))
        }
        Err(AdminCheckError::UserNotFound) => {
            Err(HttpResponse::Unauthorized()
                .content_type(ContentType::json())
                .body(serde_json::to_string(&ErrorResponse {
                    success: false,
                    message: "User not found".to_string(),
                    error: Some("User does not exist".to_string()),
                }).unwrap()))
        }
        Err(AdminCheckError::DatabaseError) => {
            Err(HttpResponse::InternalServerError()
                .content_type(ContentType::json())
                .body(serde_json::to_string(&ErrorResponse {
                    success: false,
                    message: "Database error".to_string(),
                    error: Some("Failed to query user information".to_string()),
                }).unwrap()))
        }
    }
}

pub async fn verify_share_access(
    req: &HttpRequest,
    pool: &web::Data<crate::DbPool>,
) -> Result<String, HttpResponse> {
    // 1. 验证 JWT token（包括数据库验证）
    let claims = match validate_token_with_db(req, pool).await {
        Ok(claims) => claims,
        Err(response) => return Err(response),
    };

    // 2. 检查用户是否为 admin
    let username = claims.sub;
    
    match is_share_or_admin(pool.get_ref(), &username) {
        Ok(true) => Ok(username),
        Ok(false) => {
            Err(HttpResponse::Forbidden()
                .content_type(ContentType::json())
                .body(serde_json::to_string(&ErrorResponse {
                    success: false,
                    message: "Permission denied".to_string(),
                    error: Some("Only admin users can perform this operation".to_string()),
                }).unwrap()))
        }
        Err(AdminCheckError::UserNotFound) => {
            Err(HttpResponse::Unauthorized()
                .content_type(ContentType::json())
                .body(serde_json::to_string(&ErrorResponse {
                    success: false,
                    message: "User not found".to_string(),
                    error: Some("User does not exist".to_string()),
                }).unwrap()))
        }
        Err(AdminCheckError::DatabaseError) => {
            Err(HttpResponse::InternalServerError()
                .content_type(ContentType::json())
                .body(serde_json::to_string(&ErrorResponse {
                    success: false,
                    message: "Database error".to_string(),
                    error: Some("Failed to query user information".to_string()),
                }).unwrap()))
        }
    }
}
