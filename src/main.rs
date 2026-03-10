use actix_web::{get, post, web, App, HttpResponse, HttpServer, Responder};
use diesel::prelude::*;
use diesel::r2d2::{self, ConnectionManager};
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
use serde::{Deserialize, Serialize};
use std::process::Command;

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

// 获取所有用户
#[get("/users")]
async fn get_users(pool: web::Data<DbPool>) -> impl Responder {
    let mut conn = pool.get().expect("Couldn't get db connection from pool");
    
    let result = web::block(move || {
        users.load::<User>(&mut conn)
    })
    .await
    .unwrap();
    
    match result {
        Ok(user_list) => HttpResponse::Ok().json(user_list),
        Err(_) => HttpResponse::InternalServerError().body("Error loading users"),
    }
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
        
        // 如果要创建的是 admin 用户，先检查是否已存在
        if user_data.type_ == "admin" {
            let admin_exists: bool = users
                .filter(type_.eq("admin"))
                .first::<User>(&mut conn)
                .optional()
                .map_err(|_| (500, "Error checking admin user".to_string()))?
                .is_some();
            
            if admin_exists {
                return Err((409, "Admin user already exists".to_string()));
            }
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

// 验证 Linux 系统用户密码
fn verify_system_password(username: &str, user_password: &str) -> bool {
    // 使用 su 命令验证密码
    // su -c "echo success" username
    // 如果密码正确，命令会成功执行
    let mut child = match Command::new("su")
        .arg("-c")
        .arg("echo authenticated")
        .arg(username)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn() {
        Ok(child) => child,
        Err(_) => return false,
    };
    
    // 写入密码
    if let Some(stdin) = child.stdin.as_mut() {
        use std::io::Write;
        if stdin.write_all(format!("{}\n", user_password).as_bytes()).is_err() {
            return false;
        }
    }
    
    // 等待命令执行完成
    match child.wait_with_output() {
        Ok(output) => {
            output.status.success() && 
            String::from_utf8_lossy(&output.stdout).contains("authenticated")
        }
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
    
    let result: Result<bool, String> = web::block(move || {
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
                    return Ok(false); // Linux 系统用户不存在
                }
                
                // 3. 使用 Linux 系统验证密码（不再比较数据库中的密码）
                if verify_system_password(&user.name, &req_password) {
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            Ok(None) => Ok(false), // 数据库中不存在该用户
            Err(e) => Err(format!("Database error: {}", e)),
        }
    })
    .await
    .unwrap();
    
    match result {
        Ok(true) => HttpResponse::Ok().json(LoginResponse {
            success: true,
            message: "Login successful".to_string(),
        }),
        Ok(false) => HttpResponse::Unauthorized().json(LoginResponse {
            success: false,
            message: "Invalid username or password".to_string(),
        }),
        Err(msg) => HttpResponse::InternalServerError().json(LoginResponse {
            success: false,
            message: msg,
        }),
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // 加载 .env 文件
    dotenvy::dotenv().ok();
    
    // 设置数据库连接池
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let manager = ConnectionManager::<SqliteConnection>::new(&database_url);
    let pool = r2d2::Pool::builder()
        .build(manager)
        .expect("Failed to create pool");

    // 加载 TLS 证书
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    builder
        .set_private_key_file("certs/key.pem", SslFiletype::PEM)
        .unwrap();
    builder
        .set_certificate_chain_file("certs/cert.pem")
        .unwrap();

    println!("HTTPS Server running at https://127.0.0.1:8443");
    println!("Try: curl -k https://localhost:8443/ping");
    println!("Database connected: {}", database_url);

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .service(index)
            .service(ping)
            .service(get_users)
            .service(create_user)
            .service(login)
    })
    .bind_openssl("127.0.0.1:8443", builder)?
    .run()
    .await
}
