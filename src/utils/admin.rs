use diesel::prelude::*;

use crate::models::User;
use crate::schema::users::dsl::*;

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
        Ok(Some(user)) => Ok(user.type_ == "admin"),
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
