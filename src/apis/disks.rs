use actix_web::{get, post, web, HttpRequest, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use std::process::Command;

use crate::disk::get_free_disks as get_free_disk_list;
use crate::jwt::extract_and_validate_token;

// 分区信息结构体
#[derive(Serialize, Deserialize, Debug)]
pub struct PartitionInfo {
    pub name: String,
}

// get_free_parts 响应结构体
#[derive(Serialize)]
pub struct FreePartsResponse {
    pub success: bool,
    pub data: Option<Vec<PartitionInfo>>,
    pub error: Option<String>,
}

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

// part_disk 请求结构体
#[derive(Deserialize)]
pub struct PartDiskRequest {
    pub disk_name: String,
    pub size: String, // 例如: "10G", "500M", "100%"(使用剩余所有空间)
}

// part_disk 响应结构体
#[derive(Serialize)]
pub struct PartDiskResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

// part_disk API - 在硬盘上创建新的 ZFS 分区（需要 JWT 认证）
#[post("/part_disk")]
pub async fn part_disk(
    req: HttpRequest,
    part_req: web::Json<PartDiskRequest>,
) -> impl Responder {
    // 1. 验证 JWT token
    let _claims = match extract_and_validate_token(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };

    let disk_name = &part_req.disk_name;
    let size = &part_req.size;

    // 2. 验证磁盘名称合法性（只允许字母数字）
    if !disk_name.chars().all(|c| c.is_alphanumeric()) {
        return HttpResponse::BadRequest().json(PartDiskResponse {
            success: false,
            message: "Invalid disk name format".to_string(),
            error: Some("Disk name must be alphanumeric".to_string()),
        });
    }

    // 3. 验证 size 格式（支持 G/M/K 或百分比）
    // 格式: 数字 + 可选的单位(G/M/K) 或 数字 + % 或 0
    let size_lower = size.to_lowercase();
    let is_valid_size = if size_lower == "0" || size_lower == "100%" {
        true
    } else if size_lower.ends_with('%') {
        size_lower[..size_lower.len()-1].parse::<u64>().is_ok()
    } else if size_lower.ends_with('g') || size_lower.ends_with('m') || size_lower.ends_with('k') {
        size_lower[..size_lower.len()-1].parse::<u64>().is_ok()
    } else {
        size_lower.parse::<u64>().is_ok()
    };
    
    if !is_valid_size {
        return HttpResponse::BadRequest().json(PartDiskResponse {
            success: false,
            message: "Invalid size format".to_string(),
            error: Some("Size must be like '10G', '500M', '100%' or '0' (for remaining space)".to_string()),
        });
    }

    let device_path = format!("/dev/{}", disk_name);

    // 4. 获取当前分区信息以确定下一个分区号
    let parted_output = match Command::new("parted")
        .args(["-s", &device_path, "print"])
        .output()
    {
        Ok(result) => result,
        Err(e) => {
            return HttpResponse::InternalServerError().json(PartDiskResponse {
                success: false,
                message: "Failed to get partition info".to_string(),
                error: Some(format!("parted error: {}", e)),
            });
        }
    };

    // 解析 parted 输出获取最大分区号
    let parted_stdout = String::from_utf8_lossy(&parted_output.stdout);
    let mut max_part_num = 0;
    
    for line in parted_stdout.lines() {
        // 查找形如 " 1 " 或 " 1\t" 开头的行（分区号）
        let trimmed = line.trim();
        if let Some(first_char) = trimmed.chars().next() {
            if first_char.is_ascii_digit() {
                // 提取分区号
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                if let Some(num_str) = parts.first() {
                    if let Ok(num) = num_str.parse::<u32>() {
                        if num > max_part_num {
                            max_part_num = num;
                        }
                    }
                }
            }
        }
    }

    let next_part_num = max_part_num + 1;

    // 5. 使用 sgdisk 创建新分区
    // ZFS 分区类型 GUID: 6A898CC3-1DD2-11B2-99A6-080020736631 (Solaris /usr & Mac ZFS)
    // 或者使用短代码 bf01
    
    // 确定大小参数
    let size_arg = if size.to_lowercase() == "100%" || size == "0" {
        "0".to_string() // 0 表示使用剩余所有空间
    } else if size.ends_with('%') {
        // 处理百分比：1% 到 99%
        let percent_str = &size[..size.len()-1];
        if let Ok(percent) = percent_str.parse::<u64>() {
            if percent >= 1 && percent <= 99 {
                // 获取磁盘总大小（字节）
                match Command::new("blockdev")
                    .args(["--getsize64", &device_path])
                    .output() 
                {
                    Ok(output) => {
                        let size_str = String::from_utf8_lossy(&output.stdout);
                        if let Ok(total_bytes) = size_str.trim().parse::<u64>() {
                            // 计算百分比对应的大小（字节）
                            let calc_bytes = total_bytes * percent / 100;
                            // 转换为扇区数（假设扇区大小为512字节）
                            let sectors = calc_bytes / 512;
                            sectors.to_string()
                        } else {
                            return HttpResponse::InternalServerError().json(PartDiskResponse {
                                success: false,
                                message: "Failed to parse disk size".to_string(),
                                error: Some("Invalid blockdev output".to_string()),
                            });
                        }
                    }
                    Err(e) => {
                        return HttpResponse::InternalServerError().json(PartDiskResponse {
                            success: false,
                            message: "Failed to get disk size".to_string(),
                            error: Some(format!("blockdev error: {}", e)),
                        });
                    }
                }
            } else {
                return HttpResponse::BadRequest().json(PartDiskResponse {
                    success: false,
                    message: "Invalid percentage".to_string(),
                    error: Some("Percentage must be between 1 and 99".to_string()),
                });
            }
        } else {
            return HttpResponse::BadRequest().json(PartDiskResponse {
                success: false,
                message: "Invalid percentage format".to_string(),
                error: Some("Failed to parse percentage".to_string()),
            });
        }
    } else {
        size.clone()
    };

    // 构建 sgdisk 命令: -n <partnum>:<start>:<size> -t <partnum>:<type>
    // start=0 表示从第一个可用扇区开始
    let sgdisk_output = Command::new("sgdisk")
        .args([
            "-n", &format!("{}:0:{}", next_part_num, size_arg),
            "-t", &format!("{}:bf01", next_part_num),
            &device_path,
        ])
        .output();

    match sgdisk_output {
        Ok(result) if result.status.success() => {
            // 执行 partprobe 刷新分区表
            let _ = Command::new("partprobe").arg(&device_path).output();
            
            HttpResponse::Ok().json(PartDiskResponse {
                success: true,
                message: format!(
                    "Created ZFS partition {} on disk '{}' with size '{}'",
                    next_part_num, disk_name, size
                ),
                error: None,
            })
        }
        Ok(result) => {
            let stderr = String::from_utf8_lossy(&result.stderr);
            HttpResponse::InternalServerError().json(PartDiskResponse {
                success: false,
                message: format!("Failed to create partition on disk '{}'", disk_name),
                error: Some(stderr.to_string()),
            })
        }
        Err(e) => HttpResponse::InternalServerError().json(PartDiskResponse {
            success: false,
            message: "Failed to execute sgdisk command".to_string(),
            error: Some(format!("Command error: {}", e)),
        }),
    }
}

// get_free_parts API - 返回空闲分区列表（需要 JWT 认证）
#[get("/get_free_parts")]
pub async fn get_free_parts(req: HttpRequest) -> impl Responder {
    // 1. 验证 JWT token
    let _claims = match extract_and_validate_token(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };

    // 2. 执行 lsblk -fpJ 命令获取所有分区信息
    let output = match Command::new("lsblk")
        .args(["-fpJ"])
        .output()
    {
        Ok(result) => result,
        Err(e) => {
            return HttpResponse::InternalServerError().json(FreePartsResponse {
                success: false,
                data: None,
                error: Some(format!("Failed to execute lsblk: {}", e)),
            });
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return HttpResponse::InternalServerError().json(FreePartsResponse {
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
            return HttpResponse::InternalServerError().json(FreePartsResponse {
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
                return HttpResponse::InternalServerError().json(FreePartsResponse {
                    success: false,
                    data: None,
                    error: Some("Invalid lsblk output format".to_string()),
                });
            }
        },
        None => {
            return HttpResponse::InternalServerError().json(FreePartsResponse {
                success: false,
                data: None,
                error: Some("No blockdevices found in lsblk output".to_string()),
            });
        }
    };

    // 5. 遍历所有设备和其子分区，收集 fstype 为 null 的分区
    let mut free_parts: Vec<PartitionInfo> = Vec::new();

    for device in blockdevices {
        // 检查设备是否有 children（分区）
        if let Some(children) = device.get("children").and_then(|c| c.as_array()) {
            for child in children {
                // 检查 fstype 是否为 null
                if child.get("fstype").map(|f| f.is_null()).unwrap_or(true) {
                    // 解析分区信息
                    let name = child
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("")
                        .to_string();

                    free_parts.push(PartitionInfo { name });
                }
            }
        }
    }

    // 6. 返回结果
    HttpResponse::Ok().json(FreePartsResponse {
        success: true,
        data: Some(free_parts),
        error: None,
    })
}
