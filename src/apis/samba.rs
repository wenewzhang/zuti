use actix_web::{post, web, HttpRequest, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::process::Command;

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
#[post("/smb_public_share")]
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

    // 3. 检查目录是否存在
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

    // 4. 创建 /etc/samba/conf.d 目录
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

    // 5. 生成 Samba 配置文件内容
    let config_content = format!(
        "[public]\n    path = {}\n    browseable = {}\n    read only = {}\n    guest ok = {}\n",
        share_req.directory,
        share_req.browseable.to_lowercase(),
        share_req.read_only.to_lowercase(),
        share_req.guest_ok.to_lowercase()
    );

    // 6. 写入 public.conf 文件
    let conf_file = conf_dir.join("public.conf");
    match fs::write(&conf_file, config_content) {
        Ok(_) => {}
        Err(e) => {
            return HttpResponse::InternalServerError().json(SmbPublicShareResponse {
                success: false,
                message: "Failed to write public.conf".to_string(),
                error: Some(format!("{}", e)),
            });
        }
    }

    // 9. 尝试重新加载 Samba 服务（如果存在）
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
