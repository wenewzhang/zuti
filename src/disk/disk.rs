use crate::disk::lsblk::{get_all_disks, DiskBasicInfo};
use crate::disk::zfs_utils::get_zfs_disks;

/// 获取空闲硬盘列表
///
/// 通过从所有硬盘中排除 ZFS 正在使用的硬盘，返回未被使用的硬盘列表
pub fn get_free_disks() -> Vec<DiskBasicInfo> {
    let all_disks = get_all_disks();
    let zfs_disks = get_zfs_disks();

    all_disks
        .into_iter()
        .filter(|disk| !zfs_disks.contains(&disk.name))
        .collect()
}

/// 检查指定硬盘是否空闲
///
/// 如果硬盘不在 ZFS 使用的硬盘列表中，则返回 true
pub fn is_disk_free(disk_name: &str) -> bool {
    let zfs_disks = get_zfs_disks();
    !zfs_disks.contains(&disk_name.to_string())
}

/// 在 /dev/disk/by-id/ 下查找设备的长 ID
pub fn find_disk_by_id(device: &str) -> Result<String, String> {
    let is_partition = device.chars().last().map(|c| c.is_ascii_digit()).unwrap_or(false);
    let device_path = format!("/dev/{}", device);
    
    let entries = match std::fs::read_dir("/dev/disk/by-id/") {
        Ok(entries) => entries,
        Err(e) => return Err(format!("Failed to read /dev/disk/by-id/: {}", e)),
    };
    
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        
        let path = entry.path();
        let file_name = match entry.file_name().into_string() {
            Ok(name) => name,
            Err(_) => continue,
        };
        
        if file_name.starts_with("scsi-") || file_name.starts_with("ata-") || 
           file_name.starts_with("nvme-") || file_name.starts_with("wwn-") {
            match std::fs::canonicalize(&path) {
                Ok(real_path) => {
                    if is_partition {
                        if real_path.to_string_lossy().ends_with(device) {
                            if file_name.starts_with("ata-") || file_name.starts_with("nvme-eui.") {
                                return Ok(path.to_string_lossy().to_string());
                            }
                        }
                    } else {
                        let real_path_str = real_path.to_string_lossy();
                        if real_path_str == device_path {
                            if file_name.starts_with("ata-") || 
                               (file_name.starts_with("nvme-") && !file_name.contains("-part")) {
                                return Ok(path.to_string_lossy().to_string());
                            }
                        }
                    }
                }
                Err(_) => continue,
            }
        }
    }
    
    Err(format!("Cannot find long ID for device '{}' in /dev/disk/by-id/", device))
}

/// 查找设备的分区 long ID
pub fn find_partition_by_id(disk_name: &str, part_suffix: &str) -> Result<String, String> {
    let device_path = format!("/dev/{}{}", disk_name, part_suffix);
    
    let entries = match std::fs::read_dir("/dev/disk/by-id/") {
        Ok(entries) => entries,
        Err(e) => return Err(format!("Failed to read /dev/disk/by-id/: {}", e)),
    };
    
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        
        let path = entry.path();
        let file_name = match entry.file_name().into_string() {
            Ok(name) => name,
            Err(_) => continue,
        };
        
        if file_name.contains("-part") {
            match std::fs::canonicalize(&path) {
                Ok(real_path) => {
                    if real_path.to_string_lossy() == device_path {
                        return Ok(path.to_string_lossy().to_string());
                    }
                }
                Err(_) => continue,
            }
        }
    }
    
    Err(format!("Cannot find partition ID for '{}{}'", disk_name, part_suffix))
}

/// 获取设备的 by-id 路径
pub fn get_device_by_id(device: &str) -> Result<String, String> {
    let is_partition = device.chars().last().map(|c| c.is_ascii_digit()).unwrap_or(false);
    
    if is_partition {
        if device.starts_with("nvme") {
            if let Some(pos) = device.rfind('p') {
                let disk_name = &device[..pos];
                let part_suffix = &device[pos..]; // 包含 p
                return find_partition_by_id(disk_name, part_suffix);
            }
        } else {
            let chars: Vec<char> = device.chars().collect();
            let mut num_start = chars.len();
            for (i, c) in chars.iter().enumerate().rev() {
                if c.is_ascii_digit() {
                    num_start = i;
                } else {
                    break;
                }
            }
            if num_start < chars.len() {
                let disk_name: String = chars[..num_start].iter().collect();
                let part_num: String = chars[num_start..].iter().collect();
                return find_partition_by_id(&disk_name, &part_num);
            }
        }
    }
    
    find_disk_by_id(device)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_free_disks_returns_valid_list() {
        // 此测试仅验证函数不 panic，实际结果取决于系统环境
        let free_disks = get_free_disks();
        // 结果应该是一个磁盘信息向量
        for disk in &free_disks {
            assert!(!disk.name.is_empty());
        }
    }

    #[test]
    fn test_is_disk_free_no_panic() {
        // 测试函数不会 panic
        let _ = is_disk_free("sda");
        let _ = is_disk_free("nvme0n1");
    }
}
