use std::process::Command;

/// 获取系统中所有的物理磁盘
/// 
/// 使用 lsblk 命令列出所有磁盘设备（sd*, nvme*, hd*, vd*）
pub fn get_all_disks() -> Vec<String> {
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
