use actix_web::{get, post, web, HttpRequest, HttpResponse, Responder};
use chrono::{Duration, Utc};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use std::process::Command;
use uuid::Uuid;

use crate::jwt;
use crate::models::{NewUser, User, UserInsert};
use crate::schema;
use crate::schema::users::dsl::*;
use crate::utils;
use crate::DbPool;
use crate::utils::admin::validate_token_with_db;
use crate::utils::consts::FORBIDDEN_USERNAME;

// 登录请求结构体
#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

// 登录响应结构体
#[derive(Serialize)]
pub struct LoginResponse {
    pub success: bool,
    pub message: String,
    pub token: Option<String>,
}

// 创建用户响应结构体
#[derive(Serialize)]
pub struct CreateUserResponse {
    pub message: String,
    pub system_user_created: bool,
    pub error: Option<String>,
}

// HasAdmin 响应结构体
#[derive(Serialize)]
pub struct HasAdminResponse {
    pub has_admin: bool,
}

// 检查是否存在 admin 用户
#[get("/has_admin")]
pub async fn has_admin(pool: web::Data<DbPool>) -> impl Responder {
    let pool = pool.get_ref().clone();
    
    let result: Result<bool, String> = web::block(move || {
        let mut conn = pool.get().expect("Couldn't get db connection from pool");
        
        use crate::schema::users::dsl::*;
        
        let admin_exists: bool = users
            .filter(type_.eq("admin"))
            .first::<User>(&mut conn)
            .optional()
            .map_err(|e| format!("Database error: {}", e))?
            .is_some();
        
        Ok(admin_exists)
    })
    .await
    .unwrap();
    
    match result {
        Ok(admin_exists) => HttpResponse::Ok().json(HasAdminResponse { has_admin: admin_exists }),
        Err(_e) => HttpResponse::InternalServerError().json(HasAdminResponse { has_admin: false }),
    }
}

// 创建用户
#[post("/admin_user")]
pub async fn create_admin_user(pool: web::Data<DbPool>, new_user: web::Json<NewUser>) -> impl Responder {
    // 检查是否使用了禁止的用户名
    if new_user.name == FORBIDDEN_USERNAME {
        return HttpResponse::Forbidden().json(CreateUserResponse {
            message: format!("Username '{}' is not allowed", FORBIDDEN_USERNAME),
            system_user_created: false,
            error: Some("Cannot use forbidden username".to_string()),
        });
    }

    let db_pool = pool.get_ref().clone();
    let user_data = new_user.into_inner();
    
    // 提前克隆需要的值用于系统用户创建
    let sys_username = user_data.name.clone();
    let sys_password = user_data.password.clone();
    
    let result: Result<(), (u16, String)> = web::block(move || {
        let mut conn = db_pool.get().expect("Couldn't get db connection from pool");
        
        // 检查 users 表中是否已有数据，如果有则不再插入
        let user_exists: bool = users
            .filter(type_.eq("admin"))
            .first::<User>(&mut conn)
            .optional()
            .map_err(|_| (500, "Error checking existing user".to_string()))?
            .is_some();
        
        if user_exists {
            return Err((409, "User already exists, cannot create more".to_string()));
        }
        
        // 只插入用户名和类型，不存储密码
        // type_ 固定为 admin，不接受 new_user 中的 type
        let user_insert = UserInsert {
            name: user_data.name,
            type_: "admin".to_string(),
        };
        
        diesel::insert_into(users)
            .values(&user_insert)
            .execute(&mut conn)
            .map_err(|_| (500, "Error creating user".to_string()))?;
        
        Ok(())
    })
    .await
    .unwrap();
    
    match result {
        Ok(_) => {
            // 数据库用户创建成功，创建 Linux 系统用户
            match create_system_user(&sys_username, &sys_password) {
                Ok(_) => HttpResponse::Created().json(CreateUserResponse {
                    message: "User created successfully".to_string(),
                    system_user_created: true,
                    error: None,
                }),
                Err(e) => {
                    // Linux 用户创建失败，回滚删除数据库用户
                    let rollback_pool = pool.get_ref().clone();
                    let username_to_delete = sys_username.clone();
                    let _ = web::block(move || {
                        let mut conn = rollback_pool.get().expect("Couldn't get db connection from pool");
                        diesel::delete(users.filter(name.eq(username_to_delete)))
                            .execute(&mut conn)
                    }).await;
                    HttpResponse::Created().json(CreateUserResponse {
                        message: "Database user created, but system user creation failed".to_string(),
                        system_user_created: false,
                        error: Some(e),
                    })
                }
            }
        }
        Err((409, msg)) => HttpResponse::Conflict().json(CreateUserResponse {
            message: msg,
            system_user_created: false,
            error: Some("User already exists".to_string()),
        }),
        Err((_, msg)) => HttpResponse::InternalServerError().json(CreateUserResponse {
            message: msg,
            system_user_created: false,
            error: Some("Internal server error".to_string()),
        }),
    }
}

// 创建 Linux 系统用户并设置密码
pub fn create_system_user(username: &str, user_password: &str) -> Result<(), String> {
    // 1. 创建系统用户（普通用户，不能登录，可以修改为可登录）
    let output = Command::new("useradd")
        .args(["-M", "-s", "/usr/sbin/nologin", username])
        .output();
    
    match output {
        Ok(result) => {
            if !result.status.success() {
                let stderr = String::from_utf8_lossy(&result.stderr);
                return Err(format!("Failed to create system user: {}", stderr));
            }
        }
        Err(e) => return Err(format!("Command error: {}", e)),
    }
    
    // 2. 设置用户密码
    let passwd_input = format!("{}:{}", username, user_password);
    let output = Command::new("chpasswd")
        .stdin(std::process::Stdio::piped())
        .spawn();
    
    match output {
        Ok(mut child) => {
            use std::io::Write;
            if let Some(stdin) = child.stdin.as_mut() {
                if let Err(e) = stdin.write_all(passwd_input.as_bytes()) {
                    return Err(format!("Failed to write password: {}", e));
                }
            }
            let result = child.wait_with_output();
            match result {
                Ok(output) => {
                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        return Err(format!("Failed to set password: {}", stderr));
                    }
                }
                Err(e) => return Err(format!("Failed to set password: {}", e)),
            }
        }
        Err(e) => return Err(format!("Command error: {}", e)),
    }
    
    Ok(())
}

// 新增用户请求结构体（仅 admin 可调用）
#[derive(Deserialize)]
pub struct AddUserRequest {
    pub username: String,
    pub password: String,
    pub user_type: String, // USER_TYPE_READ 或 USER_TYPE_SHARE
}

// 新增用户响应结构体
#[derive(Serialize)]
pub struct AddUserResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

/// 新增用户 API（仅 admin 可调用）
/// 
/// 创建 read 或 share 类型的用户，同时创建 Linux 系统用户
/// 密码仅存储在 Linux 系统中，数据库只保存用户名和类型
#[post("/add_user")]
pub async fn add_user(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    user_req: web::Json<AddUserRequest>,
) -> impl Responder {
    // 1. 验证 JWT token 并检查 admin 权限
    let _admin_username = match utils::verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let username = &user_req.username;
    let user_password = &user_req.password;
    let user_type = &user_req.user_type;

    // 2. 验证用户类型只能是 USER_TYPE_READ 或 USER_TYPE_SHARE
    if user_type != crate::utils::consts::USER_TYPE_READ && user_type != crate::utils::consts::USER_TYPE_SHARE {
        return HttpResponse::BadRequest().json(AddUserResponse {
            success: false,
            message: "Invalid user type".to_string(),
            error: Some("User type must be 'read' or 'share'".to_string()),
        });
    }

    // 3. 验证用户名合法性（只允许字母数字、下划线）
    if !username.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return HttpResponse::BadRequest().json(AddUserResponse {
            success: false,
            message: "Invalid username format".to_string(),
            error: Some("Username must contain only alphanumeric characters or underscores".to_string()),
        });
    }

    // 4. 检查用户名是否已存在
    let pool_clone = pool.get_ref().clone();
    let username_clone = username.clone();
    let user_type_clone = user_type.clone();
    
    let check_result: Result<bool, String> = web::block(move || {
        let mut conn = pool_clone.get().map_err(|e| format!("Database connection error: {}", e))?;
        
        let existing: Option<User> = users
            .filter(name.eq(&username_clone))
            .first::<User>(&mut conn)
            .optional()
            .map_err(|e| format!("Database query error: {}", e))?;
        
        Ok(existing.is_some())
    })
    .await
    .unwrap();

    match check_result {
        Ok(true) => {
            return HttpResponse::Conflict().json(AddUserResponse {
                success: false,
                message: "User already exists".to_string(),
                error: Some(format!("Username '{}' is already taken", username)),
            });
        }
        Ok(false) => {}
        Err(e) => {
            return HttpResponse::InternalServerError().json(AddUserResponse {
                success: false,
                message: "Database error".to_string(),
                error: Some(e),
            });
        }
    }

    // 5. 创建 Linux 系统用户并设置密码
    if let Err(e) = create_system_user(username, user_password) {
        return HttpResponse::InternalServerError().json(AddUserResponse {
            success: false,
            message: "Failed to create system user".to_string(),
            error: Some(e),
        });
    }

    // 6. 插入数据库（只存用户名和类型，不存密码）
    let pool_clone = pool.get_ref().clone();
    let username_clone = username.clone();
    
    let insert_result: Result<(), String> = web::block(move || {
        let mut conn = pool_clone.get().map_err(|e| format!("Database connection error: {}", e))?;
        
        let user_insert = UserInsert {
            name: username_clone,
            type_: user_type_clone,
        };
        
        diesel::insert_into(users)
            .values(&user_insert)
            .execute(&mut conn)
            .map_err(|e| format!("Failed to insert user: {}", e))?;
        
        Ok(())
    })
    .await
    .unwrap();

    match insert_result {
        Ok(_) => {
            HttpResponse::Created().json(AddUserResponse {
                success: true,
                message: format!("User '{}' created successfully with type '{}'", username, user_type),
                error: None,
            })
        }
        Err(e) => {
            // 数据库插入失败，但 Linux 用户已创建
            HttpResponse::InternalServerError().json(AddUserResponse {
                success: false,
                message: "Database insert failed, but system user was created".to_string(),
                error: Some(e),
            })
        }
    }
}

// 登出响应结构体
#[derive(Serialize)]
pub struct LogoutResponse {
    pub success: bool,
    pub message: String,
}

// 登出接口
#[post("/logout")]
pub async fn logout(pool: web::Data<DbPool>, req: HttpRequest) -> impl Responder {
        // 验证 JWT token
      // 验证 JWT token 并获取 claims
      let claims = match validate_token_with_db(&req, &pool).await {
          Ok(claims) => claims,
          Err(response) => return response,
      };


    let username = claims.sub;
    let pool_clone = pool.get_ref().clone();
    let username_clone = username.clone();

    // 2. 将用户的 token 清空
    let result: Result<(), String> = web::block(move || {
        let mut conn = pool_clone.get().map_err(|e| format!("Database connection error: {}", e))?;

        diesel::update(users.filter(name.eq(&username_clone)))
            .set(schema::users::token.eq(None::<String>))
            .execute(&mut conn)
            .map_err(|e| format!("Failed to clear token: {}", e))?;

        Ok(())
    })
    .await
    .unwrap();

    match result {
        Ok(_) => HttpResponse::Ok().json(LogoutResponse {
            success: true,
            message: format!("User '{}' logged out successfully", username),
        }),
        Err(e) => HttpResponse::InternalServerError().json(LogoutResponse {
            success: false,
            message: e,
        }),
    }
}

// 登录接口
#[post("/login")]
pub async fn login(pool: web::Data<DbPool>, login_req: web::Json<LoginRequest>) -> impl Responder {
    let pool = pool.get_ref().clone();
    let req_username = login_req.username.clone();
    let req_password = login_req.password.clone();
    
    let result: Result<Option<String>, String> = web::block(move || {
        let mut conn = pool.get().expect("Couldn't get db connection from pool");
        
        // 1. 查询数据库中的用户
        let user_result = users
            .filter(name.eq(&req_username))
            .first::<User>(&mut conn)
            .optional();
        
        match user_result {
            Ok(Some(user)) => {
                // 2. 检查 Linux 系统用户是否存在（id=name）
                if !utils::check_system_user(&user.name) {
                    return Ok(None); // Linux 系统用户不存在
                }
                
                // 3. 使用 Linux 系统验证密码（不再比较数据库中的密码）
                if utils::verify_password(&user.name, &req_password) {
                    // 4. 生成 JWT token
                    let now = Utc::now();
                    let token_id = Uuid::new_v4().to_string();
                    
                    let new_token = jwt::generate_token(
                        user.name.clone(),
                        now.timestamp(),
                        (now + Duration::days(crate::utils::consts::JWT_TOKEN_EXPIRE_DAYS)).timestamp(),
                        token_id.clone(),
                    )?;
                    
                    // 5. 将 token 存入数据库
                    diesel::update(users.filter(name.eq(&req_username)))
                        .set(schema::users::token.eq(&token_id))
                        .execute(&mut conn)
                        .map_err(|e| format!("Failed to update token: {}", e))?;
                    
                    Ok(Some(new_token))
                } else {
                    Ok(None)
                }
            }
            Ok(None) => Ok(None), // 数据库中不存在该用户
            Err(e) => Err(format!("Database error: {}", e)),
        }
    })
    .await
    .unwrap();
    
    match result {
        Ok(Some(token_value)) => HttpResponse::Ok()
            .json(LoginResponse {
                success: true,
                message: "Login successful".to_string(),
                token: Some(token_value),
            }),
        Ok(None) => HttpResponse::Unauthorized().json(LoginResponse {
            success: false,
            message: "Invalid username or password".to_string(),
            token: None,
        }),
        Err(msg) => HttpResponse::InternalServerError().json(LoginResponse {
            success: false,
            message: msg,
            token: None,
        }),
    }
}

#[derive(Serialize, Clone)]
pub struct UserInfo {
    pub name: String,
    pub type_: String,
    pub in_samba: bool,
}

#[derive(Serialize)]
pub struct ListUsersResponse {
    pub success: bool,
    pub users: Vec<UserInfo>,
}

// 管理员修改用户密码请求结构体
#[derive(Deserialize)]
pub struct ChangeUserPasswordRequest {
    pub user_id: String,      // 用户名（id）
    pub new_password: String, // 新密码
}

// 管理员修改用户密码响应结构体
#[derive(Serialize)]
pub struct ChangeUserPasswordResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

/// 管理员修改指定用户密码
/// 
/// 需要 admin access token，无需验证旧密码，直接修改系统用户密码
#[post("/change_passwd")]
pub async fn change_passwd(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    pwd_req: web::Json<ChangeUserPasswordRequest>,
) -> impl Responder {
    // 1. 验证 JWT token 并检查 admin 权限
    let _admin_username = match utils::verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let target_user = &pwd_req.user_id;
    let new_password = &pwd_req.new_password;

    // 2. 验证用户名非空
    if target_user.is_empty() {
        return HttpResponse::BadRequest().json(ChangeUserPasswordResponse {
            success: false,
            message: "User ID cannot be empty".to_string(),
            error: Some("User ID is required".to_string()),
        });
    }

    // 3. 验证新密码非空
    if new_password.is_empty() {
        return HttpResponse::BadRequest().json(ChangeUserPasswordResponse {
            success: false,
            message: "New password cannot be empty".to_string(),
            error: Some("New password is required".to_string()),
        });
    }

    // 4. 检查目标用户是否存在于数据库
    let pool_clone = pool.get_ref().clone();
    let target_user_clone = target_user.clone();
    
    let user_exists: Result<bool, String> = web::block(move || {
        let mut conn = pool_clone.get().map_err(|e| format!("Database connection error: {}", e))?;
        
        let existing: Option<User> = users
            .filter(name.eq(&target_user_clone))
            .first::<User>(&mut conn)
            .optional()
            .map_err(|e| format!("Database query error: {}", e))?;
        
        Ok(existing.is_some())
    })
    .await
    .unwrap();

    match user_exists {
        Ok(false) => {
            return HttpResponse::NotFound().json(ChangeUserPasswordResponse {
                success: false,
                message: "User not found".to_string(),
                error: Some(format!("User '{}' does not exist in database", target_user)),
            });
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(ChangeUserPasswordResponse {
                success: false,
                message: "Database error".to_string(),
                error: Some(e),
            });
        }
        Ok(true) => {}
    }

    // 5. 检查系统用户是否存在
    if !utils::check_system_user(target_user) {
        return HttpResponse::NotFound().json(ChangeUserPasswordResponse {
            success: false,
            message: "System user not found".to_string(),
            error: Some(format!("User '{}' does not exist in Linux system", target_user)),
        });
    }

    // 6. 修改系统用户密码
    match change_system_password(target_user, new_password) {
        Ok(_) => HttpResponse::Ok().json(ChangeUserPasswordResponse {
            success: true,
            message: format!("Password for user '{}' changed successfully", target_user),
            error: None,
        }),
        Err(e) => HttpResponse::InternalServerError().json(ChangeUserPasswordResponse {
            success: false,
            message: "Failed to change password".to_string(),
            error: Some(e),
        }),
    }
}

fn get_samba_users() -> Vec<String> {
    let output = Command::new("pdbedit")
        .args(["-L"])
        .output();

    match output {
        Ok(result) => {
            if result.status.success() {
                let stdout = String::from_utf8_lossy(&result.stdout);
                stdout
                    .lines()
                    .filter_map(|line| line.split(':').next().map(|s| s.to_string()))
                    .collect()
            } else {
                vec![]
            }
        }
        Err(_) => vec![],
    }
}

// 修改密码请求结构体
#[derive(Deserialize)]
pub struct ChangePasswordRequest {
    pub old_password: String,
    pub new_password: String,
}

// 修改密码响应结构体
#[derive(Serialize)]
pub struct ChangePasswordResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

/// 修改当前 admin 用户密码
/// 
/// 需要 admin access token，验证旧密码后修改系统用户密码
#[post("/change_admin_passwd")]
pub async fn change_admin_passwd(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    pwd_req: web::Json<ChangePasswordRequest>,
) -> impl Responder {
    // 1. 验证 JWT token 并检查 admin 权限
    let admin_username = match utils::verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let old_password = &pwd_req.old_password;
    let new_password = &pwd_req.new_password;

    // 2. 验证旧密码是否正确
    if !utils::verify_password(&admin_username, old_password) {
        return HttpResponse::InternalServerError().json(ChangePasswordResponse {
            success: false,
            message: "Invalid old password".to_string(),
            error: Some("Old password verification failed".to_string()),
        });
    }

    // 3. 修改系统用户密码
    match change_system_password(&admin_username, new_password) {
        Ok(_) => HttpResponse::Ok().json(ChangePasswordResponse {
            success: true,
            message: "Password changed successfully".to_string(),
            error: None,
        }),
        Err(e) => HttpResponse::InternalServerError().json(ChangePasswordResponse {
            success: false,
            message: "Failed to change password".to_string(),
            error: Some(e),
        }),
    }
}

/// 修改 Linux 系统用户密码
fn change_system_password(username: &str, new_password: &str) -> Result<(), String> {
    let passwd_input = format!("{}:{}", username, new_password);
    let output = Command::new("chpasswd")
        .stdin(std::process::Stdio::piped())
        .spawn();

    match output {
        Ok(mut child) => {
            use std::io::Write;
            if let Some(stdin) = child.stdin.as_mut() {
                if let Err(e) = stdin.write_all(passwd_input.as_bytes()) {
                    return Err(format!("Failed to write password: {}", e));
                }
            }
            let result = child.wait_with_output();
            match result {
                Ok(output) => {
                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        return Err(format!("Failed to change password: {}", stderr));
                    }
                }
                Err(e) => return Err(format!("Failed to change password: {}", e)),
            }
        }
        Err(e) => return Err(format!("Command error: {}", e)),
    }

    Ok(())
}

// 删除用户请求结构体
#[derive(Deserialize)]
pub struct DeleteUserRequest {
    pub username: String,
}

// 删除用户响应结构体
#[derive(Serialize)]
pub struct DeleteUserResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

/// 删除 Linux 系统用户
fn delete_system_user(username: &str) -> Result<(), String> {
    let output = Command::new("userdel")
        .args(["-r", username])  // -r 选项会同时删除用户主目录
        .output();

    match output {
        Ok(result) => {
            if result.status.success() {
                Ok(())
            } else {
                let stderr = String::from_utf8_lossy(&result.stderr);
                Err(format!("Failed to delete system user: {}", stderr))
            }
        }
        Err(e) => Err(format!("Command error: {}", e)),
    }
}

/// 删除用户 API（仅 admin 可调用）
/// 
/// 传入用户名，如果不为 admin 类型，则在 Linux 系统和数据库中删除该用户
#[post("/delete_user")]
pub async fn delete_user(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    del_req: web::Json<DeleteUserRequest>,
) -> impl Responder {
    // 1. 验证 JWT token 并检查 admin 权限
    let _admin_username = match utils::verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let target_username = &del_req.username;

    // 2. 验证用户名非空
    if target_username.is_empty() {
        return HttpResponse::BadRequest().json(DeleteUserResponse {
            success: false,
            message: "Username cannot be empty".to_string(),
            error: Some("Username is required".to_string()),
        });
    }

    // 3. 禁止删除 root 用户
    if target_username == crate::utils::consts::FORBIDDEN_USERNAME {
        return HttpResponse::Forbidden().json(DeleteUserResponse {
            success: false,
            message: format!("Cannot delete user '{}'", target_username),
            error: Some("Forbidden username".to_string()),
        });
    }

    // 4. 查询数据库获取用户信息和类型
    let pool_clone = pool.get_ref().clone();
    let target_user_clone = target_username.clone();

    let user_query: Result<Option<User>, String> = web::block(move || {
        let mut conn = pool_clone.get().map_err(|e| format!("Database connection error: {}", e))?;

        let existing: Option<User> = users
            .filter(name.eq(&target_user_clone))
            .first::<User>(&mut conn)
            .optional()
            .map_err(|e| format!("Database query error: {}", e))?;

        Ok(existing)
    })
    .await
    .unwrap();

    let user = match user_query {
        Ok(Some(u)) => u,
        Ok(None) => {
            return HttpResponse::NotFound().json(DeleteUserResponse {
                success: false,
                message: "User not found".to_string(),
                error: Some(format!("User '{}' does not exist in database", target_username)),
            });
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(DeleteUserResponse {
                success: false,
                message: "Database error".to_string(),
                error: Some(e),
            });
        }
    };

    // 5. 检查用户类型是否为 admin，admin 用户不能删除
    if user.type_ == crate::utils::consts::USER_TYPE_ADMIN {
        return HttpResponse::Forbidden().json(DeleteUserResponse {
            success: false,
            message: "Cannot delete admin user".to_string(),
            error: Some(format!("User '{}' is an admin and cannot be deleted", target_username)),
        });
    }

    // 6. 删除 Linux 系统用户
    if let Err(e) = delete_system_user(target_username) {
        return HttpResponse::InternalServerError().json(DeleteUserResponse {
            success: false,
            message: "Failed to delete system user".to_string(),
            error: Some(e),
        });
    }

    // 7. 从数据库中删除用户
    let pool_clone = pool.get_ref().clone();
    let target_user_clone = target_username.clone();

    let delete_result: Result<(), String> = web::block(move || {
        let mut conn = pool_clone.get().map_err(|e| format!("Database connection error: {}", e))?;

        diesel::delete(users.filter(name.eq(&target_user_clone)))
            .execute(&mut conn)
            .map_err(|e| format!("Failed to delete user from database: {}", e))?;

        Ok(())
    })
    .await
    .unwrap();

    match delete_result {
        Ok(_) => HttpResponse::Ok().json(DeleteUserResponse {
            success: true,
            message: format!("User '{}' deleted successfully", target_username),
            error: None,
        }),
        Err(e) => HttpResponse::InternalServerError().json(DeleteUserResponse {
            success: false,
            message: "System user deleted, but database delete failed".to_string(),
            error: Some(e),
        }),
    }
}

#[get("/list_users")]
pub async fn list_users(req: HttpRequest, pool: web::Data<DbPool>) -> impl Responder {
        // 验证 JWT token
    if let Err(response) = validate_token_with_db(&req, &pool).await {
        return response;
    }
    let pool = pool.get_ref().clone();

    let result: Result<(Vec<(String, String)>, Vec<String>), String> = web::block(move || {
        let mut conn = pool.get().map_err(|e| format!("Database connection error: {}", e))?;

        let db_users: Vec<(String, String)> = users
            .select((name, type_))
            .load::<(String, String)>(&mut conn)
            .map_err(|e| format!("Database query error: {}", e))?;

        let samba_users = get_samba_users();

        Ok((db_users, samba_users))
    })
    .await
    .unwrap();

    match result {
        Ok((db_users, samba_users)) => {
            let mut user_list: Vec<UserInfo> = Vec::new();

            // 记录已经在列表中的用户名，用于去重
            let mut processed_names = std::collections::HashSet::new();

            // 1. 处理数据库中的用户
            for (user_name, user_type) in db_users {
                let is_in_samba = samba_users.contains(&user_name);
                processed_names.insert(user_name.clone());
                
                user_list.push(UserInfo {
                    name: user_name,
                    type_: user_type,
                    in_samba: is_in_samba,
                });
            }

            // 2. 处理仅在 Samba 中存在的用户（串联逻辑）
            for s_user in samba_users {
                if !processed_names.contains(&s_user) {
                    user_list.push(UserInfo {
                        name: s_user,
                        type_: "samba".to_string(), // 标记类型为 samba
                        in_samba: true,
                    });
                }
            }            

            HttpResponse::Ok().json(ListUsersResponse {
                success: true,
                users: user_list,
            })
        }
        Err(_e) => HttpResponse::InternalServerError().json(ListUsersResponse {
            success: false,
            users: vec![],
        }),
    }
}
