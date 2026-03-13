use actix_web::{get, post, web, App, HttpResponse, HttpServer, Responder};
use chrono::{Duration, Utc};
use diesel::prelude::*;
use diesel::r2d2::{self, ConnectionManager};
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
use serde::{Deserialize, Serialize};
use std::process::Command;
use uuid::Uuid;

mod apis;
mod disk;
mod jwt;
mod models;
mod schema;

use models::{NewUser, User, UserInsert};
use schema::users::dsl::*;

// 数据库连接池类型
type DbPool = r2d2::Pool<ConnectionManager<SqliteConnection>>;

#[derive(Serialize)]
struct PingResponse {
    status: String,
    message: String,
}

// 登录请求结构体
#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

// 登录响应结构体
#[derive(Serialize)]
struct LoginResponse {
    success: bool,
    message: String,
    token: Option<String>,
}

#[get("/ping")]
async fn ping() -> impl Responder {
    HttpResponse::Ok().json(PingResponse {
        status: "ok".to_string(),
        message: "pong".to_string(),
    })
}

#[get("/")]
async fn index() -> impl Responder {
    HttpResponse::Ok().body("HTTPS Server is running!")
}

// 创建用户
#[post("/users")]
async fn create_user(pool: web::Data<DbPool>, new_user: web::Json<NewUser>) -> impl Responder {
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
                Ok(_) => HttpResponse::Created().json(serde_json::json!({
                    "message": "User created successfully",
                    "system_user_created": true
                })),
                Err(e) => {
                    // Linux 用户创建失败，但数据库用户已创建
                    // 可以在这里添加回滚逻辑
                    HttpResponse::Created().json(serde_json::json!({
                        "message": "Database user created, but system user creation failed",
                        "system_user_created": false,
                        "error": e
                    }))
                }
            }
        }
        Err((409, msg)) => HttpResponse::Conflict().body(msg),
        Err((_, msg)) => HttpResponse::InternalServerError().body(msg),
    }
}

// 验证 Linux 系统用户是否存在（id=name）
fn check_system_user(username: &str) -> bool {
    let output = Command::new("id")
        .arg(username)
        .output();
    
    match output {
        Ok(result) => result.status.success(),
        Err(_) => false,
    }
}

// 验证 Linux 系统用户密码（使用 PAM）
fn verify_system_password(username: &str, user_password: &str) -> bool {
    use pam::Client;

    // 创建 PAM 客户端，使用系统默认的认证服务
    let mut client = match Client::with_password("system-auth") {
        Ok(client) => client,
        Err(_) => {
            // 如果 system-auth 不可用，尝试其他常见服务名
            match Client::with_password("login") {
                Ok(client) => client,
                Err(_) => return false,
            }
        }
    };

    // 设置用户名和密码
    client.conversation_mut().set_credentials(username, user_password);

    // 执行认证
    match client.authenticate() {
        Ok(_) => true,
        Err(_) => false,
    }
}

// 创建 Linux 系统用户并设置密码
fn create_system_user(username: &str, user_password: &str) -> Result<(), String> {
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
async fn login(pool: web::Data<DbPool>, login_req: web::Json<LoginRequest>) -> impl Responder {
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
                if !check_system_user(&user.name) {
                    return Ok(None); // Linux 系统用户不存在
                }
                
                // 3. 使用 Linux 系统验证密码（不再比较数据库中的密码）
                if verify_system_password(&user.name, &req_password) {
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

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // 加载 .env 文件
    dotenvy::dotenv().ok();
    
    // 设置数据库连接池
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    
    // 读取服务器地址
    let server_address = std::env::var("SERVER_ADDRESS").unwrap_or_else(|_| "127.0.0.1:8443".to_string());
    let manager = ConnectionManager::<SqliteConnection>::new(&database_url);
    let pool = r2d2::Pool::builder()
        .build(manager)
        .expect("Failed to create pool");

    // 启动检查：检查 users 表第一个用户是否存在于 Linux 系统
    {
        use models::User;
        use schema::users::dsl::*;
        
        let mut conn = pool.get().expect("Couldn't get db connection from pool");
        
        match users.first::<User>(&mut conn).optional() {
            Ok(Some(first_user)) => {
                if check_system_user(&first_user.name) {
                    println!("Startup check passed: user '{}' exists in Linux system", first_user.name);
                } else {
                    eprintln!("\x1b[31mStartup check warning: user '{}' does NOT exist in Linux system\x1b[0m", first_user.name);
                }
            }
            Ok(None) => {
                println!("Startup check: no users found in database");
                println!("You can create a user with:");
                // ANSI 红色: \x1b[31m, 重置: \x1b[0m
                println!("\x1b[31mcurl -k -X POST https://192.168.3.248:8443/users \\\x1b[0m");
                println!("\x1b[31m    -H \"Content-Type: application/json\" \\\x1b[0m");
                println!("\x1b[31m    -d '{{\"name\": \"myadmin\", \"type_\": \"admin\", \"password\":\"123321\"}}'\x1b[0m");
            }
            Err(e) => {
                eprintln!("Startup check error: failed to query database - {}", e);
            }
        }
    }

    // 加载 TLS 证书
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    builder
        .set_private_key_file("certs/key.pem", SslFiletype::PEM)
        .unwrap();
    builder
        .set_certificate_chain_file("certs/cert.pem")
        .unwrap();

    println!("HTTPS Server running at https://{}", server_address);
    println!("Try: curl -k https://{}/ping", server_address);
    println!("Database connected: {}", database_url);

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .service(index)
            .service(ping)
            .service(create_user)
            .service(login)
            .service(apis::disks::get_disks)
            .service(apis::disks::get_free_disks)
    })
    .bind_openssl(&server_address, builder)?
    .run()
    .await
}
