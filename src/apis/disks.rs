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
pub struct FormatDiskResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

// format_disk API - 格式化空闲硬盘（需要 JWT 认证）
#[post("/format_disk")]
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
        return HttpResponse::BadRequest().json(FormatDiskResponse {
            success: false,
            message: "Invalid disk name format".to_string(),
            error: Some("Disk name must be alphanumeric".to_string()),
        });
    }

    // 3. 检查硬盘是否在空闲硬盘列表中
    let free_disks = get_free_disk_list();
    if !free_disks.contains(&disk_name.to_string()) {
        return HttpResponse::BadRequest().json(FormatDiskResponse {
            success: false,
            message: format!("Disk '{}' is not available for formatting", disk_name),
            error: Some("Disk is either in use by ZFS or does not exist".to_string()),
        });
    }

    // 4. 执行格式化命令
    // 4.1 先用 wipefs -a 清除
    let device_path = format!("/dev/{}", disk_name);
    let wipefs_output = match Command::new("wipefs")
        .args(["-a", &device_path])
        .output()
    {
        Ok(result) => result,
        Err(e) => {
            return HttpResponse::InternalServerError().json(FormatDiskResponse {
                success: false,
                message: "Failed to execute wipefs command".to_string(),
                error: Some(format!("Command error: {}", e)),
            });
        }
    };

    if !wipefs_output.status.success() {
        let stderr = String::from_utf8_lossy(&wipefs_output.stderr);
        return HttpResponse::InternalServerError().json(FormatDiskResponse {
            success: false,
            message: format!("Failed to wipe disk '{}'", disk_name),
            error: Some(stderr.to_string()),
        });
    }

    // 4.2 再用 sgdisk -Z 清空分区表
    let sgdisk_output = match Command::new("sgdisk")
        .args(["-Z", &device_path])
        .output()
    {
        Ok(result) => result,
        Err(e) => {
            return HttpResponse::InternalServerError().json(FormatDiskResponse {
                success: false,
                message: "Failed to execute sgdisk command".to_string(),
                error: Some(format!("Command error: {}", e)),
            });
        }
    };

    // 5. 检查格式化结果
    if sgdisk_output.status.success() {
        HttpResponse::Ok().json(FormatDiskResponse {
            success: true,
            message: format!("Disk '{}' formatted successfully (wiped and partition table cleared)", disk_name),
            error: None,
        })
    } else {
        let stderr = String::from_utf8_lossy(&sgdisk_output.stderr);
        HttpResponse::InternalServerError().json(FormatDiskResponse {
            success: false,
            message: format!("Failed to clear partition table on disk '{}'", disk_name),
            error: Some(stderr.to_string()),
        })
    }
}
