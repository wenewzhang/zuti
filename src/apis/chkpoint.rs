use actix_web::{delete, get, post, HttpRequest, HttpResponse, Responder, web};
use serde::{Deserialize, Serialize};
use std::process::Command;

use crate::utils::admin::{validate_token_with_db, verify_admin_access};

/// 查询 checkpoint 请求结构体
#[derive(Deserialize)]
pub struct GetCheckpointRequest {
    pub poolname: String,
}

/// 创建 checkpoint 请求结构体
#[derive(Deserialize)]
pub struct CreateCheckpointRequest {
    pub poolname: String,
}

/// 创建 checkpoint 响应结构体
#[derive(Serialize)]
pub struct CreateCheckpointResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

/// Checkpoint 信息结构体
#[derive(Serialize)]
pub struct CheckpointInfo {
    pub pool: String,
    pub property: String,
    pub value: String,
    pub source: String,
}

/// Checkpoint 响应结构体
#[derive(Serialize)]
pub struct CheckpointResponse {
    pub success: bool,
    pub data: Option<CheckpointInfo>,
    pub error: Option<String>,
}

/// 获取指定 pool 的 checkpoint 信息
/// 执行命令: zpool get checkpoint <poolname>
fn get_checkpoint_info(poolname: &str) -> Option<CheckpointInfo> {
    let output = Command::new("zpool")
        .args(["get", "checkpoint", "-H", poolname])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    // 解析输出格式: "one-pool\tcheckpoint\t-\tlocal"
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 4 {
            return Some(CheckpointInfo {
                pool: parts[0].trim().to_string(),
                property: parts[1].trim().to_string(),
                value: parts[2].trim().to_string(),
                source: parts[3].trim().to_string(),
            });
        }
    }

    None
}

/// POST /chkpoint - 创建 ZFS pool 的 checkpoint（需要 JWT 认证，仅管理员可用）
#[post("/chk/checkpoint")]
pub async fn create_chkpoint(
    req: HttpRequest,
    checkpoint_req: web::Json<CreateCheckpointRequest>,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    // 1. 验证 JWT token 并检查 admin 权限
    let _username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let poolname = &checkpoint_req.poolname;

    // 2. 验证 pool 名称合法性
    // ZFS pool 名称允许: 字母、数字、下划线、连字符、点
    if !poolname.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.') {
        return HttpResponse::BadRequest().json(CreateCheckpointResponse {
            success: false,
            message: "Invalid pool name format".to_string(),
            error: Some("Pool name must contain only alphanumeric characters, underscores, hyphens, or dots".to_string()),
        });
    }

    // 3. 执行 zpool checkpoint 命令
    let output = match Command::new("zpool")
        .args(["checkpoint", poolname])
        .output()
    {
        Ok(result) => result,
        Err(e) => {
            return HttpResponse::InternalServerError().json(CreateCheckpointResponse {
                success: false,
                message: "Failed to execute zpool checkpoint command".to_string(),
                error: Some(format!("Command error: {}", e)),
            });
        }
    };

    if output.status.success() {
        HttpResponse::Ok().json(CreateCheckpointResponse {
            success: true,
            message: format!("Successfully created checkpoint for pool '{}'", poolname),
            error: None,
        })
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        HttpResponse::InternalServerError().json(CreateCheckpointResponse {
            success: false,
            message: format!("Failed to create checkpoint for pool '{}'", poolname),
            error: Some(stderr.to_string()),
        })
    }
}

/// DELETE /chkpoint - 删除 ZFS pool 的 checkpoint（需要 JWT 认证，仅管理员可用）
#[delete("/chk/checkpoint")]
pub async fn remove_chkpoint(
    req: HttpRequest,
    checkpoint_req: web::Json<CreateCheckpointRequest>,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    // 1. 验证 JWT token 并检查 admin 权限
    let _username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let poolname = &checkpoint_req.poolname;

    // 2. 验证 pool 名称合法性
    // ZFS pool 名称允许: 字母、数字、下划线、连字符、点
    if !poolname.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.') {
        return HttpResponse::BadRequest().json(CreateCheckpointResponse {
            success: false,
            message: "Invalid pool name format".to_string(),
            error: Some("Pool name must contain only alphanumeric characters, underscores, hyphens, or dots".to_string()),
        });
    }

    // 3. 执行 zpool checkpoint -d 命令
    let output = match Command::new("zpool")
        .args(["checkpoint", "-d", poolname])
        .output()
    {
        Ok(result) => result,
        Err(e) => {
            return HttpResponse::InternalServerError().json(CreateCheckpointResponse {
                success: false,
                message: "Failed to execute zpool checkpoint -d command".to_string(),
                error: Some(format!("Command error: {}", e)),
            });
        }
    };

    if output.status.success() {
        HttpResponse::Ok().json(CreateCheckpointResponse {
            success: true,
            message: format!("Successfully removed checkpoint for pool '{}'", poolname),
            error: None,
        })
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        HttpResponse::InternalServerError().json(CreateCheckpointResponse {
            success: false,
            message: format!("Failed to remove checkpoint for pool '{}'", poolname),
            error: Some(stderr.to_string()),
        })
    }
}

/// Rollback checkpoint 请求结构体
#[derive(Deserialize)]
pub struct RollbackCheckpointRequest {
    pub poolname: String,
}

/// Rollback checkpoint 响应结构体
#[derive(Serialize)]
pub struct RollbackCheckpointResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

/// POST /chk/rollback - 回滚 ZFS pool 到 checkpoint（需要 JWT 认证，仅管理员可用）
/// 执行: zpool export <poolname>, 然后 zpool import --rewind-to-checkpoint <poolname>
#[post("/chk/rollback")]
pub async fn rollback_chkpoint(
    req: HttpRequest,
    rollback_req: web::Json<RollbackCheckpointRequest>,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    // 1. 验证 JWT token 并检查 admin 权限
    let _username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let poolname = &rollback_req.poolname;

    // 2. 验证 pool 名称合法性
    // ZFS pool 名称允许: 字母、数字、下划线、连字符、点
    if !poolname.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.') {
        return HttpResponse::BadRequest().json(RollbackCheckpointResponse {
            success: false,
            message: "Invalid pool name format".to_string(),
            error: Some("Pool name must contain only alphanumeric characters, underscores, hyphens, or dots".to_string()),
        });
    }

    // 3. 执行 zpool export 命令
    let export_output = match Command::new("zpool")
        .args(["export", poolname])
        .output()
    {
        Ok(result) => result,
        Err(e) => {
            return HttpResponse::InternalServerError().json(RollbackCheckpointResponse {
                success: false,
                message: "Failed to execute zpool export command".to_string(),
                error: Some(format!("Command error: {}", e)),
            });
        }
    };

    if !export_output.status.success() {
        let stderr = String::from_utf8_lossy(&export_output.stderr);
        return HttpResponse::InternalServerError().json(RollbackCheckpointResponse {
            success: false,
            message: format!("Failed to export pool '{}'", poolname),
            error: Some(stderr.to_string()),
        });
    }

    // 4. 执行 zpool import --rewind-to-checkpoint 命令
    let import_output = match Command::new("zpool")
        .args(["import", "--rewind-to-checkpoint", poolname])
        .output()
    {
        Ok(result) => result,
        Err(e) => {
            // 尝试重新导入 pool（不带 rewind）以恢复可用状态
            let _ = Command::new("zpool")
                .args(["import", poolname])
                .output();
            
            return HttpResponse::InternalServerError().json(RollbackCheckpointResponse {
                success: false,
                message: "Failed to execute zpool import --rewind-to-checkpoint command".to_string(),
                error: Some(format!("Command error: {}", e)),
            });
        }
    };

    if import_output.status.success() {
        HttpResponse::Ok().json(RollbackCheckpointResponse {
            success: true,
            message: format!("Successfully rolled back pool '{}' to checkpoint", poolname),
            error: None,
        })
    } else {
        let stderr = String::from_utf8_lossy(&import_output.stderr);
        // 尝试重新导入 pool（不带 rewind）以恢复可用状态
        let _ = Command::new("zpool")
            .args(["import", poolname])
            .output();
        
        HttpResponse::InternalServerError().json(RollbackCheckpointResponse {
            success: false,
            message: format!("Failed to rollback pool '{}' to checkpoint", poolname),
            error: Some(stderr.to_string()),
        })
    }
}

/// GET /chkpoint?poolname=<poolname> - 获取指定 ZFS pool 的 checkpoint 信息（需要 JWT 认证）
#[get("/chk/checkpoint")]
pub async fn get_chkpoint(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
    query: web::Query<GetCheckpointRequest>,
) -> impl Responder {
    // 验证 JWT token
    if let Err(response) = validate_token_with_db(&req, &pool).await {
        return response;
    }

    let poolname = &query.poolname;

    // 验证 pool 名称合法性
    // ZFS pool 名称允许: 字母、数字、下划线、连字符、点
    if !poolname.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.') {
        return HttpResponse::BadRequest().json(CheckpointResponse {
            success: false,
            data: None,
            error: Some("Invalid pool name format".to_string()),
        });
    }

    // 获取 checkpoint 信息
    match get_checkpoint_info(poolname) {
        Some(info) => HttpResponse::Ok().json(CheckpointResponse {
            success: true,
            data: Some(info),
            error: None,
        }),
        None => HttpResponse::InternalServerError().json(CheckpointResponse {
            success: false,
            data: None,
            error: Some(format!("Failed to get checkpoint for pool '{}'", poolname)),
        }),
    }
}
