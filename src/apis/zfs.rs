use actix_web::{get, HttpRequest, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use std::process::Command;

use crate::utils::admin::validate_token_with_db;

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

/// 获取当前 / 挂载的 bootfs (findmnt -n -o SOURCE /)
fn get_bootfs_source() -> Option<String> {
    Command::new("findmnt")
        .args(["-n", "-o", "SOURCE", "/"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let trimmed = stdout.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
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
pub async fn get_bootfs(req: HttpRequest, pool: actix_web::web::Data<crate::DbPool>) -> impl Responder {
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
