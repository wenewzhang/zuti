use actix_web::{get, post, HttpRequest, HttpResponse, Responder, web};
use serde::Serialize;
use std::process::Command;

use crate::utils::admin::verify_admin_access;

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

/// system/recommended_apps API - 获取推荐应用列表（无需认证）
#[get("/system/recommended_apps")]
pub async fn get_recommended_apps() -> impl Responder {
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

    let content = if path.starts_with("http://") || path.starts_with("https://") {
        match reqwest::get(&path).await {
            Ok(resp) => match resp.text().await {
                Ok(text) => text,
                Err(e) => {
                    return HttpResponse::InternalServerError().json(RecommendedAppsResponse {
                        success: false,
                        data: None,
                        error: Some(format!("Failed to read response body from '{}': {}", path, e)),
                    });
                }
            },
            Err(e) => {
                return HttpResponse::InternalServerError().json(RecommendedAppsResponse {
                    success: false,
                    data: None,
                    error: Some(format!("Failed to fetch recommended apps from '{}': {}", path, e)),
                });
            }
        }
    } else {
        match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                return HttpResponse::InternalServerError().json(RecommendedAppsResponse {
                    success: false,
                    data: None,
                    error: Some(format!("Failed to read recommended apps file '{}': {}", path, e)),
                });
            }
        }
    };

    let json_data: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            return HttpResponse::InternalServerError().json(RecommendedAppsResponse {
                success: false,
                data: None,
                error: Some(format!("Failed to parse recommended apps JSON: {}", e)),
            });
        }
    };

    HttpResponse::Ok().json(RecommendedAppsResponse {
        success: true,
        data: Some(json_data),
        error: None,
    })
}
