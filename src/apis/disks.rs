use actix_web::{get, post, web, HttpRequest, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use std::process::Command;

use crate::disk::get_free_disks as get_free_disk_list;
use crate::jwt::extract_and_validate_token;

// lsblk 输出项
#[derive(Serialize, Deserialize, Debug)]
pub struct DiskInfo {
    pub name: String,
    pub size: String,
    #[serde(rename = "type")]
    pub disk_type: String,
    pub mountpoint: Option<String>,
    pub children: Option<Vec<DiskInfo>>,
}

// get_disks 响应结构体
#[derive(Serialize)]
pub struct DisksResponse {
    pub success: bool,
    pub data: Option<Vec<DiskInfo>>,
    pub error: Option<String>,
}

// get_disks API - 返回硬盘信息（需要 JWT 认证）
#[get("/get_disks")]
pub async fn get_disks(req: HttpRequest) -> impl Responder {
    // 1. 验证 JWT token
    let _claims = match extract_and_validate_token(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };

    // 2. 执行 lsblk 命令获取硬盘信息
    let output = match Command::new("lsblk")
        .args(["-J", "-o", "NAME,SIZE,TYPE,MOUNTPOINT"])
        .output()
    {
        Ok(result) => result,
        Err(e) => {
            return HttpResponse::InternalServerError().json(DisksResponse {
                success: false,
                data: None,
                error: Some(format!("Failed to execute lsblk: {}", e)),
            });
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return HttpResponse::InternalServerError().json(DisksResponse {
            success: false,
            data: None,
            error: Some(format!("lsblk command failed: {}", stderr)),
        });
    }

    // 3. 解析 JSON 输出
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lsblk_output: serde_json::Value = match serde_json::from_str(&stdout) {
        Ok(val) => val,
        Err(e) => {
            return HttpResponse::InternalServerError().json(DisksResponse {
                success: false,
                data: None,
                error: Some(format!("Failed to parse lsblk output: {}", e)),
            });
        }
    };

    // 4. 提取 blockdevices 数组
    let blockdevices = match lsblk_output.get("blockdevices") {
        Some(devices) => match devices.as_array() {
            Some(arr) => arr,
            None => {
                return HttpResponse::InternalServerError().json(DisksResponse {
                    success: false,
                    data: None,
                    error: Some("Invalid lsblk output format".to_string()),
                });
            }
        },
        None => {
            return HttpResponse::InternalServerError().json(DisksResponse {
                success: false,
                data: None,
                error: Some("No blockdevices found in lsblk output".to_string()),
            });
        }
    };

    // 5. 解析为 DiskInfo 结构体
    let disks: Vec<DiskInfo> = match blockdevices
        .iter()
        .map(|d| serde_json::from_value::<DiskInfo>(d.clone()))
        .collect()
    {
        Ok(d) => d,
        Err(e) => {
            return HttpResponse::InternalServerError().json(DisksResponse {
                success: false,
                data: None,
                error: Some(format!("Failed to parse disk info: {}", e)),
            });
        }
    };

    // 6. 返回结果
    HttpResponse::Ok().json(DisksResponse {
        success: true,
        data: Some(disks),
        error: None,
    })
}

// get_free_disks 响应结构体
#[derive(Serialize)]
pub struct FreeDisksResponse {
    pub success: bool,
    pub data: Option<Vec<String>>,
    pub error: Option<String>,
}

// get_free_disks API - 返回空闲硬盘列表（需要 JWT 认证）
#[get("/get_free_disks")]
pub async fn get_free_disks(req: HttpRequest) -> impl Responder {
    // 1. 验证 JWT token
    let _claims = match extract_and_validate_token(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };

    // 2. 获取空闲硬盘列表
    let free_disks = get_free_disk_list();

    // 3. 返回结果
    HttpResponse::Ok().json(FreeDisksResponse {
        success: true,
        data: Some(free_disks),
        error: None,
    })
}

// format_disk 请求结构体
#[derive(Deserialize)]
pub struct FormatDiskRequest {
    pub disk_name: String,
}

// format_disk 响应结构体
#[derive(Serialize)]
pub struct DeleteDiskResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

// format_disk API - 格式化空闲硬盘（需要 JWT 认证）
#[post("/delete_disk")]
pub async fn format_disk(
    req: HttpRequest,
    format_req: web::Json<FormatDiskRequest>,
) -> impl Responder {
    // 1. 验证 JWT token
    let _claims = match extract_and_validate_token(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };

    let disk_name = &format_req.disk_name;

    // 2. 验证磁盘名称合法性（只允许字母数字）
    if !disk_name.chars().all(|c| c.is_alphanumeric()) {
        return HttpResponse::BadRequest().json(DeleteDiskResponse {
            success: false,
            message: "Invalid disk name format".to_string(),
            error: Some("Disk name must be alphanumeric".to_string()),
        });
    }

    // 3. 检查硬盘是否在空闲硬盘列表中
    let free_disks = get_free_disk_list();
    if !free_disks.contains(&disk_name.to_string()) {
        return HttpResponse::BadRequest().json(DeleteDiskResponse {
            success: false,
            message: format!("Disk '{}' is not available for formatting", disk_name),
            error: Some("Disk is either in use by ZFS or does not exist".to_string()),
        });
    }

    // 4. 执行格式化命令
    let device_path = format!("/dev/{}", disk_name);

    // 4.1 先用 zpool labelclear 清除 ZFS label（如果存在）
    let _ = Command::new("zpool")
        .args(["labelclear", "-f", &device_path])
        .output();

    // 4.2 用 dd 覆盖 ZFS label 所在的关键区域
    // ZFS label 位于: L0(0-256KB), L1(256KB-512KB), L2(磁盘末尾-256KB), L3(磁盘末尾-512KB到末尾-256KB)
    let dd_zero_1m = Command::new("dd")
        .args([
            "if=/dev/zero",
            &format!("of={}", device_path),
            "bs=1M",
            "count=16",
            "status=none",
        ])
        .output();

    if let Err(e) = dd_zero_1m {
        return HttpResponse::InternalServerError().json(DeleteDiskResponse {
            success: false,
            message: "Failed to clear disk header".to_string(),
            error: Some(format!("dd error: {}", e)),
        });
    }

    // 4.3 获取磁盘大小，清除末尾的 ZFS label
    let disk_size_output = Command::new("blockdev")
        .args(["--getsize64", &device_path])
        .output();

    if let Ok(output) = disk_size_output {
        if output.status.success() {
            let size_str = String::from_utf8_lossy(&output.stdout);
            if let Ok(size) = size_str.trim().parse::<u64>() {
                // 计算最后 4MB 的位置（覆盖 L2 和 L3 label）
                let skip_bytes = size.saturating_sub(4 * 1024 * 1024);
                let _ = Command::new("dd")
                    .args([
                        "if=/dev/zero",
                        &format!("of={}", device_path),
                        "bs=1M",
                        "seek=0",
                        &format!("skip={}", skip_bytes / (1024 * 1024)),
                        "count=4",
                        "status=none",
                    ])
                    .output();
            }
        }
    }

    // 4.4 用 wipefs -a 清除其他文件系统签名
    let _ = Command::new("wipefs")
        .args(["-a", &device_path])
        .output();

    // 4.5 用 sgdisk -Z 清空分区表
    let sgdisk_output = Command::new("sgdisk")
        .args(["-Z", &device_path])
        .output();

    match sgdisk_output {
        Ok(result) if result.status.success() => {
            HttpResponse::Ok().json(DeleteDiskResponse {
                success: true,
                message: format!(
                    "Disk '{}' fully cleared (ZFS labels, partition table and signatures removed)",
                    disk_name
                ),
                error: None,
            })
        }
        Ok(result) => {
            let stderr = String::from_utf8_lossy(&result.stderr);
            HttpResponse::InternalServerError().json(DeleteDiskResponse {
                success: false,
                message: format!("Failed to clear partition table on disk '{}'", disk_name),
                error: Some(stderr.to_string()),
            })
        }
        Err(e) => HttpResponse::InternalServerError().json(DeleteDiskResponse {
            success: false,
            message: "Failed to execute sgdisk command".to_string(),
            error: Some(format!("Command error: {}", e)),
        }),
    }
}
