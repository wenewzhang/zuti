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

// Samba auth share 请求结构体
#[derive(Deserialize)]
pub struct SmbAuthShareRequest {
    pub directory: String,       // 共享目录路径
    pub browseable: String,      // yes/no
    pub read_only: String,       // yes/no
    pub valid_users: Vec<String>, // 允许访问的用户列表
    pub write_list: Vec<String>,  // 有写权限的用户列表
}

// Samba add user 请求结构体
#[derive(Deserialize)]
pub struct SmbAddUserRequest {
    pub username: String,    // 用户名
    pub password: String,    // 密码
}

// Samba add user 响应结构体
#[derive(Serialize)]
pub struct SmbAddUserResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

// Samba delete user 请求结构体
#[derive(Deserialize)]
pub struct SmbDeleteUserRequest {
    pub username: String,    // 用户名
}

// Samba delete user 响应结构体
#[derive(Serialize)]
pub struct SmbDeleteUserResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

// Samba user 信息
#[derive(Serialize)]
pub struct SmbUserInfo {
    pub username: String,
    pub uid: String,
    pub description: String,
}

// Samba list users 响应结构体
#[derive(Serialize)]
pub struct SmbListUsersResponse {
    pub success: bool,
    pub users: Vec<SmbUserInfo>,
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

// smb_auth_share API - 创建认证 Samba 共享配置（需要 JWT 认证）
#[post("/smb/auth_share")]
pub async fn smb_auth_share(
    req: HttpRequest,
    share_req: web::Json<SmbAuthShareRequest>,
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

    // 3. 验证 valid_users 不为空
    if share_req.valid_users.is_empty() {
        return HttpResponse::BadRequest().json(SmbPublicShareResponse {
            success: false,
            message: "Invalid valid_users".to_string(),
            error: Some("valid_users cannot be empty".to_string()),
        });
    }

    // 4. 检查是否为禁止的目录
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

    // 5. 检查目录是否存在
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
        "a-w"
    } else {
        "a+w"
    };
    if let Err(e) = Command::new("chmod").arg(chmod_arg).arg(&share_req.directory).output() {
        return HttpResponse::InternalServerError().json(SmbPublicShareResponse {
            success: false,
            message: format!("Failed to set directory permissions: {}", share_req.directory),
            error: Some(format!("{}", e)),
        });
    }

    let conf_dir = Path::new("/etc/samba/conf.d");
    // 7. 从目录路径提取共享名（取最后的路径组件）
    let share_name = dir_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("private");

    // 8. 使用 rust-ini 写入 private.conf 文件
    let conf_file = conf_dir.join("private.conf");

    // 加载或创建配置文件
    let mut ini = if conf_file.exists() {
        match Ini::load_from_file(&conf_file) {
            Ok(ini) => ini,
            Err(e) => {
                return HttpResponse::InternalServerError().json(SmbPublicShareResponse {
                    success: false,
                    message: "Failed to parse private.conf".to_string(),
                    error: Some(format!("{}", e)),
                });
            }
        }
    } else {
        Ini::new()
    };

    // 将用户列表转换为空格分隔的字符串
    let valid_users_str = share_req.valid_users.join(" ");
    let write_list_str = if share_req.write_list.is_empty() {
        None
    } else {
        Some(share_req.write_list.join(" "))
    };

    // 设置或更新 share section（存在则修改，不存在则创建）
    let mut section = ini.with_section(Some(share_name.to_string()));
    section
        .set("path", &share_req.directory)
        .set("browseable", &share_req.browseable.to_lowercase())
        .set("read only", &share_req.read_only.to_lowercase())
        .set("valid users", &valid_users_str);
    
    // 如果 write_list 不为空，则添加 write list 配置
    if let Some(write_list) = write_list_str {
        section.set("write list", &write_list);
    }

    // 写入文件
    if let Err(e) = ini.write_to_file(&conf_file) {
        return HttpResponse::InternalServerError().json(SmbPublicShareResponse {
            success: false,
            message: "Failed to write private.conf".to_string(),
            error: Some(format!("{}", e)),
        });
    }

    // 9. 尝试重新加载 Samba 服务（如果存在）
    let _ = Command::new("systemctl")
        .args(["restart", "smbd"])
        .output();

    HttpResponse::Ok().json(SmbPublicShareResponse {
        success: true,
        message: format!(
            "Samba auth share configured successfully. Config saved to {}",
            conf_file.display()
        ),
        error: None,
    })
}

// smb_add_user API - 添加 Samba 用户（需要 JWT 认证）
// 如果 Linux 系统用户不存在，会自动创建
#[post("/smb/add_user")]
pub async fn smb_add_user(
    req: HttpRequest,
    user_req: web::Json<SmbAddUserRequest>,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    // 1. 验证 JWT token
    let _username = match crate::utils::verify_share_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let username = &user_req.username;
    let password = &user_req.password;

    // 2. 验证用户名合法性（只允许字母数字、下划线）
    if !username.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return HttpResponse::BadRequest().json(SmbAddUserResponse {
            success: false,
            message: "Invalid username format".to_string(),
            error: Some("Username must contain only alphanumeric characters or underscores".to_string()),
        });
    }

    // 3. 验证密码长度
    if password.len() < 4 {
        return HttpResponse::BadRequest().json(SmbAddUserResponse {
            success: false,
            message: "Password too short".to_string(),
            error: Some("Password must be at least 4 characters".to_string()),
        });
    }

    // 4. 检查系统用户是否存在，不存在则创建
    let output = Command::new("id")
        .arg(username)
        .output();

    let user_exists = match output {
        Ok(result) => result.status.success(),
        Err(_) => false,
    };

    if !user_exists {
        // 创建系统用户（无登录权限）
        let output = Command::new("useradd")
            .args(["-M", "-s", "/usr/sbin/nologin", username])
            .output();

        match output {
            Ok(result) => {
                if !result.status.success() {
                    let stderr = String::from_utf8_lossy(&result.stderr);
                    return HttpResponse::InternalServerError().json(SmbAddUserResponse {
                        success: false,
                        message: "Failed to create system user".to_string(),
                        error: Some(format!("{}", stderr)),
                    });
                }
            }
            Err(e) => {
                return HttpResponse::InternalServerError().json(SmbAddUserResponse {
                    success: false,
                    message: "Failed to create system user".to_string(),
                    error: Some(format!("{}", e)),
                });
            }
        }
    }

    // 5. 添加到 Samba 用户（smbpasswd -a）
    // 使用 echo 呼结合 smbpasswd -a -s 来非交互式添加用户
    let smbpasswd_input = format!("{}\n{}\n", password, password);
    let output = Command::new("smbpasswd")
        .args(["-a", "-s", username])
        .stdin(std::process::Stdio::piped())
        .spawn();

    match output {
        Ok(mut child) => {
            use std::io::Write;
            if let Some(stdin) = child.stdin.as_mut() {
                if let Err(e) = stdin.write_all(smbpasswd_input.as_bytes()) {
                    return HttpResponse::InternalServerError().json(SmbAddUserResponse {
                        success: false,
                        message: "Failed to add samba user".to_string(),
                        error: Some(format!("{}", e)),
                    });
                }
            }
            let result = child.wait_with_output();
            match result {
                Ok(output) => {
                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        return HttpResponse::InternalServerError().json(SmbAddUserResponse {
                            success: false,
                            message: "Failed to add samba user".to_string(),
                            error: Some(format!("{}", stderr)),
                        });
                    }
                }
                Err(e) => {
                    return HttpResponse::InternalServerError().json(SmbAddUserResponse {
                        success: false,
                        message: "Failed to add samba user".to_string(),
                        error: Some(format!("{}", e)),
                    });
                }
            }
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(SmbAddUserResponse {
                success: false,
                message: "Failed to add samba user".to_string(),
                error: Some(format!("{}", e)),
            });
        }
    }

    HttpResponse::Ok().json(SmbAddUserResponse {
        success: true,
        message: format!("Samba user '{}' created/updated successfully", username),
        error: None,
    })
}

// smb_delete_user API - 删除 Samba 用户（需要 JWT 认证）
#[post("/smb/delete_user")]
pub async fn smb_delete_user(
    req: HttpRequest,
    user_req: web::Json<SmbDeleteUserRequest>,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    // 1. 验证 JWT token
    let _username = match crate::utils::verify_share_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let username = &user_req.username;

    // 2. 验证用户名合法性（只允许字母数字、下划线）
    if !username.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return HttpResponse::BadRequest().json(SmbDeleteUserResponse {
            success: false,
            message: "Invalid username format".to_string(),
            error: Some("Username must contain only alphanumeric characters or underscores".to_string()),
        });
    }

    // 3. 删除 Samba 用户（smbpasswd -x）
    let output = Command::new("smbpasswd")
        .args(["-x", username])
        .output();

    match output {
        Ok(result) => {
            if !result.status.success() {
                let stderr = String::from_utf8_lossy(&result.stderr);
                return HttpResponse::InternalServerError().json(SmbDeleteUserResponse {
                    success: false,
                    message: "Failed to delete samba user".to_string(),
                    error: Some(format!("{}", stderr)),
                });
            }
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(SmbDeleteUserResponse {
                success: false,
                message: "Failed to delete samba user".to_string(),
                error: Some(format!("{}", e)),
            });
        }
    }

    HttpResponse::Ok().json(SmbDeleteUserResponse {
        success: true,
        message: format!("Samba user '{}' deleted successfully", username),
        error: None,
    })
}

// smb_list_users API - 列出所有 Samba 用户（需要 JWT 认证）
#[post("/smb/list_users")]
pub async fn smb_list_users(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    // 1. 验证 JWT token
    let _username = match crate::utils::verify_share_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    // 2. 获取 Samba 用户列表（pdbedit -L）
    let output = Command::new("pdbedit")
        .args(["-L"])
        .output();

    let users = match output {
        Ok(result) => {
            if !result.status.success() {
                let stderr = String::from_utf8_lossy(&result.stderr);
                return HttpResponse::InternalServerError().json(SmbListUsersResponse {
                    success: false,
                    users: vec![],
                    message: "Failed to list samba users".to_string(),
                    error: Some(format!("{}", stderr)),
                });
            }
            
            // 解析 pdbedit 输出
            // 格式: username:uid:description
            let stdout = String::from_utf8_lossy(&result.stdout);
            let mut user_list = Vec::new();
            
            for line in stdout.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                
                let parts: Vec<&str> = line.split(':').collect();
                if parts.len() >= 2 {
                    user_list.push(SmbUserInfo {
                        username: parts[0].to_string(),
                        uid: parts[1].to_string(),
                        description: parts.get(2).unwrap_or(&"").to_string(),
                    });
                }
            }
            
            user_list
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(SmbListUsersResponse {
                success: false,
                users: vec![],
                message: "Failed to list samba users".to_string(),
                error: Some(format!("{}", e)),
            });
        }
    };

    HttpResponse::Ok().json(SmbListUsersResponse {
        success: true,
        users,
        message: "Samba users listed successfully".to_string(),
        error: None,
    })
}
