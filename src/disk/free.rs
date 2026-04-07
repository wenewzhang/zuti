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
