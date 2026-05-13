use actix_web::{get, post, HttpRequest, HttpResponse, Responder, web};
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
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

    let services = vec!["ttyd.service", "ssh.service", "sysconfig"];
    let mut result = Vec::with_capacity(services.len());

    for svc in services {
        let (enabled, active) = if svc == "sysconfig" {
            let exists = std::path::Path::new("/etc/systemd/system/getty@tty1.service.d/override.conf").exists();
            if exists {
                ("true".to_string(), "true".to_string())
            } else {
                ("false".to_string(), "false".to_string())
            }
        } else {
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

            (enabled, active)
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

// service_start 请求结构体
#[derive(Deserialize)]
pub struct ServiceStartRequest {
    pub service_name: String,
}

// service_start 响应结构体
#[derive(Serialize)]
pub struct ServiceStartResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

/// system/service_start API - 启动指定服务（需要 JWT 认证）
#[post("/system/service_start")]
pub async fn start_service(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
    body: web::Json<ServiceStartRequest>,
) -> impl Responder {
    // 1. 验证 JWT token 并检查 admin 权限
    let _username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let service_name = body.service_name.trim();
    if service_name.is_empty() {
        return HttpResponse::BadRequest().json(ServiceStartResponse {
            success: false,
            message: "Service name cannot be empty".to_string(),
            error: Some("service_name is required".to_string()),
        });
    }

    // 2. 执行 systemctl start <service_name> 命令
    let output = match Command::new("systemctl")
        .args(["start", service_name])
        .output()
    {
        Ok(result) => result,
        Err(e) => {
            return HttpResponse::InternalServerError().json(ServiceStartResponse {
                success: false,
                message: "Failed to execute systemctl start command".to_string(),
                error: Some(format!("Command error: {}", e)),
            });
        }
    };

    if output.status.success() {
        HttpResponse::Ok().json(ServiceStartResponse {
            success: true,
            message: format!("Service '{}' started successfully", service_name),
            error: None,
        })
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        HttpResponse::InternalServerError().json(ServiceStartResponse {
            success: false,
            message: format!("Failed to start service '{}'", service_name),
            error: Some(stderr.to_string()),
        })
    }
}

// service_stop 请求结构体
#[derive(Deserialize)]
pub struct ServiceStopRequest {
    pub service_name: String,
}

// service_stop 响应结构体
#[derive(Serialize)]
pub struct ServiceStopResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

/// system/service_stop API - 停止指定服务（需要 JWT 认证）
#[post("/system/service_stop")]
pub async fn stop_service(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
    body: web::Json<ServiceStopRequest>,
) -> impl Responder {
    // 1. 验证 JWT token 并检查 admin 权限
    let _username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let service_name = body.service_name.trim();
    if service_name.is_empty() {
        return HttpResponse::BadRequest().json(ServiceStopResponse {
            success: false,
            message: "Service name cannot be empty".to_string(),
            error: Some("service_name is required".to_string()),
        });
    }

    // 2. 执行 systemctl stop <service_name> 命令
    let output = match Command::new("systemctl")
        .args(["stop", service_name])
        .output()
    {
        Ok(result) => result,
        Err(e) => {
            return HttpResponse::InternalServerError().json(ServiceStopResponse {
                success: false,
                message: "Failed to execute systemctl stop command".to_string(),
                error: Some(format!("Command error: {}", e)),
            });
        }
    };

    if output.status.success() {
        HttpResponse::Ok().json(ServiceStopResponse {
            success: true,
            message: format!("Service '{}' stopped successfully", service_name),
            error: None,
        })
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        HttpResponse::InternalServerError().json(ServiceStopResponse {
            success: false,
            message: format!("Failed to stop service '{}'", service_name),
            error: Some(stderr.to_string()),
        })
    }
}

// service_restart 请求结构体
#[derive(Deserialize)]
pub struct ServiceRestartRequest {
    pub service_name: String,
}

// service_restart 响应结构体
#[derive(Serialize)]
pub struct ServiceRestartResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

/// system/service_restart API - 重启指定服务（需要 JWT 认证）
#[post("/system/service_restart")]
pub async fn restart_service(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
    body: web::Json<ServiceRestartRequest>,
) -> impl Responder {
    // 1. 验证 JWT token 并检查 admin 权限
    let _username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let service_name = body.service_name.trim();
    if service_name.is_empty() {
        return HttpResponse::BadRequest().json(ServiceRestartResponse {
            success: false,
            message: "Service name cannot be empty".to_string(),
            error: Some("service_name is required".to_string()),
        });
    }

    // 2. 执行 systemctl restart <service_name> 命令
    let output = match Command::new("systemctl")
        .args(["restart", service_name])
        .output()
    {
        Ok(result) => result,
        Err(e) => {
            return HttpResponse::InternalServerError().json(ServiceRestartResponse {
                success: false,
                message: "Failed to execute systemctl restart command".to_string(),
                error: Some(format!("Command error: {}", e)),
            });
        }
    };

    if output.status.success() {
        HttpResponse::Ok().json(ServiceRestartResponse {
            success: true,
            message: format!("Service '{}' restarted successfully", service_name),
            error: None,
        })
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        HttpResponse::InternalServerError().json(ServiceRestartResponse {
            success: false,
            message: format!("Failed to restart service '{}'", service_name),
            error: Some(stderr.to_string()),
        })
    }
}

// service_autostart 请求结构体
#[derive(Deserialize)]
pub struct ServiceAutostartRequest {
    pub service_name: String,
    pub enable: bool,
}

// service_autostart 响应结构体
#[derive(Serialize)]
pub struct ServiceAutostartResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

/// system/service_autostart API - 设置服务开机自启（需要 JWT 认证）
#[post("/system/service_autostart")]
pub async fn service_autostart(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
    body: web::Json<ServiceAutostartRequest>,
) -> impl Responder {
    // 1. 验证 JWT token 并检查 admin 权限
    let _username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let service_name = body.service_name.trim();
    if service_name.is_empty() {
        return HttpResponse::BadRequest().json(ServiceAutostartResponse {
            success: false,
            message: "Service name cannot be empty".to_string(),
            error: Some("service_name is required".to_string()),
        });
    }

    // 2. 执行 systemctl enable/disable <service_name> 命令
    let action = if body.enable { "enable" } else { "disable" };
    let output = match Command::new("systemctl")
        .args([action, service_name])
        .output()
    {
        Ok(result) => result,
        Err(e) => {
            return HttpResponse::InternalServerError().json(ServiceAutostartResponse {
                success: false,
                message: format!("Failed to execute systemctl {} command", action),
                error: Some(format!("Command error: {}", e)),
            });
        }
    };

    if output.status.success() {
        HttpResponse::Ok().json(ServiceAutostartResponse {
            success: true,
            message: format!("Service '{}' {}d successfully", service_name, action),
            error: None,
        })
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        HttpResponse::InternalServerError().json(ServiceAutostartResponse {
            success: false,
            message: format!("Failed to {} service '{}'", action, service_name),
            error: Some(stderr.to_string()),
        })
    }
}

const SSHD_CONFIG_PATH: &str = "/etc/ssh/sshd_config";

// ssh_setting 响应结构体
#[derive(Serialize)]
pub struct SshSettingResponse {
    pub success: bool,
    pub data: Option<SshSettingData>,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct SshSettingData {
    pub permit_root_login: String,
    pub password_authentication: String,
}

fn read_sshd_setting(key: &str) -> Option<String> {
    let content = std::fs::read_to_string(SSHD_CONFIG_PATH).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        if let Some(k) = parts.next() {
            if k.eq_ignore_ascii_case(key) {
                if let Some(v) = parts.next() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

/// system/ssh_setting API - 获取 SSH 配置（需要 JWT 认证）
#[get("/system/ssh_setting")]
pub async fn get_ssh_setting(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    let _claims = match crate::utils::admin::validate_token_with_db(&req, &pool).await {
        Ok(claims) => claims,
        Err(response) => return response,
    };

    let permit_root_login = read_sshd_setting("PermitRootLogin").filter(|s| !s.is_empty()).unwrap_or_else(|| "no".to_string());
    let password_authentication = read_sshd_setting("PasswordAuthentication").filter(|s| !s.is_empty()).unwrap_or_else(|| "no".to_string());

    HttpResponse::Ok().json(SshSettingResponse {
        success: true,
        data: Some(SshSettingData {
            permit_root_login,
            password_authentication,
        }),
        error: None,
    })
}

// ssh_setting 请求结构体
#[derive(Deserialize)]
pub struct SshSettingRequest {
    pub permit_root_login: String,
    pub password_authentication: String,
}

// ssh_setting 设置响应结构体
#[derive(Serialize)]
pub struct SshSettingUpdateResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

fn write_sshd_setting(key: &str, value: &str) -> Result<(), String> {
    let content = std::fs::read_to_string(SSHD_CONFIG_PATH)
        .map_err(|e| format!("Failed to read sshd_config: {}", e))?;

    let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    let mut found = false;

    for line in &mut lines {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            let after_hash = trimmed[1..].trim();
            let mut parts = after_hash.split_whitespace();
            if let Some(k) = parts.next() {
                if k.eq_ignore_ascii_case(key) {
                    *line = format!("{} {}", key, value);
                    found = true;
                    break;
                }
            }
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        if let Some(k) = parts.next() {
            if k.eq_ignore_ascii_case(key) {
                *line = format!("{} {}", key, value);
                found = true;
                break;
            }
        }
    }

    if !found {
        lines.push(format!("{} {}", key, value));
    }

    std::fs::write(SSHD_CONFIG_PATH, lines.join("\n") + "\n")
        .map_err(|e| format!("Failed to write sshd_config: {}", e))?;

    Ok(())
}

/// system/ssh_setting API - 设置 SSH 配置（需要 JWT 认证，仅管理员可用）
#[post("/system/ssh_setting")]
pub async fn set_ssh_setting(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
    body: web::Json<SshSettingRequest>,
) -> impl Responder {
    let _username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let valid_permit_root_login = ["yes", "no", "prohibit-password"];
    let permit_root_login = body.permit_root_login.trim();
    if !valid_permit_root_login.contains(&permit_root_login) {
        return HttpResponse::BadRequest().json(SshSettingUpdateResponse {
            success: false,
            message: "Invalid permit_root_login value".to_string(),
            error: Some(format!(
                "permit_root_login must be one of: {}",
                valid_permit_root_login.join(", ")
            )),
        });
    }

    let valid_password_auth = ["yes", "no"];
    let password_authentication = body.password_authentication.trim();
    if !valid_password_auth.contains(&password_authentication) {
        return HttpResponse::BadRequest().json(SshSettingUpdateResponse {
            success: false,
            message: "Invalid password_authentication value".to_string(),
            error: Some(format!(
                "password_authentication must be one of: {}",
                valid_password_auth.join(", ")
            )),
        });
    }

    if let Err(e) = write_sshd_setting("PermitRootLogin", permit_root_login) {
        return HttpResponse::InternalServerError().json(SshSettingUpdateResponse {
            success: false,
            message: "Failed to update PermitRootLogin".to_string(),
            error: Some(e),
        });
    }

    if let Err(e) = write_sshd_setting("PasswordAuthentication", password_authentication) {
        return HttpResponse::InternalServerError().json(SshSettingUpdateResponse {
            success: false,
            message: "Failed to update PasswordAuthentication".to_string(),
            error: Some(e),
        });
    }

    // 重启 ssh 服务使配置生效
    let output = match Command::new("systemctl")
        .args(["restart", "ssh.service"])
        .output()
    {
        Ok(result) => result,
        Err(e) => {
            return HttpResponse::InternalServerError().json(SshSettingUpdateResponse {
                success: false,
                message: "Failed to restart ssh service".to_string(),
                error: Some(format!("Command error: {}", e)),
            });
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return HttpResponse::InternalServerError().json(SshSettingUpdateResponse {
            success: false,
            message: "Failed to restart ssh service".to_string(),
            error: Some(stderr.to_string()),
        });
    }

    HttpResponse::Ok().json(SshSettingUpdateResponse {
        success: true,
        message: "SSH setting updated and service restarted".to_string(),
        error: None,
    })
}
