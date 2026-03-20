use actix_web::{get, web, App, HttpResponse, HttpServer, Responder};
use diesel::prelude::*;
use diesel::r2d2::{self, ConnectionManager};
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
use serde::Serialize;
use dotenvy::{dotenv, from_path};
use std::env;
use std::path::Path;
use std::fs;

mod apis;
mod config;
mod disk;
mod jwt;
mod models;
mod schema;
mod utils;

use config::logger;

use models::User;
use schema::users::dsl::*;

// 数据库连接池类型
pub type DbPool = r2d2::Pool<ConnectionManager<SqliteConnection>>;

#[derive(Serialize)]
struct PingResponse {
    status: String,
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

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // 初始化日志
    logger::init_logger();
    
    let env_path = Path::new("/etc/zuti/.env");
    if env_path.exists() {
        log::info!("Loading env from: {}", env_path.display());
        from_path(env_path).ok();
    } else {
        log::info!("Env file not found at: {}, falling back to default .env", env_path.display());
        // 方式2：回退到默认的 .env
        dotenv().ok();
    }

    
    // 设置数据库连接池
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    log::info!("Database URL: {}", database_url);
    
    // 读取服务器地址
    let server_address = std::env::var("SERVER_ADDRESS").unwrap_or_else(|_| "127.0.0.1:8443".to_string());
    let manager = ConnectionManager::<SqliteConnection>::new(&database_url);
    let pool = r2d2::Pool::builder()
        .build(manager)
        .expect("Failed to create pool");

    // 启动检查：检查 users 表第一个用户是否存在于 Linux 系统
    {
        let mut conn = pool.get().expect("Couldn't get db connection from pool");
        
        match users.first::<User>(&mut conn).optional() {
            Ok(Some(first_user)) => {
                if utils::check_system_user(&first_user.name) {
                    println!("Startup check passed: user '{}' exists in Linux system", first_user.name);
                } else {
                    eprintln!("\x1b[31mStartup check warning: user '{}' does NOT exist in Linux system\x1b[0m", first_user.name);
                }
            }
            Ok(None) => {
                println!("Startup check: no users found in database");
                println!("You can create a user with:");
                // ANSI 红色: \x1b[31m, 重置: \x1b[0m
                println!("\x1b[31mcurl -k -X POST https://192.168.3.248:8443/users \\x1b[0m");
                println!("\x1b[31m    -H \"Content-Type: application/json\" \\x1b[0m");
                println!("\x1b[31m    -d '{{\"name\": \"myadmin\", \"type_\": \"admin\", \"password\":\"123321\"}}'\x1b[0m");
            }
            Err(e) => {
                eprintln!("Startup check error: failed to query database - {}", e);
            }
        }
    }

    // 加载 TLS 证书
    let cert_file = std::env::var("CERT_FILE").unwrap_or_else(|_| "certs/cert.pem".to_string());
    let key_file = std::env::var("KEY_FILE").unwrap_or_else(|_| "certs/key.pem".to_string());
    
    log::info!("Certificate file: {}", cert_file);
    log::info!("Key file: {}", key_file);
    
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    builder
        .set_private_key_file(&key_file, SslFiletype::PEM)
        .unwrap();
    builder
        .set_certificate_chain_file(&cert_file)
        .unwrap();

    log::info!("HTTPS Server running at https://{}", server_address);
    println!("HTTPS Server running at https://{}", server_address);
    println!("Try: curl -k https://{}/ping", server_address);
    println!("Database connected: {}", database_url);

    // 检查是否存在 smb.conf 备份文件，如果没有则创建
    let smb_conf_path = Path::new("/etc/samba/smb.conf");
    let smb_conf_bak_path = Path::new("/etc/samba/smb.conf.bak");
    if !smb_conf_bak_path.exists() {
        if let Err(e) = std::fs::copy(smb_conf_path, smb_conf_bak_path) {
            log::warn!("Failed to backup smb.conf: {}", e);
        } else {
            log::info!("Created backup: /etc/samba/smb.conf.bak");
        }
    }

        // 6. 创建 /etc/samba/conf.d 目录
    let conf_dir = Path::new("/etc/samba/conf.d");
    if !conf_dir.exists() {
        match fs::create_dir_all(conf_dir) {
            Ok(_) => {}
            Err(e) => {
               log::info!("Failed to create /etc/samba/conf.d directory")               
            }
        }
    }
    // 7. 检查主配置文件是否包含 conf.d 目录引用
    if let Err(e) = crate::utils::ensure_global_include(smb_conf_path, "/etc/samba/conf.d/all-share.conf") {
        log::warn!("Failed to update smb.conf: {}", e);
    } else {
        log::info!("Keep smb.conf global add include /etc/samba/conf.d/all-share.conf");
    }

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .service(index)
            .service(ping)
            .service(apis::users::create_admin_user)
            .service(apis::users::add_user)
            .service(apis::users::login)
            .service(apis::users::logout)
            .service(apis::disks::get_disks)
            .service(apis::disks::get_free_disks)
            .service(apis::disks::get_free_parts)
            .service(apis::disks::format_disk)
            .service(apis::disks::part_disk)
            .service(apis::disks::create_pool)
            .service(apis::disks::destroy_pool)
            .service(apis::samba::smb_public_share)
            .service(apis::samba::smb_auth_share)
            .service(apis::samba::smb_add_user)
            .service(apis::samba::smb_delete_user)
            .service(apis::samba::smb_list_users)
            .service(apis::samba::list_zfs_pools)
            .service(apis::samba::create_zfs_share)
            .service(apis::samba::list_zfs_shares)
            .service(apis::samba::list_dir_shares)
            .service(apis::samba::delete_dir_share)
    })
    .bind_openssl(&server_address, builder)?
    .run()
    .await
}
