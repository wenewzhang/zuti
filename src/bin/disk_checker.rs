use std::collections::HashSet;

use zuti::disk::{get_all_disks, get_zfs_disks};

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
