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
    
    // 检查外部依赖命令
    match utils::check_external_commands() {
        Ok(()) => {
            println!("All external commands are available");
        }
        Err(missing) => {
            eprintln!("\x1b[31mWarning: The following external commands are missing:\x1b[0m");
            for cmd in &missing {
                eprintln!("  - {}", cmd);
            }
        }
    }
    
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
                    println!("Startup check passed: user 'xxx' exists in Linux system");
                } else {
                    eprintln!("\x1b[31mStartup check warning: user 'xxx' does NOT exist in Linux system\x1b[0m");
                }
            }
            Ok(None) => {
                println!("Startup check: no users found in database");
                println!("You can create a user with:");
                // ANSI 红色: \x1b[31m, 重置: \x1b[0m
                println!("\x1b[31mcurl -k -X POST https://192.168.3.206:8443/admin_user \\\x1b[0m");
                println!("\x1b[31m    -H \"Content-Type: application/json\" \\\x1b[0m");
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
            .service(apis::chkpoint::get_chkpoint)
            .service(apis::chkpoint::create_chkpoint)
            .service(apis::chkpoint::remove_chkpoint)
            .service(apis::chkpoint::rollback_chkpoint)
            .service(apis::users::has_admin)
            .service(apis::users::create_admin_user)
            .service(apis::users::add_user)
            .service(apis::users::list_users)
            .service(apis::users::login)
            .service(apis::users::logout)
            .service(apis::users::change_admin_passwd)
            .service(apis::users::change_passwd)
            .service(apis::users::delete_user)            
            .service(apis::disks::get_disks)
            .service(apis::disks::get_free_disks)
            .service(apis::disks::get_free_parts)
            .service(apis::disks::format_disk)
            .service(apis::disks::part_disk)
            .service(apis::disks::create_pool)
            .service(apis::disks::destroy_pool)
            .service(apis::disks::label_clear)
            .service(apis::samba::smb_public_share)
            .service(apis::samba::smb_auth_share)
            .service(apis::samba::smb_add_user)
            .service(apis::samba::smb_delete_user)
            .service(apis::samba::smb_list_users)
            .service(apis::samba::smb_change_passwd)
            .service(apis::samba::list_pools)
            .service(apis::samba::create_zfs_share)
            .service(apis::samba::list_zfs_shares)
            .service(apis::samba::list_dir_shares)
            .service(apis::samba::remove_dir_share)
            .service(apis::samba::remove_zfs_share)
            .service(apis::docker::get_images)
            .service(apis::docker::delete_image)
            .service(apis::docker::start_pull_image)
            .service(apis::docker::get_pull_task)
            .service(apis::docker::list_pull_tasks)
            .service(apis::docker::create_container)
            .service(apis::docker::set_registry)
            .service(apis::docker::get_registries)
            .service(apis::docker::delete_registry)
            .service(apis::docker::add_registry_mirror)
            .service(apis::docker::list_registry_mirrors)
            .service(apis::docker::delete_registry_mirror_entry)
            .service(apis::docker::set_volume)
            .service(apis::docker::get_volumes)
            .service(apis::docker::delete_volume)
            .service(apis::docker_compose::compose_up)
            .service(apis::docker_compose::compose_list)
            .service(apis::docker_compose::compose_down)
            .service(apis::docker_compose::compose_delete)
            .service(apis::docker_compose::pod_list)
            .service(apis::docker_compose::pod_start)
            .service(apis::docker_compose::pod_stop)
            .service(apis::docker_compose::pod_restart)
            .service(apis::docker_compose::pod_remove)
            .service(apis::container::get_containers)
            .service(apis::container::start_container)
            .service(apis::container::stop_container)
            .service(apis::container::restart_container)
            .service(apis::container::remove_container)
            .service(apis::zfs::get_bootfs)
            .service(apis::zfs::set_bootfs)
            .service(apis::zfs::clone_dataset)
            .service(apis::zfs::promote_dataset)
            .service(apis::zfs::get_datasets)
            .service(apis::zfs::destroy_dataset)
            .service(apis::zfs::get_dataset_depends)
            .service(apis::zfs::online_pools)
            .service(apis::zfs::offline_pools)
            .service(apis::zfs::import_pool)
            .service(apis::zfs::export_pool)
            .service(apis::zfs::reboot_system)
            .service(apis::zfs::reboot_system_force)
            .service(apis::zfs::shutdown_system)
            .service(apis::zfs::get_pool_advanced_setting)
            .service(apis::zfs::set_pool_advanced_setting)
            .service(apis::zfs::get_pool_devices_handler)
    })
    .bind_openssl(&server_address, builder)?
    .run()
    .await
}
