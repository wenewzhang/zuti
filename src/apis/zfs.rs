use actix_web::{get, post, HttpRequest, HttpResponse, Responder, web};
use serde::{Deserialize, Serialize};
use std::process::Command;

use crate::utils::admin::{validate_token_with_db, verify_admin_access};
use crate::utils::consts::FORBID_DIRECTORY;
use crate::disk::zfs_utils::DatasetDetail;

/// Dataset 信息结构体
#[derive(Serialize, Deserialize, Debug)]
pub struct DatasetInfo {
    pub name: String,
    pub mountpoint: String,
}

/// Dataset 详细信息响应结构体
#[derive(Serialize)]
pub struct DatasetDetailInfo {
    pub name: String,
    pub used: String,
    pub avail: String,
    pub refer: String,
    pub mountpoint: String,
}

impl From<DatasetDetail> for DatasetDetailInfo {
    fn from(d: DatasetDetail) -> Self {
        Self {
            name: d.name,
            used: d.used,
            avail: d.avail,
            refer: d.refer,
            mountpoint: d.mountpoint,
        }
    }
}

/// Datasets 响应结构体
#[derive(Serialize)]
pub struct DatasetsResponse {
    pub success: bool,
    pub data: Option<Vec<DatasetDetailInfo>>,
    pub error: Option<String>,
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

/// zfs/datasets API - 获取所有 ZFS datasets（包括 snapshots 和 volumes，需要 JWT 认证）
#[get("/zfs/datasets")]
pub async fn get_datasets(req: HttpRequest, pool: web::Data<crate::DbPool>) -> impl Responder {
    // 验证 JWT token
    if let Err(response) = validate_token_with_db(&req, &pool).await {
        return response;
    }

    // 调用 disk/zfs 模块的 get_datasets 函数
    let datasets = crate::disk::zfs_utils::get_datasets();

    if datasets.is_empty() {
        // 可能是命令执行失败或没有 datasets
        return HttpResponse::Ok().json(DatasetsResponse {
            success: true,
            data: Some(Vec::new()),
            error: None,
        });
    }

    // 转换为 API 响应格式
    let dataset_infos: Vec<DatasetDetailInfo> = datasets.into_iter().map(|d| d.into()).collect();

    HttpResponse::Ok().json(DatasetsResponse {
        success: true,
        data: Some(dataset_infos),
        error: None,
    })
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

/// zfs/reboot_force API - 强制重启系统（需要 JWT 认证，仅管理员可用）
#[post("/zfs/reboot_force")]
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

/// Dataset 属性结构体
#[derive(Debug)]
struct DatasetProperties {
    mountpoint: String,
    canmount: String,
}

/// 获取 dataset 的 mountpoint 和 canmount 属性
fn get_dataset_properties(dataset: &str) -> Option<DatasetProperties> {
    let output = Command::new("zfs")
        .args(["get", "mountpoint,canmount", "-H", "-o", "property,value", dataset])
        .output()
        .ok()?;
    
    if !output.status.success() {
        return None;
    }
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut mountpoint = String::new();
    let mut canmount = String::new();
    
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 2 {
            let property = parts[0].trim();
            let value = parts[1].trim();
            match property {
                "mountpoint" => mountpoint = value.to_string(),
                "canmount" => canmount = value.to_string(),
                _ => {}
            }
        }
    }
    
    if mountpoint.is_empty() || canmount.is_empty() {
        None
    } else {
        Some(DatasetProperties { mountpoint, canmount })
    }
}

/// 设置 dataset 的 mountpoint 属性
fn set_mountpoint(dataset: &str, mountpoint: &str) -> Result<(), String> {
    let output = Command::new("zfs")
        .args(["set", &format!("mountpoint={}", mountpoint), dataset])
        .output()
        .map_err(|e| format!("Command error: {}", e))?;
    
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

/// 设置 dataset 的 canmount 属性
fn set_canmount(dataset: &str, canmount: &str) -> Result<(), String> {
    let output = Command::new("zfs")
        .args(["set", &format!("canmount={}", canmount), dataset])
        .output()
        .map_err(|e| format!("Command error: {}", e))?;
    
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

// zfs clone 请求结构体
#[derive(Deserialize)]
pub struct CloneDatasetRequest {
    pub new_name: String,  // 例如: "zuti-260225_NEW"
    pub dataset: String,   // 例如: "one-pool/ROOT/zuti-260225"
}

// zfs clone 响应结构体
#[derive(Serialize)]
pub struct CloneDatasetResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

/// zfs/clone API - 创建 ZFS snapshot 并 clone（需要 JWT 认证，仅管理员可用）
#[post("/zfs/clone")]
pub async fn clone_dataset(
    req: HttpRequest,
    clone_req: web::Json<CloneDatasetRequest>,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    // 1. 验证 JWT token 并检查 admin 权限
    let _username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let new_name = &clone_req.new_name;
    let dataset = &clone_req.dataset;

    // 2. 验证 new_name 名称合法性
    // snapshot 名称允许: 字母、数字、下划线、连字符、点
    if !new_name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.') {
        return HttpResponse::BadRequest().json(CloneDatasetResponse {
            success: false,
            message: "Invalid new_name format".to_string(),
            error: Some("new_name must contain only alphanumeric characters, underscores, hyphens, or dots".to_string()),
        });
    }

    // 3. 验证 dataset 名称合法性
    // ZFS dataset 名称允许: 字母、数字、下划线、连字符、点、斜杠
    if !dataset.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '/') {
        return HttpResponse::BadRequest().json(CloneDatasetResponse {
            success: false,
            message: "Invalid dataset name format".to_string(),
            error: Some("Dataset name must contain only alphanumeric characters, underscores, hyphens, dots, or slashes".to_string()),
        });
    }

    // 4. 获取原 dataset 的 mountpoint 和 canmount 属性（在创建 snapshot 前获取）
    let props = match get_dataset_properties(dataset) {
        Some(p) => p,
        None => {
            return HttpResponse::InternalServerError().json(CloneDatasetResponse {
                success: false,
                message: "Failed to get source dataset properties".to_string(),
                error: Some(format!("Cannot get mountpoint/canmount from '{}'", dataset)),
            });
        }
    };

    // 5. 创建 snapshot: zfs snapshot dataset@new_name
    let snapshot_name = format!("{}@{}", dataset, new_name);
    let snapshot_output = match Command::new("zfs")
        .args(["snapshot", &snapshot_name])
        .output()
    {
        Ok(result) => result,
        Err(e) => {
            return HttpResponse::InternalServerError().json(CloneDatasetResponse {
                success: false,
                message: "Failed to execute zfs snapshot command".to_string(),
                error: Some(format!("Command error: {}", e)),
            });
        }
    };

    if !snapshot_output.status.success() {
        let stderr = String::from_utf8_lossy(&snapshot_output.stderr);
        return HttpResponse::InternalServerError().json(CloneDatasetResponse {
            success: false,
            message: format!("Failed to create snapshot '{}'", snapshot_name),
            error: Some(stderr.to_string()),
        });
    }

    // 6. 创建 clone: zfs clone dataset@new_name new_dataset
    // new_dataset 是去掉 dataset 最后一段后的路径 + new_name
    // 例如: one-pool/ROOT/zuti-260225 -> one-pool/ROOT/ + zuti-260225_NEW
    let new_dataset = if let Some(last_slash_pos) = dataset.rfind('/') {
        format!("{}{}", &dataset[..=last_slash_pos], new_name)
    } else {
        // 如果没有斜杠，直接在前面添加 new_name（这种情况在 ZFS 中很少见）
        new_name.to_string()
    };
    let clone_name = new_dataset.clone();
    let clone_output = match Command::new("zfs")
        .args(["clone", &snapshot_name, &clone_name])
        .output()
    {
        Ok(result) => result,
        Err(e) => {
            return HttpResponse::InternalServerError().json(CloneDatasetResponse {
                success: false,
                message: "Failed to execute zfs clone command".to_string(),
                error: Some(format!("Command error: {}", e)),
            });
        }
    };

    if !clone_output.status.success() {
        let stderr = String::from_utf8_lossy(&clone_output.stderr);
        return HttpResponse::InternalServerError().json(CloneDatasetResponse {
            success: false,
            message: format!("Failed to clone snapshot '{}' to '{}'", snapshot_name, clone_name),
            error: Some(stderr.to_string()),
        });
    }

    // 7. 设置新 clone 的 mountpoint 和 canmount 属性
    // 判断是否为系统盘配置：mountpoint=/ 且 canmount=on
    let is_system_root = props.mountpoint == "/" && props.canmount == "on";
    
    // 如果是系统盘，自动将 canmount 改为 noauto，避免挂载冲突
    let target_canmount = if is_system_root {
        "noauto"
    } else {
        &props.canmount
    };

    // 先设置 canmount=off 防止自动挂载冲突，再设置 mountpoint，最后设置目标 canmount
    if let Err(e) = set_canmount(&clone_name, "off") {
        return HttpResponse::InternalServerError().json(CloneDatasetResponse {
            success: false,
            message: format!("Clone created but failed to set canmount=off for '{}'", clone_name),
            error: Some(e),
        });
    }

    if let Err(e) = set_mountpoint(&clone_name, &props.mountpoint) {
        return HttpResponse::InternalServerError().json(CloneDatasetResponse {
            success: false,
            message: format!("Clone created but failed to set mountpoint for '{}'", clone_name),
            error: Some(e),
        });
    }

    if let Err(e) = set_canmount(&clone_name, target_canmount) {
        return HttpResponse::InternalServerError().json(CloneDatasetResponse {
            success: false,
            message: format!("Clone created but failed to set canmount='{}' for '{}'", target_canmount, clone_name),
            error: Some(e),
        });
    }

    // 构建返回消息
    let message = if is_system_root {
        format!(
            "Successfully created snapshot '{}' and cloned to '{}' with mountpoint='{}', canmount changed from 'on' to 'noauto' (system root detected)",
            snapshot_name, clone_name, props.mountpoint
        )
    } else {
        format!(
            "Successfully created snapshot '{}' and cloned to '{}' with mountpoint='{}', canmount='{}'",
            snapshot_name, clone_name, props.mountpoint, target_canmount
        )
    };

    HttpResponse::Ok().json(CloneDatasetResponse {
        success: true,
        message,
        error: None,
    })
}

// zfs promote 请求结构体
#[derive(Deserialize)]
pub struct PromoteDatasetRequest {
    pub dataset: String,  // 例如: "one-pool/ROOT/zuti-260225_NEW"
}

// zfs promote 响应结构体
#[derive(Serialize)]
pub struct PromoteDatasetResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

/// zfs/prompt API - 提升 ZFS clone dataset（需要 JWT 认证，仅管理员可用）
#[post("/zfs/prompt")]
pub async fn promote_dataset(
    req: HttpRequest,
    promote_req: web::Json<PromoteDatasetRequest>,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    // 1. 验证 JWT token 并检查 admin 权限
    let _username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let dataset = &promote_req.dataset;

    // 2. 验证 dataset 名称合法性
    // ZFS dataset 名称允许: 字母、数字、下划线、连字符、点、斜杠
    if !dataset.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '/') {
        return HttpResponse::BadRequest().json(PromoteDatasetResponse {
            success: false,
            message: "Invalid dataset name format".to_string(),
            error: Some("Dataset name must contain only alphanumeric characters, underscores, hyphens, dots, or slashes".to_string()),
        });
    }

    // 3. 检查 dataset 是否存在
    let check_output = match Command::new("zfs")
        .args(["list", "-H", dataset])
        .output()
    {
        Ok(result) => result,
        Err(e) => {
            return HttpResponse::InternalServerError().json(PromoteDatasetResponse {
                success: false,
                message: "Failed to check dataset existence".to_string(),
                error: Some(format!("Command error: {}", e)),
            });
        }
    };

    if !check_output.status.success() {
        let stderr = String::from_utf8_lossy(&check_output.stderr);
        return HttpResponse::BadRequest().json(PromoteDatasetResponse {
            success: false,
            message: format!("Dataset '{}' does not exist", dataset),
            error: Some(stderr.to_string()),
        });
    }

    // 4. 执行 zfs promote 命令
    let output = match Command::new("zfs")
        .args(["promote", dataset])
        .output()
    {
        Ok(result) => result,
        Err(e) => {
            return HttpResponse::InternalServerError().json(PromoteDatasetResponse {
                success: false,
                message: "Failed to execute zfs promote command".to_string(),
                error: Some(format!("Command error: {}", e)),
            });
        }
    };

    if output.status.success() {
        HttpResponse::Ok().json(PromoteDatasetResponse {
            success: true,
            message: format!("Successfully promoted dataset '{}'", dataset),
            error: None,
        })
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        HttpResponse::InternalServerError().json(PromoteDatasetResponse {
            success: false,
            message: format!("Failed to promote dataset '{}'", dataset),
            error: Some(stderr.to_string()),
        })
    }
}

// zfs destroy 请求结构体
#[derive(Deserialize)]
pub struct DestroyDatasetRequest {
    pub dataset: String,  // 例如: "one-pool/ROOT/zuti-260225_NEW"
}

// zfs destroy 响应结构体
#[derive(Serialize)]
pub struct DestroyDatasetResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

/// zfs/destroy API - 删除 ZFS dataset（需要 JWT 认证，仅管理员可用）
#[post("/zfs/destroy")]
pub async fn destroy_dataset(
    req: HttpRequest,
    destroy_req: web::Json<DestroyDatasetRequest>,
    pool: web::Data<crate::DbPool>,
) -> impl Responder {
    // 1. 验证 JWT token 并检查 admin 权限
    let _username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let dataset = &destroy_req.dataset;

    // 2. 验证 dataset 名称合法性
    // ZFS dataset 名称允许: 字母、数字、下划线、连字符、点、斜杠、@（用于 snapshot）
    if !dataset.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '/' || c == '@') {
        return HttpResponse::BadRequest().json(DestroyDatasetResponse {
            success: false,
            message: "Invalid dataset name format".to_string(),
            error: Some("Dataset name must contain only alphanumeric characters, underscores, hyphens, dots, slashes, or @".to_string()),
        });
    }

    // 3. 检查 dataset 是否存在
    let check_output = match Command::new("zfs")
        .args(["list", "-H", dataset])
        .output()
    {
        Ok(result) => result,
        Err(e) => {
            return HttpResponse::InternalServerError().json(DestroyDatasetResponse {
                success: false,
                message: "Failed to check dataset existence".to_string(),
                error: Some(format!("Command error: {}", e)),
            });
        }
    };

    if !check_output.status.success() {
        let stderr = String::from_utf8_lossy(&check_output.stderr);
        return HttpResponse::BadRequest().json(DestroyDatasetResponse {
            success: false,
            message: format!("Dataset '{}' does not exist", dataset),
            error: Some(stderr.to_string()),
        });
    }

    // 4. 执行 zfs destroy 命令
    let output = match Command::new("zfs")
        .args(["destroy", dataset])
        .output()
    {
        Ok(result) => result,
        Err(e) => {
            return HttpResponse::InternalServerError().json(DestroyDatasetResponse {
                success: false,
                message: "Failed to execute zfs destroy command".to_string(),
                error: Some(format!("Command error: {}", e)),
            });
        }
    };

    if output.status.success() {
        HttpResponse::Ok().json(DestroyDatasetResponse {
            success: true,
            message: format!("Successfully destroyed dataset '{}'", dataset),
            error: None,
        })
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        HttpResponse::InternalServerError().json(DestroyDatasetResponse {
            success: false,
            message: format!("Failed to destroy dataset '{}'", dataset),
            error: Some(stderr.to_string()),
        })
    }
}

/// Dataset 依赖信息结构体
#[derive(Serialize)]
pub struct DatasetDependInfo {
    pub name: String,
    pub mountpoint: String,
    pub canmount: String,
    pub origin: String,
}

/// Dataset 依赖列表响应结构体
#[derive(Serialize)]
pub struct DatasetDependsResponse {
    pub success: bool,
    pub data: Option<Vec<DatasetDependInfo>>,
    pub error: Option<String>,
}

/// ZFS Pool 详细信息响应结构体
#[derive(Serialize)]
pub struct ZpoolDetailInfo {
    pub name: String,
    pub size: String,
    pub alloc: String,
    pub free: String,
    pub ckpoint: String,
    pub expandsz: String,
    pub frag: String,
    pub cap: String,
    pub dedup: String,
    pub health: String,
    pub altroot: String,
    pub canmount: String,
    pub mountpoint: String,
}

impl From<crate::disk::zfs_utils::ZpoolInfo> for ZpoolDetailInfo {
    fn from(p: crate::disk::zfs_utils::ZpoolInfo) -> Self {
        Self {
            name: p.name,
            size: p.size,
            alloc: p.alloc,
            free: p.free,
            ckpoint: p.ckpoint,
            expandsz: p.expandsz,
            frag: p.frag,
            cap: p.cap,
            dedup: p.dedup,
            health: p.health,
            altroot: p.altroot,
            canmount: p.canmount,
            mountpoint: p.mountpoint,
        }
    }
}

/// ZFS Pools 响应结构体
#[derive(Serialize)]
pub struct ZpoolListResponse {
    pub success: bool,
    pub data: Option<Vec<ZpoolDetailInfo>>,
    pub error: Option<String>,
}

/// 获取 zfs list -r -o name,mountpoint,canmount,origin 命令的输出
fn get_zfs_depends_output() -> Option<String> {
    Command::new("zfs")
        .args(["list", "-r", "-o", "name,mountpoint,canmount,origin", "-H"])
        .output()
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
}

/// 从 zfs list 输出中解析依赖信息
fn parse_dataset_depends(output: &str) -> Vec<DatasetDependInfo> {
    let mut result = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // 解析行: "name\tmountpoint\tcanmount\torigin"
        let parts: Vec<&str> = trimmed.split('\t').collect();
        if parts.len() >= 4 {
            result.push(DatasetDependInfo {
                name: parts[0].to_string(),
                mountpoint: parts[1].to_string(),
                canmount: parts[2].to_string(),
                origin: parts[3].to_string(),
            });
        }
    }

    result
}

/// zfs/depends API - 获取所有 ZFS datasets 的依赖信息（需要 JWT 认证，普通用户可用）
#[get("/zfs/depends")]
pub async fn get_dataset_depends(req: HttpRequest, pool: web::Data<crate::DbPool>) -> impl Responder {
    // 验证 JWT token（普通用户即可）
    if let Err(response) = validate_token_with_db(&req, &pool).await {
        return response;
    }

    // 执行 zfs list 命令
    let output = match get_zfs_depends_output() {
        Some(output) => output,
        None => {
            return HttpResponse::InternalServerError().json(DatasetDependsResponse {
                success: false,
                data: None,
                error: Some("Failed to execute zfs list command".to_string()),
            });
        }
    };

    // 解析输出
    let datasets = parse_dataset_depends(&output);

    HttpResponse::Ok().json(DatasetDependsResponse {
        success: true,
        data: Some(datasets),
        error: None,
    })
}

/// zfs/online_pools API - 获取所有在线 ZFS pools（需要 JWT 认证）
#[get("/zfs/online_pools")]
pub async fn online_pools(req: HttpRequest, pool: web::Data<crate::DbPool>) -> impl Responder {
    // 验证 JWT token
    if let Err(response) = validate_token_with_db(&req, &pool).await {
        return response;
    }

    let pools = crate::disk::zfs_utils::get_online_pools();

    if pools.is_empty() {
        return HttpResponse::Ok().json(ZpoolListResponse {
            success: true,
            data: Some(Vec::new()),
            error: None,
        });
    }

    let pool_infos: Vec<ZpoolDetailInfo> = pools
        .into_iter()
        .map(|mut p| {
            // 获取 canmount 和 mountpoint 属性并合并到 pool 信息中
            if let Some(props) = crate::disk::zfs_utils::get_pool_extra_props(&p.name) {
                p.canmount = props.canmount;
                p.mountpoint = props.mountpoint;
            }
            p.into()
        })
        .collect();

    HttpResponse::Ok().json(ZpoolListResponse {
        success: true,
        data: Some(pool_infos),
        error: None,
    })
}

/// ZFS Offline Pools 响应结构体
#[derive(Serialize)]
pub struct OfflinePoolsResponse {
    pub success: bool,
    pub data: Option<Vec<String>>,
    pub error: Option<String>,
}

/// zfs/offline_pools API - 获取所有可导入但未在线的 ZFS pools（需要 JWT 认证）
#[get("/zfs/offline_pools")]
pub async fn offline_pools(req: HttpRequest, pool: web::Data<crate::DbPool>) -> impl Responder {
    // 验证 JWT token
    if let Err(response) = validate_token_with_db(&req, &pool).await {
        return response;
    }

    let pools = crate::disk::zfs_utils::get_offline_pools();

    HttpResponse::Ok().json(OfflinePoolsResponse {
        success: true,
        data: Some(pools),
        error: None,
    })
}

/// Import Pool 请求结构体
#[derive(Deserialize, Debug)]
pub struct ImportPoolRequest {
    pub poolname: String,
    pub dir: Option<String>,
    pub mount_on_startup: Option<bool>,
}

/// Import Pool 响应结构体
#[derive(Serialize)]
pub struct ImportPoolResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

/// zfs/import_pool API - 导入 ZFS pool（需要 JWT 认证）
/// 
/// 如果 dir 为 null，直接调用: zpool import poolname
/// 如果 dir 有值，调用: zpool import -R /dir poolname
/// 如果 mount_on_startup 为 true，挂载后执行:
///   zfs set mountpoint=/dir poolname
///   zfs set canmount=true poolname
#[post("/zfs/import_pool")]
pub async fn   import_pool(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
    import_req: web::Json<ImportPoolRequest>,
) -> impl Responder {
    // 验证 JWT token
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let poolname = &import_req.poolname;
    
    // 验证 poolname 不为空
    if poolname.is_empty() {
        return HttpResponse::BadRequest().json(ImportPoolResponse {
            success: false,
            message: "Pool name is required".to_string(),
            error: Some("poolname cannot be empty".to_string()),
        });
    }

    // 如果提供了 dir，验证是否为禁止目录
    if let Some(ref dir) = import_req.dir {
        if !dir.is_empty() {
            for &forbidden in FORBID_DIRECTORY {
                if dir == forbidden || dir.starts_with(&format!("{}/", forbidden)) || dir == "/" {
                    return HttpResponse::BadRequest().json(ImportPoolResponse {
                        success: false,
                        message: format!("Forbidden directory: {}", dir),
                        error: Some(format!("Directory '{}' is not allowed", forbidden)),
                    });
                }
            }
        }
    }
    if let Some(ref dir) = import_req.dir {
        if !dir.is_empty() {
            // 设置 mountpoint
            let mountpoint_result = Command::new("zfs")
                .args(["set", &format!("mountpoint={}", dir), poolname])
                .output();
            
            if let Err(e) = mountpoint_result {
                return HttpResponse::InternalServerError().json(ImportPoolResponse {
                    success: false,
                    message: format!("Pool '{}' imported but failed to set mountpoint", poolname),
                    error: Some(format!("Failed to set mountpoint: {}", e)),
                });
            }
        }
    }
    // 如果 mount_on_startup 为 true，设置 mountpoint 和 canmount
    if import_req.mount_on_startup == Some(true) {
                    // 设置 canmount=on
        let canmount_result = Command::new("zfs")
            .args(["set", "canmount=on", poolname])
            .output();
        
        if let Err(e) = canmount_result {
            return HttpResponse::InternalServerError().json(ImportPoolResponse {
                success: false,
                message: format!("Pool '{}' imported but failed to set canmount", poolname),
                error: Some(format!("Failed to set canmount: {}", e)),
            });
        }
    } else {
        // mount_on_startup 不为真时，设置 canmount=noauto
        let canmount_result = Command::new("zfs")
            .args(["set", "canmount=noauto", poolname])
            .output();
        
        if let Err(e) = canmount_result {
            return HttpResponse::InternalServerError().json(ImportPoolResponse {
                success: false,
                message: format!("Pool '{}' imported but failed to set canmount=off", poolname),
                error: Some(format!("Failed to set canmount=off: {}", e)),
            });
        }
    }

    // 构建 zpool import 命令
    let import_result = if let Some(ref dir) = import_req.dir {
        if dir.is_empty() {
            // dir 为空字符串时，等同于 null
            Command::new("zpool")
                .args(["import", poolname])
                .output()
        } else {
            // dir 有值时，使用 -R 选项
            Command::new("zpool")
                .args(["import", "-R", dir, poolname])
                .output()
        }
    } else {
        // dir 为 null
        Command::new("zpool")
            .args(["import", poolname])
            .output()
    };

    match import_result {
        Ok(output) => {
            if output.status.success() {

                HttpResponse::Ok().json(ImportPoolResponse {
                    success: true,
                    message: format!("Pool '{}' imported successfully", poolname),
                    error: None,
                })
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                HttpResponse::InternalServerError().json(ImportPoolResponse {
                    success: false,
                    message: format!("Failed to import pool '{}'", poolname),
                    error: Some(stderr.to_string()),
                })
            }
        }
        Err(e) => {
            HttpResponse::InternalServerError().json(ImportPoolResponse {
                success: false,
                message: format!("Failed to execute zpool import for '{}'", poolname),
                error: Some(e.to_string()),
            })
        }
    }
}

/// Export Pool 请求结构体
#[derive(Deserialize, Debug)]
pub struct ExportPoolRequest {
    pub poolname: String,
}

/// Export Pool 响应结构体
#[derive(Serialize)]
pub struct ExportPoolResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

/// zfs/export_pool API - 导出 ZFS pool（需要 JWT 认证）
#[post("/zfs/export_pool")]
pub async fn export_pool(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
    export_req: web::Json<ExportPoolRequest>,
) -> impl Responder {
    // 验证 JWT token
    let _admin_username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let poolname = &export_req.poolname;

    // 验证 poolname 不为空
    if poolname.is_empty() {
        return HttpResponse::BadRequest().json(ExportPoolResponse {
            success: false,
            message: "Pool name is required".to_string(),
            error: Some("poolname cannot be empty".to_string()),
        });
    }

    // 执行 zpool export 命令
    match Command::new("zpool")
        .args(["export", poolname])
        .output()
    {
        Ok(output) => {
            if output.status.success() {
                HttpResponse::Ok().json(ExportPoolResponse {
                    success: true,
                    message: format!("Pool '{}' exported successfully", poolname),
                    error: None,
                })
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                HttpResponse::InternalServerError().json(ExportPoolResponse {
                    success: false,
                    message: format!("Failed to export pool '{}'", poolname),
                    error: Some(stderr.to_string()),
                })
            }
        }
        Err(e) => {
            HttpResponse::InternalServerError().json(ExportPoolResponse {
                success: false,
                message: format!("Failed to execute zpool export for '{}'", poolname),
                error: Some(e.to_string()),
            })
        }
    }
}

/// Pool 高级设置属性结构体
#[derive(Serialize, Deserialize, Debug)]
pub struct PoolAdvancedSettings {
    pub primarycache: String,
    pub quota: String,
    pub mountpoint: String,
    pub recordsize: String,
    pub atime: String,
    pub relatime: String,
    pub readonly: String,
    pub aclmode: String,
    pub aclinherit: String,
    pub acltype: String,
    pub canmount: String,
    pub logbias: String,
    pub sync: String,
    pub compression: String,
    pub checksum: String,
}

/// Pool 高级设置 GET 响应结构体
#[derive(Serialize)]
pub struct PoolAdvancedSettingGetResponse {
    pub success: bool,
    pub data: Option<PoolAdvancedSettings>,
    pub error: Option<String>,
}

/// Pool 高级设置 POST 请求结构体
#[derive(Deserialize, Debug)]
pub struct PoolAdvancedSettingPostRequest {
    pub dataset: String,
    pub primarycache: Option<String>,
    pub quota: Option<String>,
    pub mountpoint: Option<String>,
    pub recordsize: Option<String>,
    pub atime: Option<String>,
    pub relatime: Option<String>,
    pub readonly: Option<String>,
    pub aclmode: Option<String>,
    pub aclinherit: Option<String>,
    pub acltype: Option<String>,
    pub canmount: Option<String>,
    pub logbias: Option<String>,
    pub sync: Option<String>,
    pub compression: Option<String>,
    pub checksum: Option<String>,
}

/// Pool 高级设置 POST 响应结构体
#[derive(Serialize)]
pub struct PoolAdvancedSettingPostResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

/// 获取 dataset 的高级属性
fn get_dataset_advanced_properties(dataset: &str) -> Option<PoolAdvancedSettings> {
    let props = [
        "primarycache", "quota", "mountpoint", "recordsize", "atime",
        "relatime", "readonly", "aclmode", "aclinherit", "acltype",
        "canmount", "logbias", "sync", "compression", "checksum",
    ];
    let props_str = props.join(",");
    
    let output = Command::new("zfs")
        .args(["get", &props_str, "-H", "-o", "property,value", dataset])
        .output()
        .ok()?;
    
    if !output.status.success() {
        return None;
    }
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut settings = PoolAdvancedSettings {
        primarycache: String::new(),
        quota: String::new(),
        mountpoint: String::new(),
        recordsize: String::new(),
        atime: String::new(),
        relatime: String::new(),
        readonly: String::new(),
        aclmode: String::new(),
        aclinherit: String::new(),
        acltype: String::new(),
        canmount: String::new(),
        logbias: String::new(),
        sync: String::new(),
        compression: String::new(),
        checksum: String::new(),
    };
    
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 2 {
            let property = parts[0].trim();
            let value = parts[1].trim();
            match property {
                "primarycache" => settings.primarycache = value.to_string(),
                "quota" => settings.quota = value.to_string(),
                "mountpoint" => settings.mountpoint = value.to_string(),
                "recordsize" => settings.recordsize = value.to_string(),
                "atime" => settings.atime = value.to_string(),
                "relatime" => settings.relatime = value.to_string(),
                "readonly" => settings.readonly = value.to_string(),
                "aclmode" => settings.aclmode = value.to_string(),
                "aclinherit" => settings.aclinherit = value.to_string(),
                "acltype" => settings.acltype = value.to_string(),
                "canmount" => settings.canmount = value.to_string(),
                "logbias" => settings.logbias = value.to_string(),
                "sync" => settings.sync = value.to_string(),
                "compression" => settings.compression = value.to_string(),
                "checksum" => settings.checksum = value.to_string(),
                _ => {}
            }
        }
    }
    
    Some(settings)
}

/// 设置单个 ZFS 属性
fn set_zfs_property(dataset: &str, property: &str, value: &str) -> Result<(), String> {
    let output = Command::new("zfs")
        .args(["set", &format!("{}={}", property, value), dataset])
        .output()
        .map_err(|e| format!("Command error: {}", e))?;
    
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

/// zfs/pool_advanced_setting GET API - 获取 ZFS dataset 的高级属性（需要 JWT 认证）
#[get("/zfs/pool_advanced_setting")]
pub async fn get_pool_advanced_setting(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
    query: web::Query<std::collections::HashMap<String, String>>,
) -> impl Responder {
    // 验证 JWT token
    if let Err(response) = validate_token_with_db(&req, &pool).await {
        return response;
    }

    // 获取 dataset 参数
    let dataset = match query.get("dataset") {
        Some(d) => d,
        None => {
            return HttpResponse::BadRequest().json(PoolAdvancedSettingGetResponse {
                success: false,
                data: None,
                error: Some("dataset parameter is required".to_string()),
            });
        }
    };

    // 验证 dataset 名称合法性
    if !dataset.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '/' || c == '@') {
        return HttpResponse::BadRequest().json(PoolAdvancedSettingGetResponse {
            success: false,
            data: None,
            error: Some("Dataset name must contain only alphanumeric characters, underscores, hyphens, dots, slashes, or @".to_string()),
        });
    }

    // 获取属性
    match get_dataset_advanced_properties(dataset) {
        Some(settings) => HttpResponse::Ok().json(PoolAdvancedSettingGetResponse {
            success: true,
            data: Some(settings),
            error: None,
        }),
        None => HttpResponse::InternalServerError().json(PoolAdvancedSettingGetResponse {
            success: false,
            data: None,
            error: Some(format!("Failed to get properties for dataset '{}'", dataset)),
        }),
    }
}

/// zfs/pool_advanced_setting POST API - 设置 ZFS dataset 的高级属性（需要 JWT 认证，仅管理员可用）
#[post("/zfs/pool_advanced_setting")]
pub async fn set_pool_advanced_setting(
    req: HttpRequest,
    pool: web::Data<crate::DbPool>,
    setting_req: web::Json<PoolAdvancedSettingPostRequest>,
) -> impl Responder {
    // 验证 JWT token 并检查 admin 权限
    let _username = match verify_admin_access(&req, &pool).await {
        Ok(username) => username,
        Err(response) => return response,
    };

    let dataset = &setting_req.dataset;

    // 验证 dataset 名称合法性
    if !dataset.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '/' || c == '@') {
        return HttpResponse::BadRequest().json(PoolAdvancedSettingPostResponse {
            success: false,
            message: "Invalid dataset name format".to_string(),
            error: Some("Dataset name must contain only alphanumeric characters, underscores, hyphens, dots, slashes, or @".to_string()),
        });
    }

    // 定义要设置的属性及其值
    let properties: Vec<(&str, Option<&String>)> = vec![
        ("primarycache", setting_req.primarycache.as_ref()),
        ("quota", setting_req.quota.as_ref()),
        ("mountpoint", setting_req.mountpoint.as_ref()),
        ("recordsize", setting_req.recordsize.as_ref()),
        ("atime", setting_req.atime.as_ref()),
        ("relatime", setting_req.relatime.as_ref()),
        ("readonly", setting_req.readonly.as_ref()),
        ("aclmode", setting_req.aclmode.as_ref()),
        ("aclinherit", setting_req.aclinherit.as_ref()),
        ("acltype", setting_req.acltype.as_ref()),
        ("canmount", setting_req.canmount.as_ref()),
        ("logbias", setting_req.logbias.as_ref()),
        ("sync", setting_req.sync.as_ref()),
        ("compression", setting_req.compression.as_ref()),
        ("checksum", setting_req.checksum.as_ref()),
    ];

    // 逐个设置属性
    for (prop, value_opt) in properties {
        if let Some(value) = value_opt {
            if let Err(e) = set_zfs_property(dataset, prop, value) {
                return HttpResponse::InternalServerError().json(PoolAdvancedSettingPostResponse {
                    success: false,
                    message: format!("Failed to set property '{}' to '{}'", prop, value),
                    error: Some(e),
                });
            }
        }
    }

    HttpResponse::Ok().json(PoolAdvancedSettingPostResponse {
        success: true,
        message: format!("Successfully updated advanced settings for dataset '{}'", dataset),
        error: None,
    })
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
