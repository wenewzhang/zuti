use actix_web::{get, post, HttpRequest, HttpResponse, Responder, web};
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use std::process::Command;
use tokio::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::io::{BufRead, BufReader, Write};

use crate::utils::admin::verify_admin_access;
use crate::utils::consts::ZUTI_HELPER_SOCK;

const RECOMMENDED_APPS_CACHE_TTL_SECONDS: u64 = 3600;
const EXTRA_SPACE_BYTES: u64 = 1 * 1024 * 1024 * 1024;

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
    static ref UPDATE_STATUS: std::sync::Mutex<(String, String)> = std::sync::Mutex::new(("idle".to_string(), String::new()));
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
                ("enabled".to_string(), "active".to_string())
            } else {
                ("disabled".to_string(), "inactive".to_string())
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

    // 对 sysconfig 特殊处理
    if service_name == "sysconfig" {
        let override_path = std::path::Path::new("/etc/systemd/system/getty@tty1.service.d/override.conf");
        if body.enable {
            if let Some(parent) = override_path.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    return HttpResponse::InternalServerError().json(ServiceAutostartResponse {
                        success: false,
                        message: "Failed to create directory for override.conf".to_string(),
                        error: Some(format!("IO error: {}", e)),
                    });
                }
            }
            let content = "[Service]\nExecStart=\nExecStart=-/sbin/agetty --autologin sysconfig --noclear %I $TERM\n";
            if let Err(e) = std::fs::write(override_path, content) {
                return HttpResponse::InternalServerError().json(ServiceAutostartResponse {
                    success: false,
                    message: "Failed to write override.conf".to_string(),
                    error: Some(format!("IO error: {}", e)),
                });
            }
            let perms = std::fs::Permissions::from_mode(0o644);
            if let Err(e) = std::fs::set_permissions(override_path, perms) {
                return HttpResponse::InternalServerError().json(ServiceAutostartResponse {
                    success: false,
                    message: "Failed to set permissions for override.conf".to_string(),
                    error: Some(format!("IO error: {}", e)),
                });
            }
        } else if override_path.exists() {
            if let Err(e) = std::fs::remove_file(override_path) {
                return HttpResponse::InternalServerError().json(ServiceAutostartResponse {
                    success: false,
                    message: "Failed to remove override.conf".to_string(),
                    error: Some(format!("IO error: {}", e)),
                });
            }
        }
        return HttpResponse::Ok().json(ServiceAutostartResponse {
            success: true,
            message: format!("Sysconfig autologin {}d successfully", if body.enable { "enable" } else { "disable" }),
            error: None,
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

// update_check 响应结构体
#[derive(Serialize)]
pub struct UpdateCheckResponse {
    pub success: bool,
    pub update_available: bool,
    pub current_version: String,
    pub latest_version: String,
    pub error: Option<String>,
}

#[derive(Deserialize)]
pub struct Manifest {
    pub version: String,
    pub filename: String,
    pub checksum: String,
    pub filesize: u64,
}

fn parse_df_available(df_output: &str) -> Option<u64> {
    let line = df_output.lines().nth(1)?;
    let parts: Vec<&str> = line.split_whitespace().collect();
    // df -B1 output format: Filesystem 1B-blocks Used Available Use% Mounted on
    // Available is the 4th column (index 3)
    if parts.len() < 5 {
        return None;
    }
    parts[3].parse().ok()
}

/// system/update_check API - 检查系统更新（需要 JWT 认证）
#[get("/system/update_check")]
pub async fn update_check(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {

    // 1. 验证 JWT token
    let _claims = match crate::utils::admin::validate_token_with_db(&req, &pool).await {
        Ok(claims) => claims,
        Err(response) => return response,
    };

    // 2. 检查 UPDATE_STATUS 是否为 idle
    let current_status = match UPDATE_STATUS.lock() {
        Ok(s) => s.0.clone(),
        Err(_) => {
            return HttpResponse::InternalServerError().json(UpdateCheckResponse {
                success: false,
                update_available: false,
                current_version: "".to_string(),
                latest_version: "".to_string(),
                error: Some("Failed to read update status".to_string()),
            });
        }
    };

    if current_status != "idle" {
        return HttpResponse::Conflict().json(UpdateCheckResponse {
            success: false,
            update_available: false,
            current_version: "".to_string(),
            latest_version: "".to_string(),
            error: Some(format!("Update check unavailable, current status: {}", current_status)),
        });
    }

    // 3. 读取本地版本
    let local_version = match std::fs::read_to_string("/.data/.version") {
        Ok(v) => v.trim().to_string(),
        Err(e) => {
            return HttpResponse::InternalServerError().json(UpdateCheckResponse {
                success: false,
                update_available: false,
                current_version: "".to_string(),
                latest_version: "".to_string(),
                error: Some(format!("Failed to read local version: {}", e)),
            });
        }
    };

    // 4. 构建 manifest URL
    let update_url = match std::env::var("UPDATE_URL") {
        Ok(u) => u,
        Err(_) => {
            return HttpResponse::InternalServerError().json(UpdateCheckResponse {
                success: false,
                update_available: false,
                current_version: local_version.clone(),
                latest_version: "".to_string(),
                error: Some("UPDATE_URL not set".to_string()),
            });
        }
    };

    let channel = match std::env::var("CHANNEL") {
        Ok(c) => c,
        Err(_) => {
            return HttpResponse::InternalServerError().json(UpdateCheckResponse {
                success: false,
                update_available: false,
                current_version: local_version.clone(),
                latest_version: "".to_string(),
                error: Some("CHANNEL not set".to_string()),
            });
        }
    };

    let manifest_url = format!("{}/{}/manifest.json", update_url.trim_end_matches('/'), channel);

    // 5. 获取远程 manifest
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(UpdateCheckResponse {
                success: false,
                update_available: false,
                current_version: local_version.clone(),
                latest_version: "".to_string(),
                error: Some(format!("Failed to build HTTP client: {}", e)),
            });
        }
    };

    let resp = match client.get(&manifest_url).send().await {
        Ok(r) => r,
        Err(e) => {
            return HttpResponse::InternalServerError().json(UpdateCheckResponse {
                success: false,
                update_available: false,
                current_version: local_version.clone(),
                latest_version: "".to_string(),
                error: Some(format!("Failed to fetch manifest: {}", e)),
            });
        }
    };

    let manifest_text = match resp.text().await {
        Ok(t) => t,
        Err(e) => {
            return HttpResponse::InternalServerError().json(UpdateCheckResponse {
                success: false,
                update_available: false,
                current_version: local_version.clone(),
                latest_version: "".to_string(),
                error: Some(format!("Failed to read manifest response: {}", e)),
            });
        }
    };

    let manifest: Manifest = match serde_json::from_str(&manifest_text) {
        Ok(m) => m,
        Err(e) => {
            return HttpResponse::InternalServerError().json(UpdateCheckResponse {
                success: false,
                update_available: false,
                current_version: local_version.clone(),
                latest_version: "".to_string(),
                error: Some(format!("Failed to parse manifest JSON: {}", e)),
            });
        }
    };

    // 6. 比较版本（取后6位进行字符串大小比较）
    fn get_version_suffix(v: &str) -> &str {
        if v.len() >= 6 {
            &v[v.len() - 6..]
        } else {
            v
        }
    }

    let local_suffix = get_version_suffix(&local_version);
    let remote_suffix = get_version_suffix(&manifest.version);

    let update_available = remote_suffix > local_suffix;

    HttpResponse::Ok().json(UpdateCheckResponse {
        success: true,
        update_available,
        current_version: local_version,
        latest_version: manifest.version,
        error: None,
    })
}


// ============================================================================
// Update Download Task System
// ============================================================================

use std::collections::HashMap;
use std::sync::Arc;

lazy_static! {
    static ref DOWNLOAD_TASKS: Arc<std::sync::Mutex<HashMap<String, DownloadTask>>> =
        Arc::new(std::sync::Mutex::new(HashMap::new()));
}

/// Download task status
#[derive(Serialize, Clone, Debug)]
pub enum DownloadTaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Stopped,
}

/// Download task information
#[derive(Serialize, Clone)]
pub struct DownloadTask {
    pub task_id: String,
    pub status: DownloadTaskStatus,
    pub progress: u8,              // 0-100
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub filename: String,
    pub file_path: String,
    pub message: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub cancelled: bool,
}

/// Start download response
#[derive(Serialize)]
pub struct StartDownloadResponse {
    pub success: bool,
    pub task_id: String,
    pub message: String,
    pub error: Option<String>,
}

/// Get download task response
#[derive(Serialize)]
pub struct GetDownloadTaskResponse {
    pub success: bool,
    pub task: Option<DownloadTask>,
    pub message: String,
    pub error: Option<String>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn update_download_task<F>(task_id: &str, updater: F)
where
    F: FnOnce(&mut DownloadTask),
{
    if let Ok(mut tasks) = DOWNLOAD_TASKS.lock() {
        if let Some(task) = tasks.get_mut(task_id) {
            updater(task);
            task.updated_at = now_secs();
        }
    }
}

/// Check if there is already a pending or running download task
fn has_active_download_task() -> Option<String> {
    if let Ok(tasks) = DOWNLOAD_TASKS.lock() {
        for (id, task) in tasks.iter() {
            match task.status {
                DownloadTaskStatus::Pending | DownloadTaskStatus::Running => {
                    return Some(id.clone());
                }
                _ => {}
            }
        }
    }
    None
}

/// system/update_download/start API - Start system update download (Admin only)
/// Only one download task can run at a time.
#[post("/system/update_download/start")]
pub async fn start_update_download(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {

    // 1. Verify admin access
    let _username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    // 2. Check if another task is already active
    if let Some(existing_task_id) = has_active_download_task() {
        return HttpResponse::Conflict().json(StartDownloadResponse {
            success: false,
            task_id: existing_task_id,
            message: "Another download task is already active".to_string(),
            error: Some("Only one download task can run at a time".to_string()),
        });
    }

    // 3. Read UPDATE_URL and CHANNEL from env
    let update_url = match std::env::var("UPDATE_URL") {
        Ok(u) => u,
        Err(_) => {
            return HttpResponse::InternalServerError().json(StartDownloadResponse {
                success: false,
                task_id: String::new(),
                message: "UPDATE_URL not set".to_string(),
                error: Some("UPDATE_URL environment variable is missing".to_string()),
            });
        }
    };

    let channel = match std::env::var("CHANNEL") {
        Ok(c) => c,
        Err(_) => {
            return HttpResponse::InternalServerError().json(StartDownloadResponse {
                success: false,
                task_id: String::new(),
                message: "CHANNEL not set".to_string(),
                error: Some("CHANNEL environment variable is missing".to_string()),
            });
        }
    };

    let manifest_url = format!("{}/{}/manifest.json", update_url.trim_end_matches('/'), channel);

    // 4. Fetch manifest.json to get filename
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(StartDownloadResponse {
                success: false,
                task_id: String::new(),
                message: "Failed to build HTTP client".to_string(),
                error: Some(e.to_string()),
            });
        }
    };

    let manifest_text = match client.get(&manifest_url).send().await {
        Ok(resp) => match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                return HttpResponse::InternalServerError().json(StartDownloadResponse {
                    success: false,
                    task_id: String::new(),
                    message: "Failed to read manifest response".to_string(),
                    error: Some(e.to_string()),
                });
            }
        },
        Err(e) => {
            return HttpResponse::InternalServerError().json(StartDownloadResponse {
                success: false,
                task_id: String::new(),
                message: "Failed to fetch manifest".to_string(),
                error: Some(e.to_string()),
            });
        }
    };

    let manifest: Manifest = match serde_json::from_str(&manifest_text) {
        Ok(m) => m,
        Err(e) => {
            return HttpResponse::InternalServerError().json(StartDownloadResponse {
                success: false,
                task_id: String::new(),
                message: "Failed to parse manifest JSON".to_string(),
                error: Some(e.to_string()),
            });
        }
    };

    if manifest.filename.is_empty() {
        return HttpResponse::InternalServerError().json(StartDownloadResponse {
            success: false,
            task_id: String::new(),
            message: "Manifest filename is empty".to_string(),
            error: Some("Invalid manifest: filename is empty".to_string()),
        });
    }

    if manifest.checksum.is_empty() {
        return HttpResponse::InternalServerError().json(StartDownloadResponse {
            success: false,
            task_id: String::new(),
            message: "Manifest checksum is empty".to_string(),
            error: Some("Invalid manifest: checksum is empty".to_string()),
        });
    }

    // 5. Check available disk space using df -B1 /
    let df_output = match Command::new("df").args(&["-B1", "/"]).output() {
        Ok(output) => String::from_utf8_lossy(&output.stdout).to_string(),
        Err(e) => {
            return HttpResponse::InternalServerError().json(StartDownloadResponse {
                success: false,
                task_id: String::new(),
                message: "Failed to check disk space".to_string(),
                error: Some(e.to_string()),
            });
        }
    };

    let available_space = parse_df_available(&df_output).unwrap_or(0);
    let required_space = manifest.filesize * 3 + EXTRA_SPACE_BYTES; // filesize * 2 + 2GB

    if available_space < required_space {
        return HttpResponse::InternalServerError().json(StartDownloadResponse {
            success: false,
            task_id: String::new(),
            message: format!(
                "Insufficient disk space. Available: {} bytes, Required: {} bytes (filesize * 2 + 2GB)",
                available_space, required_space
            ),
            error: Some("Disk space check failed".to_string()),
        });
    }

    // 6. Create task
    let task_id = uuid::Uuid::new_v4().to_string();
    let now = now_secs();
    let file_path = format!("/tmp/{}", manifest.filename);
    let filename = manifest.filename.clone();
    let download_url = format!(
        "{}/{}/{}",
        update_url.trim_end_matches('/'),
        channel,
        filename
    );

    let expected_checksum = manifest.checksum.clone();

    let task = DownloadTask {
        task_id: task_id.clone(),
        status: DownloadTaskStatus::Pending,
        progress: 0,
        downloaded_bytes: 0,
        total_bytes: 0,
        filename: filename.clone(),
        file_path: file_path.clone(),
        message: "Waiting to start download".to_string(),
        created_at: now,
        updated_at: now,
        cancelled: false,
    };

    DOWNLOAD_TASKS.lock().unwrap().insert(task_id.clone(), task);

    // 6. Spawn background download
    let task_id_clone = task_id.clone();
    actix_web::rt::spawn(async move {
        execute_download_task(task_id_clone, download_url, file_path, expected_checksum).await;
    });

    // 设置更新状态为 downloading
    if let Ok(mut status) = UPDATE_STATUS.lock() {
        status.0 = "downloading".to_string();
        status.1 = task_id.clone();
    }

    HttpResponse::Ok().json(StartDownloadResponse {
        success: true,
        task_id,
        message: "Download task started".to_string(),
        error: None,
    })
}

/// system/update_download/task/{task_id} API - Get download task status
#[get("/system/update_download/task/{task_id}")]
pub async fn get_update_download_task(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
    path: web::Path<String>,
) -> impl Responder {
    // 1. Verify JWT token
    let _claims = match crate::utils::admin::validate_token_with_db(&req, &pool).await {
        Ok(claims) => claims,
        Err(response) => return response,
    };

    let task_id = path.into_inner();

    match DOWNLOAD_TASKS.lock().unwrap().get(&task_id) {
        Some(task) => HttpResponse::Ok().json(GetDownloadTaskResponse {
            success: true,
            task: Some(task.clone()),
            message: "Task found".to_string(),
            error: None,
        }),
        None => HttpResponse::NotFound().json(GetDownloadTaskResponse {
            success: false,
            task: None,
            message: "Task not found".to_string(),
            error: Some(format!("No task with id {}", task_id)),
        }),
    }
}

// stop_download 响应结构体
#[derive(Serialize)]
pub struct StopDownloadResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

/// system/update_download/stop API - Stop active update download task (Admin only)
#[post("/system/update_download/stop")]
pub async fn stop_update_download(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    // 1. Verify admin access
    let _username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    if let Ok(mut status) = UPDATE_STATUS.lock() {
        status.0 = "idle".to_string();
    }    
    // 2. Find active task
    let active_task_id = {
        if let Ok(tasks) = DOWNLOAD_TASKS.lock() {
            tasks
                .iter()
                .find(|(_, task)| {
                    matches!(task.status, DownloadTaskStatus::Pending | DownloadTaskStatus::Running)
                })
                .map(|(id, _)| id.clone())
        } else {
            None
        }
    };

    let task_id = match active_task_id {
        Some(id) => id,
        None => {
            return HttpResponse::BadRequest().json(StopDownloadResponse {
                success: false,
                message: "No active download task".to_string(),
                error: Some("There is no pending or running download task to stop".to_string()),
            });
        }
    };

    // 3. Mark task as cancelled
    update_download_task(&task_id, |t| {
        t.cancelled = true;
        t.message = "Stopping download...".to_string();
    });

    HttpResponse::Ok().json(StopDownloadResponse {
        success: true,
        message: format!("Download task '{}' is being stopped", task_id),
        error: None,
    })
}

/// Background download execution
async fn execute_download_task(task_id: String, download_url: String, file_path: String, expected_checksum: String) {
    log::info!("execute_download_task started: task_id={}, download_url={}, file_path={}", task_id, download_url, file_path);

    // Update to running
    update_download_task(&task_id, |t| {
        t.status = DownloadTaskStatus::Running;
        t.message = "Starting download...".to_string();
        t.progress = 0;
    });

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
    {
        Ok(c) => {
            log::info!("HTTP client built successfully");
            c
        }
        Err(e) => {
            log::error!("Failed to build HTTP client: {}", e);
            update_download_task(&task_id, |t| {
                t.status = DownloadTaskStatus::Failed;
                t.message = format!("Failed to build HTTP client: {}", e);
                t.progress = 0;
            });
            return;
        }
    };

    let mut resp = match client.get(&download_url).send().await {
        Ok(r) => {
            if !r.status().is_success() {
                log::error!("HTTP error: status={}", r.status());
                update_download_task(&task_id, |t| {
                    t.status = DownloadTaskStatus::Failed;
                    t.message = format!("HTTP error: {}", r.status());
                    t.progress = 0;
                });
                return;
            }
            log::info!("HTTP response received: status={}", r.status());
            r
        }
        Err(e) => {
            log::error!("Failed to start download: {}", e);
            update_download_task(&task_id, |t| {
                t.status = DownloadTaskStatus::Failed;
                t.message = format!("Failed to start download: {}", e);
                t.progress = 0;
            });
            return;
        }
    };

    let total_bytes = resp
        .content_length()
        .unwrap_or(0);
    log::info!("total_bytes={}", total_bytes);

    update_download_task(&task_id, |t| {
        t.total_bytes = total_bytes;
        t.message = "Downloading...".to_string();
    });

    let mut file = match std::fs::File::create(&file_path) {
        Ok(f) => {
            log::info!("File created successfully: file_path={}", file_path);
            f
        }
        Err(e) => {
            log::error!("Failed to create file: file_path={}, error={}", file_path, e);
            update_download_task(&task_id, |t| {
                t.status = DownloadTaskStatus::Failed;
                t.message = format!("Failed to create file: {}", e);
                t.progress = 0;
            });
            return;
        }
    };

    let mut downloaded: u64 = 0;
    let mut last_progress: u8 = 0;
    log::info!("Starting download loop: downloaded={}, last_progress={}", downloaded, last_progress);

    loop {
        // Check cancellation
        let is_cancelled = {
            if let Ok(tasks) = DOWNLOAD_TASKS.lock() {
                tasks.get(&task_id).map(|t| t.cancelled).unwrap_or(false)
            } else {
                false
            }
        };
        if is_cancelled {
            log::info!("Download task cancelled: task_id={}", task_id);
            let _ = std::fs::remove_file(&file_path);
            update_download_task(&task_id, |t| {
                t.status = DownloadTaskStatus::Stopped;
                t.message = "Download stopped by user".to_string();
            });
            // Reset update status
            if let Ok(mut status) = UPDATE_STATUS.lock() {
                status.0 = "idle".to_string();
                status.1 = String::new();
            }
            return;
        }

        match resp.chunk().await {
            Ok(Some(chunk)) => {
                let chunk_len = chunk.len() as u64;
                // log::info!("Received chunk: chunk_len={}", chunk_len);
                if let Err(e) = std::io::Write::write_all(&mut file, &chunk) {
                    log::error!("Failed to write file: {}", e);
                    update_download_task(&task_id, |t| {
                        t.status = DownloadTaskStatus::Failed;
                        t.message = format!("Failed to write file: {}", e);
                    });
                    let _ = std::fs::remove_file(&file_path);
                    return;
                }
                downloaded += chunk_len;
                // log::info!("downloaded={}", downloaded);

                let progress = if total_bytes > 0 {
                    ((downloaded as f64 / total_bytes as f64) * 100.0) as u8
                } else {
                    0
                };
                // log::info!("progress={}", progress);

                // Update progress every 1% change to reduce lock contention
                if progress != last_progress {
                    update_download_task(&task_id, |t| {
                        t.downloaded_bytes = downloaded;
                        t.progress = progress;
                    });
                    last_progress = progress;
                    log::info!("last_progress updated to {}", last_progress);
                }
            }
            Ok(None) => {
                log::info!("Download stream ended");
                break;
            }
            Err(e) => {
                log::error!("Download stream error: {}", e);
                update_download_task(&task_id, |t| {
                    t.status = DownloadTaskStatus::Failed;
                    t.message = format!("Download stream error: {}", e);
                });
                let _ = std::fs::remove_file(&file_path);
                return;
            }
        }
    }

    if let Err(e) = std::io::Write::flush(&mut file) {
        log::error!("Failed to flush file: {}", e);
        update_download_task(&task_id, |t| {
            t.status = DownloadTaskStatus::Failed;
            t.message = format!("Failed to flush file: {}", e);
        });
        let _ = std::fs::remove_file(&file_path);
        return;
    }
    log::info!("File flushed successfully");

    // Verify SHA256 checksum using sha256sum command
    let output = match Command::new("sha256sum")
        .arg(&file_path)
        .output()
    {
        Ok(result) => result,
        Err(e) => {
            log::error!("Failed to run sha256sum: {}", e);
            let _ = std::fs::remove_file(&file_path);
            update_download_task(&task_id, |t| {
                t.status = DownloadTaskStatus::Failed;
                t.message = format!("Failed to run sha256sum: {}", e);
            });
            return;
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::error!("sha256sum command failed: {}", stderr);
        let _ = std::fs::remove_file(&file_path);
        update_download_task(&task_id, |t| {
            t.status = DownloadTaskStatus::Failed;
            t.message = format!("sha256sum command failed: {}", stderr);
        });
        return;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let actual_checksum = stdout.split_whitespace().next().unwrap_or("");
    if actual_checksum != expected_checksum {
        log::error!(
            "Checksum mismatch: expected={}, actual={}",
            expected_checksum,
            actual_checksum
        );
        let _ = std::fs::remove_file(&file_path);
        update_download_task(&task_id, |t| {
            t.status = DownloadTaskStatus::Failed;
            t.message = format!(
                "Checksum mismatch: expected={}, actual={}",
                expected_checksum,
                actual_checksum
            );
        });
        return;
    }

    log::info!("Checksum verified successfully: {}", actual_checksum);

    update_download_task(&task_id, |t| {
        t.status = DownloadTaskStatus::Completed;
        t.progress = 100;
        t.downloaded_bytes = downloaded;
        t.message = format!("Download completed and verified: {}", t.filename);
    });
    log::info!("execute_download_task completed: task_id={}, downloaded={}, file_path={}", task_id, downloaded, file_path);
}

// update_status 响应结构体
#[derive(Serialize)]
pub struct UpdateStatusResponse {
    pub success: bool,
    pub status: String,
    pub task_id: String,
    pub error: Option<String>,
}

/// system/update_status API - 获取系统更新状态（需要 JWT 认证）
#[get("/system/update_status")]
pub async fn get_update_status(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    // 1. 验证 JWT token
    let _claims = match crate::utils::admin::validate_token_with_db(&req, &pool).await {
        Ok(claims) => claims,
        Err(response) => return response,
    };

    // 2. 读取全局状态
    let (status, task_id) = match UPDATE_STATUS.lock() {
        Ok(s) => s.clone(),
        Err(e) => {
            return HttpResponse::InternalServerError().json(UpdateStatusResponse {
                success: false,
                status: "unknown".to_string(),
                task_id: String::new(),
                error: Some(format!("Failed to read update status: {}", e)),
            });
        }
    };

    HttpResponse::Ok().json(UpdateStatusResponse {
        success: true,
        status,
        task_id,
        error: None,
    })
}

// upgrade 请求结构体
#[derive(Deserialize)]
pub struct UpgradeRequest {
    pub file_path: Option<String>,
}

// upgrade progress 响应结构体
#[derive(Serialize)]
pub struct UpgradeProgressResponse {
    pub success: bool,
    pub data: Option<serde_json::Value>,
    pub error: Option<String>,
}

// upgrade 响应结构体
#[derive(Serialize)]
pub struct UpgradeResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

/// system/update_download/upgrade API - 执行系统升级（需要 JWT 认证，仅管理员可用）
#[post("/system/update_download/upgrade")]
pub async fn upgrade_system(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
    body: web::Json<UpgradeRequest>,
) -> impl Responder {
    // 1. 验证 JWT token 并检查 admin 权限
    let _username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    // 2. 确定升级文件路径
    let file_path = if let Some(path) = body.file_path.as_ref() {
        let path = path.trim();
        if path.is_empty() {
            return HttpResponse::BadRequest().json(UpgradeResponse {
                success: false,
                message: "File path cannot be empty".to_string(),
                error: Some("file_path is empty".to_string()),
            });
        }
        path.to_string()
    } else {
        // 自动查找已完成的下载任务
        let completed_task = {
            if let Ok(tasks) = DOWNLOAD_TASKS.lock() {
                tasks
                    .values()
                    .find(|t| matches!(t.status, DownloadTaskStatus::Completed))
                    .cloned()
            } else {
                None
            }
        };
        match completed_task {
            Some(task) => task.file_path,
            None => {
                return HttpResponse::BadRequest().json(UpgradeResponse {
                    success: false,
                    message: "No completed download task found".to_string(),
                    error: Some("Please start a download first or provide file_path".to_string()),
                });
            }
        }
    };

    // 3. 检查文件是否存在
    if !std::path::Path::new(&file_path).exists() {
        return HttpResponse::BadRequest().json(UpgradeResponse {
            success: false,
            message: "Update file not found".to_string(),
            error: Some(format!("File does not exist: {}", file_path)),
        });
    }

    // 4. 设置 UPDATE_STATUS 为 upgrading
    if let Ok(mut status) = UPDATE_STATUS.lock() {
        status.0 = "upgrading".to_string();
        status.1 = file_path.clone();
    }

    // 5. 向 zuti-helper 发送 upgrade 指令
    let mut stream = match UnixStream::connect(ZUTI_HELPER_SOCK) {
        Ok(s) => s,
        Err(e) => {
            return HttpResponse::InternalServerError().json(UpgradeResponse {
                success: false,
                message: "Failed to connect to zuti-helper".to_string(),
                error: Some(format!("Unix socket error: {}", e)),
            });
        }
    };

    let request_json = match serde_json::to_string(&serde_json::json!({
        "action": "upgrade",
        "file": file_path,
    })) {
        Ok(j) => j,
        Err(e) => {
            return HttpResponse::InternalServerError().json(UpgradeResponse {
                success: false,
                message: "Failed to serialize request".to_string(),
                error: Some(e.to_string()),
            });
        }
    };

    if let Err(e) = writeln!(stream, "{}", request_json) {
        return HttpResponse::InternalServerError().json(UpgradeResponse {
            success: false,
            message: "Failed to send request to zuti-helper".to_string(),
            error: Some(e.to_string()),
        });
    }

    let reader = BufReader::new(&stream);
    let response_line = match reader.lines().next() {
        Some(Ok(line)) => line,
        Some(Err(e)) => {
            return HttpResponse::InternalServerError().json(UpgradeResponse {
                success: false,
                message: "Failed to read response from zuti-helper".to_string(),
                error: Some(e.to_string()),
            });
        }
        None => {
            return HttpResponse::InternalServerError().json(UpgradeResponse {
                success: false,
                message: "No response from zuti-helper".to_string(),
                error: None,
            });
        }
    };

    let helper_resp: serde_json::Value = match serde_json::from_str(&response_line) {
        Ok(v) => v,
        Err(e) => {
            return HttpResponse::InternalServerError().json(UpgradeResponse {
                success: false,
                message: "Invalid response from zuti-helper".to_string(),
                error: Some(e.to_string()),
            });
        }
    };

    let success = helper_resp.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
    if success {
        HttpResponse::Ok().json(UpgradeResponse {
            success: true,
            message: "Upgrade command sent successfully".to_string(),
            error: None,
        })
    } else {
        let error = helper_resp.get("error").and_then(|v| v.as_str()).unwrap_or("Unknown error from zuti-helper");
        HttpResponse::InternalServerError().json(UpgradeResponse {
            success: false,
            message: "Upgrade failed".to_string(),
            error: Some(error.to_string()),
        })
    }
}

/// system/upgrade/progress API - 获取系统升级进度（需要 JWT 认证）
#[get("/system/upgrade/progress")]
pub async fn get_upgrade_progress(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    // 1. 验证 JWT token
    let _claims = match crate::utils::admin::validate_token_with_db(&req, &pool).await {
        Ok(claims) => claims,
        Err(response) => return response,
    };

    // 2. 向 zuti-helper 发送 upgrading_progress 指令
    let mut stream = match UnixStream::connect(ZUTI_HELPER_SOCK) {
        Ok(s) => s,
        Err(e) => {
            return HttpResponse::InternalServerError().json(UpgradeProgressResponse {
                success: false,
                data: None,
                error: Some(format!("Failed to connect to zuti-helper: {}", e)),
            });
        }
    };

    let request_json = match serde_json::to_string(&serde_json::json!({
        "action": "upgrading_progress",
    })) {
        Ok(j) => j,
        Err(e) => {
            return HttpResponse::InternalServerError().json(UpgradeProgressResponse {
                success: false,
                data: None,
                error: Some(format!("Failed to serialize request: {}", e)),
            });
        }
    };

    if let Err(e) = writeln!(stream, "{}", request_json) {
        return HttpResponse::InternalServerError().json(UpgradeProgressResponse {
            success: false,
            data: None,
            error: Some(format!("Failed to send request to zuti-helper: {}", e)),
        });
    }

    let reader = BufReader::new(&stream);
    let response_line = match reader.lines().next() {
        Some(Ok(line)) => line,
        Some(Err(e)) => {
            return HttpResponse::InternalServerError().json(UpgradeProgressResponse {
                success: false,
                data: None,
                error: Some(format!("Failed to read response from zuti-helper: {}", e)),
            });
        }
        None => {
            return HttpResponse::InternalServerError().json(UpgradeProgressResponse {
                success: false,
                data: None,
                error: Some("No response from zuti-helper".to_string()),
            });
        }
    };

    let helper_resp: serde_json::Value = match serde_json::from_str(&response_line) {
        Ok(v) => v,
        Err(e) => {
            return HttpResponse::InternalServerError().json(UpgradeProgressResponse {
                success: false,
                data: None,
                error: Some(format!("Invalid response from zuti-helper: {}", e)),
            });
        }
    };

    let success = helper_resp.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
    if success {
        HttpResponse::Ok().json(UpgradeProgressResponse {
            success: true,
            data: helper_resp.get("data").cloned(),
            error: None,
        })
    } else {
        let error = helper_resp.get("error").and_then(|v| v.as_str()).unwrap_or("Unknown error from zuti-helper");
        HttpResponse::InternalServerError().json(UpgradeProgressResponse {
            success: false,
            data: None,
            error: Some(error.to_string()),
        })
    }
}
