use actix_web::{post, web, HttpRequest, HttpResponse, Responder};
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

// 创建用户
#[post("/admin_user")]
pub async fn create_admin_user(pool: web::Data<DbPool>, new_user: web::Json<NewUser>) -> impl Responder {
    let pool = pool.get_ref().clone();
    let user_data = new_user.into_inner();
    
    // 提前克隆需要的值用于系统用户创建
    let sys_username = user_data.name.clone();
    let sys_password = user_data.password.clone();
    
    let result: Result<(), (u16, String)> = web::block(move || {
        let mut conn = pool.get().expect("Couldn't get db connection from pool");
        
        // 检查 users 表中是否已有数据，如果有则不再插入
        let user_exists: bool = users
            .first::<User>(&mut conn)
            .optional()
            .map_err(|_| (500, "Error checking existing user".to_string()))?
            .is_some();
        
        if user_exists {
            return Err((409, "User already exists, cannot create more".to_string()));
        }
        
        // 只插入用户名和类型，不存储密码
        let user_insert = UserInsert {
            name: user_data.name,
            type_: user_data.type_,
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
                    // Linux 用户创建失败，但数据库用户已创建
                    // 可以在这里添加回滚逻辑
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
        .args(["-m", "-s", "/bin/bash", username])
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
    pub user_type: String, // "read" 或 "share"
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
    let _admin_username = match utils::verify_admin_access(&req, &pool) {
        Ok(username) => username,
        Err(response) => return response,
    };

    let username = &user_req.username;
    let user_password = &user_req.password;
    let user_type = &user_req.user_type;

    // 2. 验证用户类型只能是 read 或 share
    if user_type != "read" && user_type != "share" {
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
                        (now + Duration::days(30)).timestamp(),
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
