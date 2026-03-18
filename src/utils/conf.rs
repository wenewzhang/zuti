use std::fs;
use std::path::Path;

/// 检查并更新配置文件，在 [global] 段内添加 include 配置
/// 
/// # Arguments
/// * `conf_path` - 配置文件路径
/// * `include_path` - 要包含的配置路径
/// 
/// # Returns
/// * `Ok(true)` - 成功添加了 include 配置
/// * `Ok(false)` - 文件已包含该配置，无需修改
/// * `Err(String)` - 操作失败，返回错误信息
pub fn ensure_global_include<P: AsRef<Path>>(
    conf_path: P,
    include_path: &str,
) -> Result<bool, String> {
    let conf_path = conf_path.as_ref();
    
    if !conf_path.exists() {
        return Err(format!("Config file not found: {}", conf_path.display()));
    }

    let content = fs::read_to_string(conf_path)
        .map_err(|e| format!("Failed to read config file: {}", e))?;

    // 检查是否已包含该 include
    let search_pattern = format!("include = {}", include_path);
    if content.contains(&search_pattern) {
        return Ok(false); // 已存在，无需修改
    }

    // 查找 [global] 段的位置
    let global_start = content
        .find("[global]")
        .ok_or_else(|| "[global] section not found".to_string())?;

    // 找到 [global] 段之后下一个 "\n[" 的位置，即下一个 share 的开始
    let after_global = &content[global_start + 8..];
    let next_section = after_global.find("\n[");

    let insert_pos = match next_section {
        Some(pos) => global_start + 8 + pos + 1, // 在换行后插入
        None => content.len(),                   // 如果没有下一个段，插到文件末尾
    };

    let include_line = format!("include = {}\n", include_path);
    let new_content = format!(
        "{}{}{}",
        &content[..insert_pos],
        include_line,
        &content[insert_pos..]
    );

    fs::write(conf_path, new_content)
        .map_err(|e| format!("Failed to write config file: {}", e))?;

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_ensure_global_include_adds_include_to_global_section() {
        let mut temp_file = NamedTempFile::new().unwrap();
        let content = r#"[global]
workgroup = WORKGROUP
server string = Samba Server

[share1]
path = /path/to/share1
"#;
        temp_file.write_all(content.as_bytes()).unwrap();

        let result = ensure_global_include(
            temp_file.path(),
            "/etc/samba/conf.d/*.conf",
        );

        assert!(result.is_ok());
        assert!(result.unwrap()); // 返回 true 表示已修改

        let updated = fs::read_to_string(temp_file.path()).unwrap();
        assert!(updated.contains("include = /etc/samba/conf.d/*.conf"));
        // 确保 include 在 [global] 段内，在 [share1] 之前
        let global_pos = updated.find("[global]").unwrap();
        let include_pos = updated.find("include =").unwrap();
        let share1_pos = updated.find("[share1]").unwrap();
        assert!(global_pos < include_pos);
        assert!(include_pos < share1_pos);
    }

    #[test]
    fn test_ensure_global_include_skips_if_already_exists() {
        let mut temp_file = NamedTempFile::new().unwrap();
        let content = r#"[global]
workgroup = WORKGROUP
include = /etc/samba/conf.d/*.conf

[share1]
path = /path/to/share1
"#;
        temp_file.write_all(content.as_bytes()).unwrap();

        let result = ensure_global_include(
            temp_file.path(),
            "/etc/samba/conf.d/*.conf",
        );

        assert!(result.is_ok());
        assert!(!result.unwrap()); // 返回 false 表示无需修改
    }

    #[test]
    fn test_ensure_global_include_appends_to_end_when_no_other_sections() {
        let mut temp_file = NamedTempFile::new().unwrap();
        let content = r#"[global]
workgroup = WORKGROUP
server string = Samba Server
"#;
        temp_file.write_all(content.as_bytes()).unwrap();

        let result = ensure_global_include(
            temp_file.path(),
            "/etc/samba/conf.d/*.conf",
        );

        assert!(result.is_ok());
        assert!(result.unwrap());

        let updated = fs::read_to_string(temp_file.path()).unwrap();
        assert!(updated.contains("include = /etc/samba/conf.d/*.conf"));
        // 确保在最后
        assert!(updated.trim_end().ends_with("*.conf"));
    }

    #[test]
    fn test_ensure_global_include_returns_error_when_no_global_section() {
        let mut temp_file = NamedTempFile::new().unwrap();
        let content = r#"[share1]
path = /path/to/share1
"#;
        temp_file.write_all(content.as_bytes()).unwrap();

        let result = ensure_global_include(
            temp_file.path(),
            "/etc/samba/conf.d/*.conf",
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("[global] section not found"));
    }

    #[test]
    fn test_ensure_global_include_returns_error_when_file_not_found() {
        let result = ensure_global_include(
            "/nonexistent/path/smb.conf",
            "/etc/samba/conf.d/*.conf",
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Config file not found"));
    }
}
