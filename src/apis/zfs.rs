use actix_web::{get, post, HttpRequest, HttpResponse, Responder, web};
use serde::{Deserialize, Serialize};
use std::process::Command;

use crate::utils::admin::{validate_token_with_db, verify_admin_access};

/// Dataset 信息结构体
#[derive(Serialize, Deserialize, Debug)]
pub struct DatasetInfo {
    pub name: String,
    pub mountpoint: String,
}

/// Bootfs 数据结构体
#[derive(Serialize)]
pub struct BootfsData {
    pub bootfs: String,
    pub datasets: Vec<DatasetInfo>,
}

/// Bootfs 响应结构体
#[derive(Serialize)]
pub struct BootfsResponse {
    pub success: bool,
    pub data: Option<BootfsData>,
    pub error: Option<String>,
}

/// 获取 zfs list 命令的输出
fn get_zfs_list_output() -> Option<String> {
    Command::new("zfs")
        .args(["list", "-o", "name,mountpoint", "-H"])
        .output()
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
}

/// 获取当前 bootfs (zpool get bootfs)
fn get_bootfs_source() -> Option<String> {
    Command::new("zpool")
        .args(["get", "bootfs", "-H"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                // 解析输出: "one-pool\tbootfs\tone-pool/ROOT/zuti-260225_NEW\tlocal"
                for line in stdout.lines() {
                    let parts: Vec<&str> = line.split('\t').collect();
                    if parts.len() >= 3 {
                        let value = parts[2].trim();
                        // 排除 "-" 和 "none" 值
                        if value != "-" && value != "none" && !value.is_empty() {
                            return Some(value.to_string());
                        }
                    }
                }
                None
            } else {
                None
            }
        })
}

/// 从 zfs list 输出中解析 mountpoint 为 / 的 dataset
fn parse_root_datasets(output: &str) -> Vec<DatasetInfo> {
    let mut result = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // 解析行: "dataset_name\t/mount/point"
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() >= 2 {
            let name = parts[0].to_string();
            let mountpoint = parts[1].to_string();

            // 筛选 mountpoint 为 / 的 dataset
            if mountpoint == "/" {
                result.push(DatasetInfo { name, mountpoint });
            }
        }
    }

    result
}

/// zfs/bootfs API - 获取所有 mountpoint 为 / 的 ZFS dataset（需要 JWT 认证）
#[get("/zfs/bootfs")]
pub async fn get_bootfs(req: HttpRequest, pool: web::Data<crate::DbPool>) -> impl Responder {
    // 验证 JWT token
    if let Err(response) = validate_token_with_db(&req, &pool).await {
        return response;
    }

    // 执行 zfs list 命令
    let output = match get_zfs_list_output() {
        Some(output) => output,
        None => {
            return HttpResponse::InternalServerError().json(BootfsResponse {
                success: false,
                data: None,
                error: Some("Failed to execute zfs list command".to_string()),
            });
        }
    };

    // 解析输出，获取 mountpoint 为 / 的 dataset
    let datasets = parse_root_datasets(&output);

    // 获取当前 bootfs
    let bootfs = get_bootfs_source().unwrap_or_default();

    HttpResponse::Ok().json(BootfsResponse {
        success: true,
        data: Some(BootfsData { bootfs, datasets }),
        error: None,
    })
}

// set_bootfs 请求结构体
#[derive(Deserialize)]
pub struct SetBootfsRequest {
    pub dataset: String,  // 例如: "one-pool/ROOT/zuti-260225_NEW"
    pub pool: String,     // 例如: "one-pool"
}

// set_bootfs 响应结构体
#[derive(Serialize)]
pub struct SetBootfsResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

// reboot 响应结构体
#[derive(Serialize)]
pub struct RebootResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

/// zfs/reboot API - 重启系统（需要 JWT 认证，仅管理员可用）
#[post("/zfs/reboot")]
pub async fn reboot_system(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    // 1. 验证 JWT token 并检查 admin 权限
    let _username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
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

/// zfs/shutdown API - 关闭系统（需要 JWT 认证，仅管理员可用）
#[post("/zfs/shutdown")]
pub async fn shutdown_system(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    // 1. 验证 JWT token 并检查 admin 权限
    let _username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
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

/// zfs/set_bootfs API - 设置 ZFS pool 的 bootfs 属性（需要 JWT 认证，仅管理员可用）
#[post("/zfs/set_bootfs")]
pub async fn set_bootfs(
    req: HttpRequest,
    bootfs_req: web::Json<SetBootfsRequest>,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    // 1. 验证 JWT token 并检查 admin 权限
    let _username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let dataset = &bootfs_req.dataset;
    let pool_name = &bootfs_req.pool;

    // 2. 验证 dataset 名称合法性
    // ZFS dataset 名称允许: 字母、数字、下划线、连字符、点、斜杠
    if !dataset.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '/') {
        return HttpResponse::BadRequest().json(SetBootfsResponse {
            success: false,
            message: "Invalid dataset name format".to_string(),
            error: Some("Dataset name must contain only alphanumeric characters, underscores, hyphens, dots, or slashes".to_string()),
        });
    }

    // 3. 验证 pool 名称合法性
    if !pool_name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.') {
        return HttpResponse::BadRequest().json(SetBootfsResponse {
            success: false,
            message: "Invalid pool name format".to_string(),
            error: Some("Pool name must contain only alphanumeric characters, underscores, hyphens, or dots".to_string()),
        });
    }

    // 4. 构建并执行 zpool set bootfs 命令
    let bootfs_value = format!("bootfs={}", dataset);
    let output = match Command::new("zpool")
        .args(["set", &bootfs_value, pool_name])
        .output()
    {
        Ok(result) => result,
        Err(e) => {
            return HttpResponse::InternalServerError().json(SetBootfsResponse {
                success: false,
                message: "Failed to execute zpool set command".to_string(),
                error: Some(format!("Command error: {}", e)),
            });
        }
    };

    if output.status.success() {
        HttpResponse::Ok().json(SetBootfsResponse {
            success: true,
            message: format!("Successfully set bootfs to '{}' for pool '{}'", dataset, pool_name),
            error: None,
        })
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        HttpResponse::InternalServerError().json(SetBootfsResponse {
            success: false,
            message: format!("Failed to set bootfs for pool '{}'", pool_name),
            error: Some(stderr.to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_root_datasets() {
        let input = "rpool/ROOT/ubuntu\t/\nrpool/home\t/home\nrpool/var\t/var\nrpool/ROOT/debian\t/\n";
        let result = parse_root_datasets(input);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "rpool/ROOT/ubuntu");
        assert_eq!(result[0].mountpoint, "/");
        assert_eq!(result[1].name, "rpool/ROOT/debian");
        assert_eq!(result[1].mountpoint, "/");
    }

    #[test]
    fn test_parse_root_datasets_empty() {
        let input = "rpool/home\t/home\nrpool/var\t/var\n";
        let result = parse_root_datasets(input);
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_root_datasets_with_legacy() {
        let input = "rpool/ROOT/ubuntu\t/\nrpool/ROOT/ubuntu/var\t/var\nrpool\tlegacy\n";
        let result = parse_root_datasets(input);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "rpool/ROOT/ubuntu");
        assert_eq!(result[0].mountpoint, "/");
    }
}
