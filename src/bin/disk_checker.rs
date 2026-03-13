use std::process::Command;
use std::collections::HashSet;

fn main() {
    // 获取所有磁盘
    let all_disks = get_all_disks();
    
    // 获取 ZFS 使用的磁盘
    let zfs_disks = get_zfs_disks();
    
    // 输出所有磁盘
    for disk in &all_disks {
        println!("{}", disk);
    }
    println!("----");
    
    // 输出 ZFS 磁盘
    for disk in &zfs_disks {
        println!("{}", disk);
    }
    println!("----");
    
    // 输出非 ZFS 磁盘
    let zfs_set: HashSet<String> = zfs_disks.into_iter().collect();
    for disk in &all_disks {
        if !zfs_set.contains(disk) {
            println!("/dev/{}", disk);
        }
    }
}

fn get_all_disks() -> Vec<String> {
    let output = Command::new("lsblk")
        .args(&["-d", "-n", "-o", "NAME"])
        .output()
        .expect("Failed to execute lsblk");
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    
    stdout
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .filter(|s| {
            s.starts_with("sd") || 
            s.starts_with("nvme") || 
            s.starts_with("hd") || 
            s.starts_with("vd")
        })
        .collect()
}

fn get_zfs_disks() -> Vec<String> {
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
