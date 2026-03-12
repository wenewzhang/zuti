use actix_web::{get, HttpRequest, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use std::process::Command;

use crate::extract_and_validate_token;

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
