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
    let output = Command::new("pdbedit")
        .args(["-L"])
        .output();

    let user_exists = match output {
        Ok(result) => {
            if result.status.success() {
                let stdout = String::from_utf8_lossy(&result.stdout);
                stdout.lines().any(|line| {
                    let parts: Vec<&str> = line.split(':').collect();
                    !parts.is_empty() && parts[0] == username
                })
            } else {
                false
            }
        }
        Err(_) => false,
    };

    if !user_exists {
        return HttpResponse::BadRequest().json(SmbDeleteUserResponse {
            success: false,
            message: format!("Samba user '{}' does not exist", username),
            error: Some("User not found in Samba user list".to_string()),
        });
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
    let _username = match crate::utils::verify_share_access(&req, &pool).await {
        Ok(username) => username,
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
    let _username = match crate::utils::verify_share_access(&req, &pool).await {
        Ok(username) => username,
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
