use actix_web::{get, post, web, App, HttpResponse, HttpServer, Responder};
use diesel::prelude::*;
use diesel::r2d2::{self, ConnectionManager};
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
use serde::Serialize;

mod models;
mod schema;

use models::{NewUser, User};
use schema::users::dsl::*;

// 数据库连接池类型
type DbPool = r2d2::Pool<ConnectionManager<SqliteConnection>>;

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
        
        diesel::insert_into(users)
            .values(&user_data)
            .execute(&mut conn)
            .map_err(|_| (500, "Error creating user".to_string()))?;
        
        Ok(())
    })
    .await
    .unwrap();
    
    match result {
        Ok(_) => HttpResponse::Created().body("User created"),
        Err((409, msg)) => HttpResponse::Conflict().body(msg),
        Err((_, msg)) => HttpResponse::InternalServerError().body(msg),
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
    })
    .bind_openssl("127.0.0.1:8443", builder)?
    .run()
    .await
}
