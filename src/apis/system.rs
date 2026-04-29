use actix_web::{get, post, HttpRequest, HttpResponse, Responder, web};
use lazy_static::lazy_static;
use serde::Serialize;
use std::process::Command;
use tokio::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::utils::admin::verify_admin_access;

const RECOMMENDED_APPS_CACHE_TTL_SECONDS: u64 = 3600;

struct RecommendedAppsCache {
    data: Option<serde_json::Value>,
    cached_at: Option<u64>,
    path: Option<String>,
}

lazy_static! {
    static ref RECOMMENDED_APPS_CACHE: Mutex<RecommendedAppsCache> = Mutex::new(RecommendedAppsCache {
        data: None,
        cached_at: None,
        path: None,
    });
}

// reboot 响应结构体
#[derive(Serialize)]
pub struct RebootResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

/// system/reboot_force API - 强制重启系统（需要 JWT 认证，仅管理员可用）
#[post("/system/reboot_force")]
pub async fn reboot_system_force(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    // 1. 验证 JWT token 并检查 admin 权限
    let _username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    // 2. 执行 /sbin/reboot -f 命令
    let output = match Command::new("/sbin/reboot")
        .arg("-f")
        .output()
    {
        Ok(result) => result,
        Err(e) => {
            return HttpResponse::InternalServerError().json(RebootResponse {
                success: false,
                message: "Failed to execute reboot -f command".to_string(),
                error: Some(format!("Command error: {}", e)),
            });
        }
    };

    if output.status.success() {
        HttpResponse::Ok().json(RebootResponse {
            success: true,
            message: "System is force rebooting...".to_string(),
            error: None,
        })
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        HttpResponse::InternalServerError().json(RebootResponse {
            success: false,
            message: "Failed to force reboot system".to_string(),
            error: Some(stderr.to_string()),
        })
    }
}

/// system/reboot API - 重启系统（需要 JWT 认证，仅管理员可用）
#[post("/system/reboot")]
pub async fn reboot_system(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    // 1. 验证 JWT token
    let _claims = match crate::utils::admin::validate_token_with_db(&req, &pool).await {
        Ok(claims) => claims,
        Err(response) => return response,
    };

    // 2. 执行 systemctl reboot 命令
    let output = match Command::new("systemctl")
        .arg("reboot")
        .output()
    {
        Ok(result) => result,
        Err(e) => {
            return HttpResponse::InternalServerError().json(RebootResponse {
                success: false,
                message: "Failed to execute reboot command".to_string(),
                error: Some(format!("Command error: {}", e)),
            });
        }
    };

    if output.status.success() {
        HttpResponse::Ok().json(RebootResponse {
            success: true,
            message: "System is rebooting...".to_string(),
            error: None,
        })
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        HttpResponse::InternalServerError().json(RebootResponse {
            success: false,
            message: "Failed to reboot system".to_string(),
            error: Some(stderr.to_string()),
        })
    }
}

// shutdown 响应结构体
#[derive(Serialize)]
pub struct ShutdownResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

/// system/shutdown API - 关闭系统（需要 JWT 认证，仅管理员可用）
#[post("/system/shutdown")]
pub async fn shutdown_system(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    // 1. 验证 JWT token
    let _claims = match crate::utils::admin::validate_token_with_db(&req, &pool).await {
        Ok(claims) => claims,
        Err(response) => return response,
    };

    // 2. 执行 systemctl poweroff 命令
    let output = match Command::new("systemctl")
        .arg("poweroff")
        .output()
    {
        Ok(result) => result,
        Err(e) => {
            return HttpResponse::InternalServerError().json(ShutdownResponse {
                success: false,
                message: "Failed to execute shutdown command".to_string(),
                error: Some(format!("Command error: {}", e)),
            });
        }
    };

    if output.status.success() {
        HttpResponse::Ok().json(ShutdownResponse {
            success: true,
            message: "System is shutting down...".to_string(),
            error: None,
        })
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        HttpResponse::InternalServerError().json(ShutdownResponse {
            success: false,
            message: "Failed to shutdown system".to_string(),
            error: Some(stderr.to_string()),
        })
    }
}

// recommended_apps 响应结构体
#[derive(Serialize)]
pub struct RecommendedAppsResponse {
    pub success: bool,
    pub data: Option<serde_json::Value>,
    pub error: Option<String>,
}

// services_status 响应结构体
#[derive(Serialize)]
pub struct ServiceStatus {
    pub name: String,
    pub enabled: String,
    pub active: String,
}

#[derive(Serialize)]
pub struct ServicesStatusResponse {
    pub success: bool,
    pub data: Vec<ServiceStatus>,
    pub error: Option<String>,
}

/// system/services_status API - 获取服务状态（需要 JWT 认证）
#[get("/system/services_status")]
pub async fn get_services_status(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    // 1. 验证 JWT token
    let _claims = match crate::utils::admin::validate_token_with_db(&req, &pool).await {
        Ok(claims) => claims,
        Err(response) => return response,
    };

    let services = vec!["ttyd.service", "ssh.service"];
    let mut result = Vec::with_capacity(services.len());

    for svc in services {
        let enabled = match Command::new("systemctl")
            .args(["is-enabled", svc])
            .output()
        {
            Ok(output) => {
                let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if output.status.success() {
                    text
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                    if !stderr.is_empty() { stderr } else { text }
                }
            }
            Err(e) => format!("error: {}", e),
        };

        let active = match Command::new("systemctl")
            .args(["is-active", svc])
            .output()
        {
            Ok(output) => {
                let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if output.status.success() {
                    text
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                    if !stderr.is_empty() { stderr } else { text }
                }
            }
            Err(e) => format!("error: {}", e),
        };

        result.push(ServiceStatus {
            name: svc.to_string(),
            enabled,
            active,
        });
    }

    HttpResponse::Ok().json(ServicesStatusResponse {
        success: true,
        data: result,
        error: None,
    })
}

/// system/recommended_apps API - 获取推荐应用列表（需要 JWT 认证）
#[get("/system/recommended_apps")]
pub async fn get_recommended_apps(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    // 1. 验证 JWT token
    let _claims = match crate::utils::admin::validate_token_with_db(&req, &pool).await {
        Ok(claims) => claims,
        Err(response) => return response,
    };

    let path = match std::env::var("RECOMMENDED_APPS") {
        Ok(p) => p,
        Err(_) => {
            return HttpResponse::Ok().json(RecommendedAppsResponse {
                success: true,
                data: Some(serde_json::Value::Array(vec![])),
                error: None,
            });
        }
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    {
        let cache = RECOMMENDED_APPS_CACHE.lock().await;
        if let (Some(data), Some(cached_at), Some(cached_path)) =
            (cache.data.as_ref(), cache.cached_at, cache.path.as_ref())
        {
            if cached_path == &path && now.saturating_sub(cached_at) < RECOMMENDED_APPS_CACHE_TTL_SECONDS {
                return HttpResponse::Ok().json(RecommendedAppsResponse {
                    success: true,
                    data: Some(data.clone()),
                    error: None,
                });
            }
        }
        drop(cache);
    }

// 4. 获取数据（增加超时控制）
    let content = if path.starts_with("http") {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10)) // 增加超时
            .build()
            .unwrap();

        match client.get(&path).send().await {
            Ok(resp) => resp.text().await.map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        }
    } else {
        std::fs::read_to_string(&path).map_err(|e| e.to_string())
    };

    let content_text = match content {
        Ok(t) => t,
        Err(e) => return HttpResponse::InternalServerError().json(RecommendedAppsResponse {
            success: false, data: None, error: Some(e) 
        }),
    };

    let json_data: serde_json::Value = match serde_json::from_str(&content_text) {
        Ok(v) => v,
        Err(e) => {
            return HttpResponse::InternalServerError().json(RecommendedAppsResponse {
                success: false,
                data: None,
                error: Some(format!("Failed to parse recommended apps JSON: {}", e)),
            });
        }
    };

    {
        let mut cache = RECOMMENDED_APPS_CACHE.lock().await;
        cache.data = Some(json_data.clone());
        cache.cached_at = Some(now);
        cache.path = Some(path.clone());
    }

    HttpResponse::Ok().json(RecommendedAppsResponse {
        success: true,
        data: Some(json_data),
        error: None,
    })
}
