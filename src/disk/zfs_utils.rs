use std::process::Command;

/// Dataset 详细信息结构体
#[derive(Debug, Clone)]
pub struct DatasetDetail {
    pub name: String,
    pub used: String,
    pub avail: String,
    pub refer: String,
    pub mountpoint: String,
}

/// 获取 zfs list -t all 命令的输出
fn get_zfs_list_all_output() -> Option<String> {
    Command::new("zfs")
        .args(["list", "-t", "all", "-H"])
        .output()
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
}

/// 从 zfs list -t all 输出中解析所有 datasets
fn parse_all_datasets(output: &str) -> Vec<DatasetDetail> {
    let mut result = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // 解析行: "name\tused\tavail\trefer\tmountpoint"
        let parts: Vec<&str> = trimmed.split('\t').collect();
        if parts.len() >= 5 {
            result.push(DatasetDetail {
                name: parts[0].to_string(),
                used: parts[1].to_string(),
                avail: parts[2].to_string(),
                refer: parts[3].to_string(),
                mountpoint: parts[4].to_string(),
            });
        }
    }

    result
}

/// 获取所有 ZFS datasets (包括 snapshots 和 volumes)
pub fn get_datasets() -> Vec<DatasetDetail> {
    match get_zfs_list_all_output() {
        Some(output) => parse_all_datasets(&output),
        None => Vec::new(),
    }
}

/// 获取 zpool status 命令的原始输出
fn get_zpool_status_output() -> Option<String> {
    Command::new("zpool")
        .args(&["status", "-L"])
        .output()
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
}

/// 从 zpool status 输出中解析 ZFS 使用的磁盘
fn parse_zfs_disks(output: &str) -> Vec<String> {
    let mut result = Vec::new();
    
    for line in output.lines() {
        let trimmed = line.trim();
        // 匹配以 sd, hd, vd, nvme 开头的行
        if trimmed.starts_with("sd") || 
           trimmed.starts_with("hd") || 
           trimmed.starts_with("nvme") || 
           trimmed.starts_with("vd") {
            // 获取第一个字段（设备名）
            if let Some(device) = trimmed.split_whitespace().next() {
                let disk = if device.starts_with("nvme") {
                    // NVMe 设备: nvme0n1p1 -> nvme0n1, nvme0n1 -> nvme0n1
                    // 格式为 nvme{controller}n{namespace}p{partition}
                    // 基础磁盘名是 nvme{controller}n{namespace}
                    let re = regex::Regex::new(r"^nvme\d+n\d+").unwrap();
                    re.find(device).map(|m| m.as_str()).unwrap_or(device)
                } else {
                    // 其他设备 (sd, hd, vd): 移除末尾的数字
                    device.trim_end_matches(|c: char| c.is_ascii_digit())
                };
                if !disk.is_empty() && !result.contains(&disk.to_string()) {
                    result.push(disk.to_string());
                }
            }
        }
    }
    
    result
}

/// 获取 ZFS 正在使用的磁盘
/// 
/// 通过 zpool status -v 命令获取 ZFS 池使用的磁盘设备
pub fn get_zfs_disks() -> Vec<String> {
    match get_zpool_status_output() {
        Some(output) => parse_zfs_disks(&output),
        None => Vec::new(),
    }
}

/// ZFS Pool 信息结构体
#[derive(Debug, Clone)]
pub struct ZpoolInfo {
    pub name: String,
    pub size: String,
    pub alloc: String,
    pub free: String,
    pub ckpoint: String,
    pub expandsz: String,
    pub frag: String,
    pub cap: String,
    pub dedup: String,
    pub health: String,
    pub altroot: String,
}

/// 获取 zpool list 命令的输出
fn get_zpool_list_output() -> Option<String> {
    Command::new("zpool")
        .args(["list", "-H", "-o", "name,size,alloc,free,ckpoint,expandsz,frag,cap,dedup,health,altroot"])
        .output()
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
}

/// 从 zpool list 输出中解析所有 pools
fn parse_zpool_list(output: &str) -> Vec<ZpoolInfo> {
    let mut result = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let parts: Vec<&str> = trimmed.split('\t').collect();
        if parts.len() >= 11 {
            result.push(ZpoolInfo {
                name: parts[0].to_string(),
                size: parts[1].to_string(),
                alloc: parts[2].to_string(),
                free: parts[3].to_string(),
                ckpoint: parts[4].to_string(),
                expandsz: parts[5].to_string(),
                frag: parts[6].to_string(),
                cap: parts[7].to_string(),
                dedup: parts[8].to_string(),
                health: parts[9].to_string(),
                altroot: parts[10].to_string(),
            });
        }
    }

    result
}

/// 获取所有在线的 ZFS pools
pub fn get_online_pools() -> Vec<ZpoolInfo> {
    match get_zpool_list_output() {
        Some(output) => parse_zpool_list(&output),
        None => Vec::new(),
    }
}

/// 获取 zpool import 命令的输出
fn get_zpool_import_output() -> Option<String> {
    Command::new("zpool")
        .args(["import"])
        .output()
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
}

/// 从 zpool import 输出中解析所有可导入的 pools
fn parse_zpool_import(output: &str) -> Vec<String> {
    let mut result = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(pool_name) = trimmed.strip_prefix("pool: ") {
            let name = pool_name.trim().to_string();
            if !name.is_empty() && !result.contains(&name) {
                result.push(name);
            }
        }
    }

    result
}

/// 获取所有可导入但当前离线的 ZFS pools
pub fn get_offline_pools() -> Vec<String> {
    match get_zpool_import_output() {
        Some(output) => parse_zpool_import(&output),
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_zfs_disks_sda1() {
        let input = "sda1    ONLINE";
        let result = parse_zfs_disks(input);
        assert_eq!(result, vec!["sda"]);
    }

    #[test]
    fn test_parse_zfs_disks_sdb() {
        let input = "sdb    ONLINE";
        let result = parse_zfs_disks(input);
        assert_eq!(result, vec!["sdb"]);
    }

    #[test]
    fn test_parse_zfs_disks_nvme() {
        let input = "nvme0n1p1    ONLINE";
        let result = parse_zfs_disks(input);
        assert_eq!(result, vec!["nvme0n1"]);
    }

    #[test]
    fn test_parse_zfs_disks_nvme2() {
        let input = "nvme0n1    ONLINE";
        let result = parse_zfs_disks(input);
        assert_eq!(result, vec!["nvme0n1"]);
    }
    #[test]
    fn test_parse_zfs_disks_nvme3() {
        let input = "nvme1n9p9    ONLINE";
        let result = parse_zfs_disks(input);
        assert_eq!(result, vec!["nvme1n9"]);
    }

    #[test]
    fn test_parse_zfs_disks_nvme4() {
        let input = "nvme1n2    ONLINE";
        let result = parse_zfs_disks(input);
        assert_eq!(result, vec!["nvme1n2"]);
    }

    #[test]
    fn test_parse_all_datasets() {
        let input = "one-pool\t10G\t90G\t1G\t/one-pool\none-pool/ROOT\t2G\t90G\t1M\t/one-pool/ROOT\none-pool/ROOT/zuti\t8G\t90G\t8G\t/\none-pool@snapshot1\t100M\t-\t1G\t-\n";
        let result = parse_all_datasets(input);
        assert_eq!(result.len(), 4);
        assert_eq!(result[0].name, "one-pool");
        assert_eq!(result[0].used, "10G");
        assert_eq!(result[0].avail, "90G");
        assert_eq!(result[0].refer, "1G");
        assert_eq!(result[0].mountpoint, "/one-pool");
        assert_eq!(result[3].name, "one-pool@snapshot1");
        assert_eq!(result[3].mountpoint, "-");
    }

    #[test]
    fn test_parse_all_datasets_empty() {
        let input = "";
        let result = parse_all_datasets(input);
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_zpool_list() {
        let input = "one-pool\t100G\t10G\t90G\t-\t-\t0%\t10%\t1.00x\tONLINE\t-\n";
        let result = parse_zpool_list(input);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "one-pool");
        assert_eq!(result[0].size, "100G");
        assert_eq!(result[0].alloc, "10G");
        assert_eq!(result[0].free, "90G");
        assert_eq!(result[0].ckpoint, "-");
        assert_eq!(result[0].expandsz, "-");
        assert_eq!(result[0].frag, "0%");
        assert_eq!(result[0].cap, "10%");
        assert_eq!(result[0].dedup, "1.00x");
        assert_eq!(result[0].health, "ONLINE");
        assert_eq!(result[0].altroot, "-");
    }

    #[test]
    fn test_parse_zpool_list_multiple() {
        let input = "one-pool\t100G\t10G\t90G\t-\t-\t0%\t10%\t1.00x\tONLINE\t-\ntwo-pool\t200G\t50G\t150G\t-\t-\t5%\t25%\t1.00x\tONLINE\t/alt\n";
        let result = parse_zpool_list(input);
        assert_eq!(result.len(), 2);
        assert_eq!(result[1].name, "two-pool");
        assert_eq!(result[1].altroot, "/alt");
    }

    #[test]
    fn test_parse_zpool_list_empty() {
        let input = "";
        let result = parse_zpool_list(input);
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_zpool_import() {
        let input = "   pool: one-pool\n     id: 1234567890\n  state: ONLINE\n   pool: two-pool\n     id: 9876543210\n  state: UNAVAIL\n";
        let result = parse_zpool_import(input);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "one-pool");
        assert_eq!(result[1], "two-pool");
    }

    #[test]
    fn test_parse_zpool_import_empty() {
        let input = "no pools available to import\n";
        let result = parse_zpool_import(input);
        assert!(result.is_empty());
    }
}
