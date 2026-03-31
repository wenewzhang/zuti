use actix_web::{post, web, HttpRequest, HttpResponse, Responder};
use diesel::prelude::*;
use ini::Ini;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::process::Command;
use crate::models::User;
use crate::schema::users as users_schema;
use crate::utils::consts::{FORBID_DIRECTORY, FORBIDDEN_USERNAME};
use crate::utils::conf::merge_samba_configs;

/// 检查指定的用户名是否存在于 Samba 用户列表中（通过 pdbedit -L）
/// 
/// # Arguments
/// * `username` - 要检查的用户名
/// 
/// # Returns
/// * `Ok(true)` - 用户存在
/// * `Ok(false)` - 用户不存在
/// * `Err(String)` - 执行命令失败
fn check_samba_user_exists(username: &str) -> Result<bool, String> {
    let output = Command::new("pdbedit")
        .args(["-L"])
        .output();

    match output {
        Ok(result) => {
            if result.status.success() {
                let stdout = String::from_utf8_lossy(&result.stdout);
                let exists = stdout.lines().any(|line| {
                    let parts: Vec<&str> = line.split(':').collect();
                    !parts.is_empty() && parts[0] == username
                });
                Ok(exists)
            } else {
                let stderr = String::from_utf8_lossy(&result.stderr);
                Err(format!("pdbedit command failed: {}", stderr))
            }
        }
        Err(e) => Err(format!("Failed to execute pdbedit: {}", e)),
    }
}

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

// Samba change password 请求结构体
#[derive(Deserialize)]
pub struct SmbChangePasswordRequest {
    pub username: String,    // 要修改密码的用户名
    pub new_password: String, // 新密码
}

// Samba change password 响应结构体
#[derive(Serialize)]
pub struct SmbChangePasswordResponse {
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
        .and_then(|n| n.to_str())
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

    // 8. 合并配置文件并重新加载 Samba 服务
    if let Err(e) = merge_samba_configs() {
        return HttpResponse::InternalServerError().json(SmbPublicShareResponse {
            success: false,
            message: "Failed to merge samba configs".to_string(),
            error: Some(e),
        });
    }
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
        "a+w"
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
        .and_then(|n| n.to_str())
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
        .set("valid users", &valid_users_str)
        .set("create mask", "0644")
        .set("directory mask", "0755");
    
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

    // 9. 合并配置文件并重新加载 Samba 服务
    if let Err(e) = merge_samba_configs() {
        return HttpResponse::InternalServerError().json(SmbPublicShareResponse {
            success: false,
            message: "Failed to merge samba configs".to_string(),
            error: Some(e),
        });
    }
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
    let user_password = &user_req.password;

    // 2. 验证用户名合法性（只允许字母数字、下划线）
    if !username.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return HttpResponse::BadRequest().json(SmbAddUserResponse {
            success: false,
            message: "Invalid username format".to_string(),
            error: Some("Username must contain only alphanumeric characters or underscores".to_string()),
        });
    }

    // 3. 禁止使用 root 作为用户名
    if username == FORBIDDEN_USERNAME {
        return HttpResponse::Forbidden().json(SmbAddUserResponse {
            success: false,
            message: "Username 'root' is not allowed".to_string(),
            error: Some("Cannot add root user".to_string()),
        });
    }

    // 4. 验证密码长度
    if user_password.len() < 4 {
        return HttpResponse::BadRequest().json(SmbAddUserResponse {
            success: false,
            message: "Password too short".to_string(),
            error: Some("Password must be at least 4 characters".to_string()),
        });
    }

    // 5. 检查系统用户是否存在，不存在则创建
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

    // 6. 添加到 Samba 用户（smbpasswd -a）
    // 使用 echo 呼结合 smbpasswd -a -s 来非交互式添加用户
    let smbpasswd_input = format!("{}\n{}\n", user_password, user_password);
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

    // 3. 检查数据库中是否存在该用户
    let username_clone = username.clone();
    let pool_clone = pool.get_ref().clone();
    let db_check_result: Result<Option<User>, String> = web::block(move || {
        let mut conn = pool_clone.get().map_err(|e| format!("Database connection error: {}", e))?;
        let user_result: Option<User> = users_schema::table
            .filter(users_schema::name.eq(&username_clone))
            .first::<User>(&mut conn)
            .optional()
            .map_err(|e| format!("Database query error: {}", e))?;
        Ok(user_result)
    })
    .await
    .unwrap();

    match db_check_result {
        Ok(Some(_user)) => {
            return HttpResponse::Forbidden().json(SmbDeleteUserResponse {
                success: false,
                message: format!("Cannot delete user '{}' from database", username),
                error: Some("Users in database cannot be deleted via this API".to_string()),
            });
        }
        Ok(None) => {
            // 数据库中不存在，继续检查 Samba 用户列表
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(SmbDeleteUserResponse {
                success: false,
                message: "Failed to check database".to_string(),
                error: Some(e),
            });
        }
    }

    // 4. 检查用户是否存在于 Samba 用户列表中
    match check_samba_user_exists(username) {
        Ok(true) => {}
        Ok(false) => {
            return HttpResponse::BadRequest().json(SmbDeleteUserResponse {
                success: false,
                message: format!("Samba user '{}' does not exist", username),
                error: Some("User not found in Samba user list".to_string()),
            });
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(SmbDeleteUserResponse {
                success: false,
                message: "Failed to check Samba user list".to_string(),
                error: Some(e),
            });
        }
    }

    // 5. 删除 Samba 用户（smbpasswd -x）
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

    // 6. 删除 Linux 系统用户（userdel）
    let output = Command::new("userdel")
        .arg(username)
        .output();

    match output {
        Ok(result) => {
            if !result.status.success() {
                let stderr = String::from_utf8_lossy(&result.stderr);
                // 如果用户不存在，跳过错误（因为 Samba 用户已经删除了）
                if !stderr.contains("does not exist") {
                    return HttpResponse::InternalServerError().json(SmbDeleteUserResponse {
                        success: false,
                        message: "Failed to delete system user".to_string(),
                        error: Some(format!("{}", stderr)),
                    });
                }
            }
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(SmbDeleteUserResponse {
                success: false,
                message: "Failed to delete system user".to_string(),
                error: Some(format!("{}", e)),
            });
        }
    }

    HttpResponse::Ok().json(SmbDeleteUserResponse {
        success: true,
        message: format!("Samba user '{}' and system user deleted successfully", username),
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
    let _claims = match crate::utils::admin::validate_token_with_db(&req, &pool).await {
        Ok(claims) => claims,
        Err(response) => return response,
    };

    // 2. 获取 Samba 用户列表（pdbedit -L）
    let output = Command::new("pdbedit")
        .args(["-L"])
        .output();

    let user_list = match output {
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
        users: user_list,
        message: "Samba users listed successfully".to_string(),
        error: None,
    })
}

// smb_change_passwd API - 修改 Samba 用户密码（需要 admin 权限）
#[post("/smb/change_passwd")]
pub async fn smb_change_passwd(
    req: HttpRequest,
    user_req: web::Json<SmbChangePasswordRequest>,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    // 1. 验证 JWT token 并检查 admin 权限
    let _admin_username = match crate::utils::verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let username = &user_req.username;
    let new_password = &user_req.new_password;

    // 2. 验证用户名不为空
    if username.is_empty() {
        return HttpResponse::BadRequest().json(SmbChangePasswordResponse {
            success: false,
            message: "Username cannot be empty".to_string(),
            error: Some("username is required".to_string()),
        });
    }

    // 3. 验证用户名合法性（只允许字母数字、下划线）
    if !username.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return HttpResponse::BadRequest().json(SmbChangePasswordResponse {
            success: false,
            message: "Invalid username format".to_string(),
            error: Some("Username must contain only alphanumeric characters or underscores".to_string()),
        });
    }

    // 4. 验证密码长度
    if new_password.len() < 4 {
        return HttpResponse::BadRequest().json(SmbChangePasswordResponse {
            success: false,
            message: "Password too short".to_string(),
            error: Some("Password must be at least 4 characters".to_string()),
        });
    }

    // 5. 检查用户是否存在于 Samba 用户列表中
    match check_samba_user_exists(username) {
        Ok(true) => {}
        Ok(false) => {
            return HttpResponse::BadRequest().json(SmbChangePasswordResponse {
                success: false,
                message: format!("Samba user '{}' does not exist", username),
                error: Some("User not found in Samba user list".to_string()),
            });
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(SmbChangePasswordResponse {
                success: false,
                message: "Failed to check Samba user list".to_string(),
                error: Some(e),
            });
        }
    }

    // 6. 修改 Samba 用户密码（smbpasswd -s）
    let smbpasswd_input = format!("{}\n{}\n", new_password, new_password);
    let output = Command::new("smbpasswd")
        .args(["-s", username])
        .stdin(std::process::Stdio::piped())
        .spawn();

    match output {
        Ok(mut child) => {
            use std::io::Write;
            if let Some(stdin) = child.stdin.as_mut() {
                if let Err(e) = stdin.write_all(smbpasswd_input.as_bytes()) {
                    return HttpResponse::InternalServerError().json(SmbChangePasswordResponse {
                        success: false,
                        message: "Failed to change samba password".to_string(),
                        error: Some(format!("{}", e)),
                    });
                }
            }
            let result = child.wait_with_output();
            match result {
                Ok(output) => {
                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        return HttpResponse::InternalServerError().json(SmbChangePasswordResponse {
                            success: false,
                            message: "Failed to change samba password".to_string(),
                            error: Some(format!("{}", stderr)),
                        });
                    }
                }
                Err(e) => {
                    return HttpResponse::InternalServerError().json(SmbChangePasswordResponse {
                        success: false,
                        message: "Failed to change samba password".to_string(),
                        error: Some(format!("{}", e)),
                    });
                }
            }
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(SmbChangePasswordResponse {
                success: false,
                message: "Failed to change samba password".to_string(),
                error: Some(format!("{}", e)),
            });
        }
    }

    // 7. 同时修改 Linux 系统用户密码（保持同步）
    let passwd_input = format!("{}:{}", username, new_password);
    let output = Command::new("chpasswd")
        .stdin(std::process::Stdio::piped())
        .spawn();

    match output {
        Ok(mut child) => {
            use std::io::Write;
            if let Some(stdin) = child.stdin.as_mut() {
                let _ = stdin.write_all(passwd_input.as_bytes());
            }
            let _ = child.wait_with_output();
        }
        Err(_) => {
            // Linux 密码修改失败不返回错误，只记录 Samba 密码修改成功
        }
    }

    HttpResponse::Ok().json(SmbChangePasswordResponse {
        success: true,
        message: format!("Password for user '{}' changed successfully", username),
        error: None,
    })
}

// ZFS Pool 信息
#[derive(Serialize)]
pub struct ZfsPoolInfo {
    pub name: String,
}

// List ZFS pools 响应结构体
#[derive(Serialize)]
pub struct ListZfsPoolsResponse {
    pub success: bool,
    pub pools: Vec<ZfsPoolInfo>,
    pub message: String,
    pub error: Option<String>,
}

// list_zfs_pools API - 获取所有 ZFS pool 名称列表（需要 JWT 认证）
#[post("/smb/list_zfs_pools")]
pub async fn list_zfs_pools(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    // 1. 验证 JWT token
    let _claims = match crate::utils::admin::validate_token_with_db(&req, &pool).await {
        Ok(claims) => claims,
        Err(response) => return response,
    };

    // 2. 执行 zpool list -H -o name 命令获取所有 pool 名称
    let output = Command::new("zpool")
        .args(["list", "-H", "-o", "name"])
        .output();

    let pool_list = match output {
        Ok(result) => {
            if !result.status.success() {
                let stderr = String::from_utf8_lossy(&result.stderr);
                return HttpResponse::InternalServerError().json(ListZfsPoolsResponse {
                    success: false,
                    pools: vec![],
                    message: "Failed to list ZFS pools".to_string(),
                    error: Some(format!("{}", stderr)),
                });
            }

            // 解析 zpool list 输出，每行一个 pool 名称
            let stdout = String::from_utf8_lossy(&result.stdout);
            let mut pools = Vec::new();

            for line in stdout.lines() {
                let pool_name = line.trim();
                if !pool_name.is_empty() {
                    pools.push(ZfsPoolInfo {
                        name: pool_name.to_string(),
                    });
                }
            }

            pools
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(ListZfsPoolsResponse {
                success: false,
                pools: vec![],
                message: "Failed to list ZFS pools".to_string(),
                error: Some(format!("{}", e)),
            });
        }
    };

    HttpResponse::Ok().json(ListZfsPoolsResponse {
        success: true,
        pools: pool_list,
        message: "ZFS pools listed successfully".to_string(),
        error: None,
    })
}

// Create ZFS Share 请求结构体
#[derive(Deserialize)]
pub struct CreateZfsShareRequest {
    pub share_name: String,      // 共享名称（dataset 名称）
    pub pool_name: String,       // ZFS pool 名称
    pub quota: String,           // 容量限制，如 "10G", "100M", "1T"
    pub samba_user: String,      // Samba 用户名（用于设置目录所有者）
}

// Create ZFS Share 响应结构体
#[derive(Serialize)]
pub struct CreateZfsShareResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

// create_zfs_share API - 创建 ZFS 共享数据集（需要 JWT 认证）
// 执行步骤：
// 1. zfs create -o sharesmb=on -o compression=lz4 <pool>/<share_name>
// 2. zfs set quota=<quota> <pool>/<share_name>
// 3. zfs set mountpoint=<mountpoint> <pool>/<share_name>
// 4. chown -R <samba_user>:<samba_user> <mountpoint>
#[post("/smb/create_zfs_share")]
pub async fn create_zfs_share(
    req: HttpRequest,
    share_req: web::Json<CreateZfsShareRequest>,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    // 1. 验证 JWT token
    let _username = match crate::utils::verify_share_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let share_name = &share_req.share_name;
    let pool_name = &share_req.pool_name;
    let quota = &share_req.quota;
    let samba_user = &share_req.samba_user;
    // 自动生成 mountpoint: /pool/share_name
    let mountpoint = format!("/{}/{}", pool_name, share_name);

    // 2. 验证 share_name 合法性（只允许字母数字、下划线、连字符）
    if !share_name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
        return HttpResponse::BadRequest().json(CreateZfsShareResponse {
            success: false,
            message: "Invalid share name format".to_string(),
            error: Some("Share name must contain only alphanumeric characters, underscores, or hyphens".to_string()),
        });
    }

    // 3. 验证 pool_name 不为空
    if pool_name.is_empty() {
        return HttpResponse::BadRequest().json(CreateZfsShareResponse {
            success: false,
            message: "Pool name cannot be empty".to_string(),
            error: Some("pool_name is required".to_string()),
        });
    }

    // 4. 验证 quota 格式（简单检查是否以 G, M, T, P 等结尾或纯数字）
    if !quota.is_empty() {
        let quota_upper = quota.to_uppercase();
        let valid_suffixes = ['M', 'G', 'T', 'P', 'E'];
        let is_valid = quota_upper.chars().all(|c| c.is_ascii_digit() || valid_suffixes.contains(&c));
        if !is_valid {
            return HttpResponse::BadRequest().json(CreateZfsShareResponse {
                success: false,
                message: "Invalid quota format".to_string(),
                error: Some("Quota must be a number optionally followed by M, G, T, P, or E".to_string()),
            });
        }
    }

    // 5. 验证 samba_user 不为空，并且存在于 Samba 用户列表中
    if samba_user.is_empty() {
        return HttpResponse::BadRequest().json(CreateZfsShareResponse {
            success: false,
            message: "Samba user cannot be empty".to_string(),
            error: Some("samba_user is required".to_string()),
        });
    }

    // 5.1 检查 samba_user 是否存在于 pdbedit 用户列表中
    match check_samba_user_exists(samba_user) {
        Ok(true) => {}
        Ok(false) => {
            return HttpResponse::BadRequest().json(CreateZfsShareResponse {
                success: false,
                message: format!("Samba user '{}' does not exist", samba_user),
                error: Some("User not found in Samba user list (pdbedit)".to_string()),
            });
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(CreateZfsShareResponse {
                success: false,
                message: "Failed to check Samba user list".to_string(),
                error: Some(e),
            });
        }
    }

    let dataset = format!("{}/{}", pool_name, share_name);
    // mountpoint 自动生成: /pool/share_name

    // Step 0: 检查 dataset 是否已存在
    let check_output = Command::new("zfs")
        .args(["list", "-H", "-o", "name", &dataset])
        .output();

    let dataset_exists = match check_output {
        Ok(result) => result.status.success(),
        Err(_) => false,
    };

    // Step 1: 如果 dataset 不存在则创建，已存在则设置 sharesmb=on
    if !dataset_exists {
        // 创建新 dataset
        let output = Command::new("zfs")
            .args([
                "create",
                "-o", "sharesmb=on",
                "-o", "compression=lz4",
                &dataset,
            ])
            .output();

        match output {
            Ok(result) => {
                if !result.status.success() {
                    let stderr = String::from_utf8_lossy(&result.stderr);
                    return HttpResponse::InternalServerError().json(CreateZfsShareResponse {
                        success: false,
                        message: format!("Failed to create ZFS dataset: {}", dataset),
                        error: Some(format!("{}", stderr)),
                    });
                }
            }
            Err(e) => {
                return HttpResponse::InternalServerError().json(CreateZfsShareResponse {
                    success: false,
                    message: format!("Failed to execute zfs create: {}", dataset),
                    error: Some(format!("{}", e)),
                });
            }
        }
    } else {
        // Dataset 已存在，设置 sharesmb=on
        let output = Command::new("zfs")
            .args(["set", "sharesmb=on", &dataset])
            .output();

        match output {
            Ok(result) => {
                if !result.status.success() {
                    let stderr = String::from_utf8_lossy(&result.stderr);
                    return HttpResponse::InternalServerError().json(CreateZfsShareResponse {
                        success: false,
                        message: format!("Failed to set sharesmb=on for dataset: {}", dataset),
                        error: Some(format!("{}", stderr)),
                    });
                }
            }
            Err(e) => {
                return HttpResponse::InternalServerError().json(CreateZfsShareResponse {
                    success: false,
                    message: format!("Failed to execute zfs set sharesmb=on: {}", dataset),
                    error: Some(format!("{}", e)),
                });
            }
        }
    }

    // Step 2: zfs set quota=<quota> <pool>/<share_name>
    let output = Command::new("zfs")
        .args([
            "set",
            &format!("quota={}", quota),
            &dataset,
        ])
        .output();

    match output {
        Ok(result) => {
            if !result.status.success() {
                let stderr = String::from_utf8_lossy(&result.stderr);
                // 尝试删除已创建的 dataset
                let _ = Command::new("zfs").args(["destroy", &dataset]).output();
                return HttpResponse::InternalServerError().json(CreateZfsShareResponse {
                    success: false,
                    message: format!("Failed to set quota: {}", quota),
                    error: Some(format!("{}", stderr)),
                });
            }
        }
        Err(e) => {
            // 尝试删除已创建的 dataset
            let _ = Command::new("zfs").args(["destroy", &dataset]).output();
            return HttpResponse::InternalServerError().json(CreateZfsShareResponse {
                success: false,
                message: format!("Failed to execute zfs set quota: {}", quota),
                error: Some(format!("{}", e)),
            });
        }
    }

    // Step 3: zfs set mountpoint=<mountpoint> <pool>/<share_name>
    let output = Command::new("zfs")
        .args([
            "set",
            &format!("mountpoint={}", &mountpoint),
            &dataset,
        ])
        .output();

    match output {
        Ok(result) => {
            if !result.status.success() {
                let stderr = String::from_utf8_lossy(&result.stderr);
                // 尝试删除已创建的 dataset
                let _ = Command::new("zfs").args(["destroy", &dataset]).output();
                return HttpResponse::InternalServerError().json(CreateZfsShareResponse {
                    success: false,
                    message: format!("Failed to set mountpoint: {}", mountpoint),
                    error: Some(format!("{}", stderr)),
                });
            }
        }
        Err(e) => {
            // 尝试删除已创建的 dataset
            let _ = Command::new("zfs").args(["destroy", &dataset]).output();
            return HttpResponse::InternalServerError().json(CreateZfsShareResponse {
                success: false,
                message: format!("Failed to execute zfs set mountpoint: {}", mountpoint),
                error: Some(format!("{}", e)),
            });
        }
    }

    // Step 4: chown -R <samba_user>:<samba_user> <mountpoint>
    let output = Command::new("chown")
        .args([
            "-R",
            &format!("{}:{}", samba_user, samba_user),
            &mountpoint,
        ])
        .output();

    match output {
        Ok(result) => {
            if !result.status.success() {
                let stderr = String::from_utf8_lossy(&result.stderr);
                // 尝试删除已创建的 dataset
                let _ = Command::new("zfs").args(["destroy", &dataset]).output();
                return HttpResponse::InternalServerError().json(CreateZfsShareResponse {
                    success: false,
                    message: format!("Failed to set ownership for user: {}", samba_user),
                    error: Some(format!("{}", stderr)),
                });
            }
        }
        Err(e) => {
            // 尝试删除已创建的 dataset
            let _ = Command::new("zfs").args(["destroy", &dataset]).output();
            return HttpResponse::InternalServerError().json(CreateZfsShareResponse {
                success: false,
                message: format!("Failed to execute chown for user: {}", samba_user),
                error: Some(format!("{}", e)),
            });
        }
    }

    HttpResponse::Ok().json(CreateZfsShareResponse {
        success: true,
        message: format!(
            "ZFS share '{}' created successfully on pool '{}', mounted at '{}' with quota '{}'",
            share_name, pool_name, mountpoint, quota
        ),
        error: None,
    })
}

// ZFS SMB Share 信息
#[derive(Serialize)]
pub struct ZfsSmbShareInfo {
    pub dataset: String,  // 数据集名称，如 "tank/myshare"
}

// List ZFS SMB Shares 响应结构体
#[derive(Serialize)]
pub struct ListZfsSmbSharesResponse {
    pub success: bool,
    pub shares: Vec<ZfsSmbShareInfo>,
    pub message: String,
    pub error: Option<String>,
}

// list_zfs_shares API - 获取所有启用了 SMB 共享的 ZFS dataset 列表（需要 JWT 认证）
// 执行命令: zfs get -H -o name,value sharesmb | grep "on$"
#[post("/smb/list_zfs_shares")]
pub async fn list_zfs_shares(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    // 1. 验证 JWT token
    let _claims = match crate::utils::admin::validate_token_with_db(&req, &pool).await {
        Ok(claims) => claims,
        Err(response) => return response,
    };

    // 2. 执行 zfs get 命令获取所有 dataset 的 sharesmb 属性
    let output = Command::new("zfs")
        .args([
            "get",
            "-H",
            "-o", "name,value",
            "sharesmb",
        ])
        .output();

    let shares = match output {
        Ok(result) => {
            if !result.status.success() {
                let stderr = String::from_utf8_lossy(&result.stderr);
                return HttpResponse::InternalServerError().json(ListZfsSmbSharesResponse {
                    success: false,
                    shares: vec![],
                    message: "Failed to list ZFS shares".to_string(),
                    error: Some(format!("{}", stderr)),
                });
            }

            // 解析输出，格式: dataset_name\ton
            // 只保留 value 为 "on" 的 dataset
            let stdout = String::from_utf8_lossy(&result.stdout);
            let mut share_list = Vec::new();

            for line in stdout.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 && parts[1] == "on" {
                    share_list.push(ZfsSmbShareInfo {
                        dataset: parts[0].to_string(),
                    });
                }
            }

            share_list
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(ListZfsSmbSharesResponse {
                success: false,
                shares: vec![],
                message: "Failed to execute zfs get command".to_string(),
                error: Some(format!("{}", e)),
            });
        }
    };

    HttpResponse::Ok().json(ListZfsSmbSharesResponse {
        success: true,
        shares,
        message: "ZFS SMB shares listed successfully".to_string(),
        error: None,
    })
}

// Directory Share 信息
#[derive(Serialize)]
pub struct DirShareInfo {
    pub name: String,       // 共享名称（section title）
}

// List Directory Shares 响应结构体
#[derive(Serialize)]
pub struct ListDirSharesResponse {
    pub success: bool,
    pub shares: Vec<DirShareInfo>,
    pub message: String,
    pub error: Option<String>,
}

// list_dir_shares API - 读取 /etc/samba/conf.d/public.conf 和 private.conf 中的所有共享名称（需要 JWT 认证）
#[post("/smb/list_dir_shares")]
pub async fn list_dir_shares(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    // 1. 验证 JWT token
    let _claims = match crate::utils::admin::validate_token_with_db(&req, &pool).await {
        Ok(claims) => claims,
        Err(response) => return response,
    };

    let mut shares = Vec::new();
    let conf_dir = Path::new("/etc/samba/conf.d");

    // 2. 读取 public.conf
    let public_conf = conf_dir.join("public.conf");
    if public_conf.exists() {
        match Ini::load_from_file(&public_conf) {
            Ok(ini) => {
                for section in ini.sections() {
                    if let Some(name) = section {
                        shares.push(DirShareInfo {
                            name: name.to_string(),
                        });
                    }
                }
            }
            Err(e) => {
                return HttpResponse::InternalServerError().json(ListDirSharesResponse {
                    success: false,
                    shares: vec![],
                    message: "Failed to parse public.conf".to_string(),
                    error: Some(format!("{}", e)),
                });
            }
        }
    }

    // 3. 读取 private.conf
    let private_conf = conf_dir.join("private.conf");
    if private_conf.exists() {
        match Ini::load_from_file(&private_conf) {
            Ok(ini) => {
                for section in ini.sections() {
                    if let Some(name) = section {
                        shares.push(DirShareInfo {
                            name: name.to_string(),
                        });
                    }
                }
            }
            Err(e) => {
                return HttpResponse::InternalServerError().json(ListDirSharesResponse {
                    success: false,
                    shares,
                    message: "Failed to parse private.conf".to_string(),
                    error: Some(format!("{}", e)),
                });
            }
        }
    }

    HttpResponse::Ok().json(ListDirSharesResponse {
        success: true,
        shares,
        message: "Directory shares listed successfully".to_string(),
        error: None,
    })
}

// Remove Directory Share 请求结构体
#[derive(Deserialize)]
pub struct RemoveDirShareRequest {
    pub share_name: String,  // 要删除的共享名称（section title）
}

// Remove Directory Share 响应结构体
#[derive(Serialize)]
pub struct RemoveDirShareResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

// remove_dir_share API - 删除 /etc/samba/conf.d/public.conf 或 private.conf 中的共享配置（需要 share/admin 权限）
#[post("/smb/remove_dir_share")]
pub async fn remove_dir_share(
    req: HttpRequest,
    remove_req: web::Json<RemoveDirShareRequest>,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    // 1. 验证 JWT token 并检查 share/admin 权限
    let _username = match crate::utils::verify_share_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let share_name = &remove_req.share_name;

    // 2. 验证 share_name 不为空
    if share_name.is_empty() {
        return HttpResponse::BadRequest().json(RemoveDirShareResponse {
            success: false,
            message: "Share name cannot be empty".to_string(),
            error: Some("share_name is required".to_string()),
        });
    }

    let conf_dir = Path::new("/etc/samba/conf.d");
    let mut found = false;
    let mut deleted_from = String::new();

    // 3. 尝试从 public.conf 中删除
    let public_conf = conf_dir.join("public.conf");
    if public_conf.exists() {
        match Ini::load_from_file(&public_conf) {
            Ok(mut ini) => {
                if ini.delete(Some(share_name)).is_some() {
                    found = true;
                    deleted_from = "public.conf".to_string();
                    // 写回文件
                    if let Err(e) = ini.write_to_file(&public_conf) {
                        return HttpResponse::InternalServerError().json(RemoveDirShareResponse {
                            success: false,
                            message: "Failed to write public.conf".to_string(),
                            error: Some(format!("{}", e)),
                        });
                    }
                }
            }
            Err(e) => {
                return HttpResponse::InternalServerError().json(RemoveDirShareResponse {
                    success: false,
                    message: "Failed to parse public.conf".to_string(),
                    error: Some(format!("{}", e)),
                });
            }
        }
    }

    // 4. 如果不在 public.conf 中，尝试从 private.conf 中删除
    if !found {
        let private_conf = conf_dir.join("private.conf");
        if private_conf.exists() {
            match Ini::load_from_file(&private_conf) {
                Ok(mut ini) => {
                    if ini.delete(Some(share_name)).is_some() {
                        found = true;
                        deleted_from = "private.conf".to_string();
                        // 写回文件
                        if let Err(e) = ini.write_to_file(&private_conf) {
                            return HttpResponse::InternalServerError().json(RemoveDirShareResponse {
                                success: false,
                                message: "Failed to write private.conf".to_string(),
                                error: Some(format!("{}", e)),
                            });
                        }
                    }
                }
                Err(e) => {
                    return HttpResponse::InternalServerError().json(RemoveDirShareResponse {
                        success: false,
                        message: "Failed to parse private.conf".to_string(),
                        error: Some(format!("{}", e)),
                    });
                }
            }
        }
    }

    // 5. 如果没有找到该 share
    if !found {
        return HttpResponse::NotFound().json(RemoveDirShareResponse {
            success: false,
            message: format!("Share '{}' not found in public.conf or private.conf", share_name),
            error: Some("Share does not exist".to_string()),
        });
    }

        // 8. 合并配置文件并重新加载 Samba 服务
    if let Err(e) = merge_samba_configs() {
        return HttpResponse::InternalServerError().json(SmbPublicShareResponse {
            success: false,
            message: "Failed to merge samba configs".to_string(),
            error: Some(e),
        });
    }
    
    // 6. 重新加载 Samba 服务
    let _ = Command::new("systemctl")
        .args(["restart", "smbd"])
        .output();

    HttpResponse::Ok().json(RemoveDirShareResponse {
        success: true,
        message: format!("Share '{}' deleted successfully from {}", share_name, deleted_from),
        error: None,
    })
}

// Remove ZFS Share 请求结构体
#[derive(Deserialize)]
pub struct RemoveZfsShareRequest {
    pub dataset: String,  // ZFS dataset 名称，如 "tank/myshare"
}

// Remove ZFS Share 响应结构体
#[derive(Serialize)]
pub struct RemoveZfsShareResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

// remove_zfs_share API - 卸载 ZFS dataset 并关闭 SMB 共享（需要 share/admin 权限）
// 执行步骤：
// 1. zfs umount <dataset>
// 2. zfs set sharesmb=off <dataset>
#[post("/smb/remove_zfs_share")]
pub async fn remove_zfs_share(
    req: HttpRequest,
    remove_req: web::Json<RemoveZfsShareRequest>,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    // 1. 验证 JWT token 并检查 share/admin 权限
    let _username = match crate::utils::verify_share_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let dataset = &remove_req.dataset;

    // 2. 验证 dataset 不为空
    if dataset.is_empty() {
        return HttpResponse::BadRequest().json(RemoveZfsShareResponse {
            success: false,
            message: "Dataset cannot be empty".to_string(),
            error: Some("dataset is required".to_string()),
        });
    }

    // 3. 卸载 dataset: zfs umount <dataset>
    let output = Command::new("zfs")
        .args(["umount", dataset])
        .output();

    match output {
        Ok(result) => {
            if !result.status.success() {
                let stderr = String::from_utf8_lossy(&result.stderr);
                // 如果错误是 dataset 不存在，继续尝试关闭 sharesmb
                if !stderr.contains("does not exist") {
                    return HttpResponse::InternalServerError().json(RemoveZfsShareResponse {
                        success: false,
                        message: format!("Failed to umount dataset: {}", dataset),
                        error: Some(format!("{}", stderr)),
                    });
                }
            }
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(RemoveZfsShareResponse {
                success: false,
                message: format!("Failed to execute zfs umount: {}", dataset),
                error: Some(format!("{}", e)),
            });
        }
    }

    // 4. 关闭 SMB 共享: zfs set sharesmb=off <dataset>
    let output = Command::new("zfs")
        .args(["set", "sharesmb=off", dataset])
        .output();

    match output {
        Ok(result) => {
            if !result.status.success() {
                let stderr = String::from_utf8_lossy(&result.stderr);
                return HttpResponse::InternalServerError().json(RemoveZfsShareResponse {
                    success: false,
                    message: format!("Failed to set sharesmb=off for dataset: {}", dataset),
                    error: Some(format!("{}", stderr)),
                });
            }
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(RemoveZfsShareResponse {
                success: false,
                message: format!("Failed to execute zfs set sharesmb=off: {}", dataset),
                error: Some(format!("{}", e)),
            });
        }
    }

    // 5. 重新加载 Samba 服务
    let _ = Command::new("systemctl")
        .args(["restart", "smbd"])
        .output();

    HttpResponse::Ok().json(RemoveZfsShareResponse {
        success: true,
        message: format!("ZFS share '{}' unmounted and SMB sharing disabled", dataset),
        error: None,
    })
}
