use actix_web::{get, web, App, HttpResponse, HttpServer, Responder};
use diesel::prelude::*;
use diesel::r2d2::{self, ConnectionManager};
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
use serde::Serialize;

mod apis;
mod disk;
mod jwt;
mod models;
mod schema;
mod utils;

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
            .service(apis::users::create_admin_user)
            .service(apis::users::add_user)
            .service(apis::users::login)
            .service(apis::disks::get_disks)
            .service(apis::disks::get_free_disks)
            .service(apis::disks::get_free_parts)
            .service(apis::disks::format_disk)
            .service(apis::disks::part_disk)
            .service(apis::disks::create_pool)
            .service(apis::disks::destroy_pool)
    })
    .bind_openssl(&server_address, builder)?
    .run()
    .await
}
