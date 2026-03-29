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
}
