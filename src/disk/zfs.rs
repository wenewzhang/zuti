use std::process::Command;

/// 获取 ZFS 正在使用的磁盘
/// 
/// 通过 zpool status -v 命令获取 ZFS 池使用的磁盘设备
pub fn get_zfs_disks() -> Vec<String> {
    let output = Command::new("zpool")
        .args(&["status", "-v"])
        .output();
    
    let mut result = Vec::new();
    
    if let Ok(output) = output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        
        for line in stdout.lines() {
            let trimmed = line.trim();
            // 匹配以 sd, hd, vd 开头的行
            if trimmed.starts_with("sd") || 
               trimmed.starts_with("hd") || 
               trimmed.starts_with("vd") {
                // 获取第一个字段（设备名）
                if let Some(device) = trimmed.split_whitespace().next() {
                    // 移除末尾的数字，获取磁盘名称
                    let disk = device.trim_end_matches(|c: char| c.is_ascii_digit());
                    if !disk.is_empty() && !result.contains(&disk.to_string()) {
                        result.push(disk.to_string());
                    }
                }
            }
        }
    }
    
    result
}
