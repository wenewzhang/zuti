use actix_web::{post, web, HttpResponse, Responder};
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
#[post("/users")]
pub async fn create_user(pool: web::Data<DbPool>, new_user: web::Json<NewUser>) -> impl Responder {
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
