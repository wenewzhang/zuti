use actix_web::{post, web, HttpRequest, HttpResponse, Responder};
use ini::Ini;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::process::Command;
use crate::utils::consts::FORBID_DIRECTORY;

// Samba public share 请求结构体
#[derive(Deserialize)]
pub struct SmbPublicShareRequest {
    pub directory: String,   // 共享目录路径
    pub browseable: String,  // yes/no
    pub read_only: String,   // yes/no
    pub guest_ok: String,    // yes/no
}

// Samba public share 响应结构体
#[derive(Serialize)]
pub struct SmbPublicShareResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

// smb_public_share API - 创建公共 Samba 共享配置（需要 JWT 认证）
#[post("/smb/public_share")]
pub async fn smb_public_share(
    req: HttpRequest,
    share_req: web::Json<SmbPublicShareRequest>,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    // 1. 验证 JWT token
    let _username = match crate::utils::verify_share_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    // 2. 验证参数值（只能是 yes 或 no）
    let valid_values = ["yes", "no"];
    
    if !valid_values.contains(&share_req.browseable.to_lowercase().as_str()) {
        return HttpResponse::BadRequest().json(SmbPublicShareResponse {
            success: false,
            message: "Invalid browseable value".to_string(),
            error: Some("browseable must be 'yes' or 'no'".to_string()),
        });
    }
    
    if !valid_values.contains(&share_req.read_only.to_lowercase().as_str()) {
        return HttpResponse::BadRequest().json(SmbPublicShareResponse {
            success: false,
            message: "Invalid read_only value".to_string(),
            error: Some("read_only must be 'yes' or 'no'".to_string()),
        });
    }
    
    if !valid_values.contains(&share_req.guest_ok.to_lowercase().as_str()) {
        return HttpResponse::BadRequest().json(SmbPublicShareResponse {
            success: false,
            message: "Invalid guest_ok value".to_string(),
            error: Some("guest_ok must be 'yes' or 'no'".to_string()),
        });
    }

    // 3. 检查是否为禁止的目录
    let directory = &share_req.directory;
    for forbid_dir in FORBID_DIRECTORY {
        if directory == *forbid_dir || directory.starts_with(&format!("{}/", forbid_dir)) || directory == "/" {
            return HttpResponse::BadRequest().json(SmbPublicShareResponse {
                success: false,
                message: format!("Forbidden directory: {}", directory),
                error: Some(format!("Directory '{}' is not allowed for sharing", forbid_dir)),
            });
        }
    }

    // 4. 检查目录是否存在
    let dir_path = Path::new(&share_req.directory);
    if !dir_path.exists() {
        // 尝试创建目录
        match fs::create_dir_all(&share_req.directory) {
            Ok(_) => {}
            Err(e) => {
                return HttpResponse::BadRequest().json(SmbPublicShareResponse {
                    success: false,
                    message: format!("Directory does not exist and cannot be created: {}", share_req.directory),
                    error: Some(format!("Failed to create directory: {}", e)),
                });
            }
        }
    }

    // Set directory permissions: remove write for others if read_only is yes, otherwise add
    let chmod_arg = if share_req.read_only.to_lowercase() == "yes" {
        "o-w"
    } else {
        "o+w"
    };
    if let Err(e) = Command::new("chmod").arg(chmod_arg).arg(&share_req.directory).output() {
        return HttpResponse::InternalServerError().json(SmbPublicShareResponse {
            success: false,
            message: format!("Failed to set directory permissions: {}", share_req.directory),
            error: Some(format!("{}", e)),
        });
    }

    // 5. 创建 /etc/samba/conf.d 目录
    let conf_dir = Path::new("/etc/samba/conf.d");
    if !conf_dir.exists() {
        match fs::create_dir_all(conf_dir) {
            Ok(_) => {}
            Err(e) => {
                return HttpResponse::InternalServerError().json(SmbPublicShareResponse {
                    success: false,
                    message: "Failed to create /etc/samba/conf.d directory".to_string(),
                    error: Some(format!("{}", e)),
                });
            }
        }
    }

    // 6. 从目录路径提取共享名（取最后的路径组件）
    let share_name = dir_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("public");

    // 7. 使用 rust-ini 写入 public.conf 文件
    let conf_file = conf_dir.join("public.conf");

    // 加载或创建配置文件
    let mut ini = if conf_file.exists() {
        match Ini::load_from_file(&conf_file) {
            Ok(ini) => ini,
            Err(e) => {
                return HttpResponse::InternalServerError().json(SmbPublicShareResponse {
                    success: false,
                    message: "Failed to parse public.conf".to_string(),
                    error: Some(format!("{}", e)),
                });
            }
        }
    } else {
        Ini::new()
    };

    // 设置或更新 share section（存在则修改，不存在则创建）
    ini.with_section(Some(share_name.to_string()))
        .set("path", &share_req.directory)
        .set("browseable", &share_req.browseable.to_lowercase())
        .set("read only", &share_req.read_only.to_lowercase())
        .set("guest ok", &share_req.guest_ok.to_lowercase());

    // 写入文件
    if let Err(e) = ini.write_to_file(&conf_file) {
        return HttpResponse::InternalServerError().json(SmbPublicShareResponse {
            success: false,
            message: "Failed to write public.conf".to_string(),
            error: Some(format!("{}", e)),
        });
    }

    // 8. 尝试重新加载 Samba 服务（如果存在）
    let _ = Command::new("systemctl")
        .args(["restart", "smbd"])
        .output();

    HttpResponse::Ok().json(SmbPublicShareResponse {
        success: true,
        message: format!(
            "Samba public share configured successfully. Config saved to {}",
            conf_file.display()
        ),
        error: None,
    })
}
