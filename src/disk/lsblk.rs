use std::process::Command;
use serde::{Deserialize, Serialize};

/// 磁盘基本信息结构体
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiskBasicInfo {
    pub name: String,
    pub size: String,
}

/// 获取系统中所有的物理磁盘
/// 
/// 使用 lsblk 命令列出所有磁盘设备（sd*, nvme*, hd*, vd*）及其大小
pub fn get_all_disks() -> Vec<DiskBasicInfo> {
    let output = Command::new("lsblk")
        .args(&["-d", "-n", "-o", "NAME,SIZE"])
        .output()
        .expect("Failed to execute lsblk");
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    
    stdout
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.trim().split_whitespace().collect();
            if parts.is_empty() {
                return None;
            }
            let name = parts[0].to_string();
            let size = parts.get(1).unwrap_or(&"").to_string();
            
            // 只保留特定类型的磁盘
            if name.starts_with("sd") || 
               name.starts_with("nvme") || 
               name.starts_with("hd") || 
               name.starts_with("vd") {
                Some(DiskBasicInfo { name, size })
            } else {
                None
            }
        })
        .collect()
}
