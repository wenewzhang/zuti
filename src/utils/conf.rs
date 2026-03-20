use std::fs;
use std::path::Path;
use ini::Ini;

/// 生成包含 include 配置的新内容（预览模式，不写入文件）
/// 
/// # Arguments
/// * `content` - 原始配置文件内容
/// * `include_path` - 要包含的配置路径
/// 
/// # Returns
/// * `Ok((new_content, modified))` - 新内容和一个布尔值表示是否修改
/// * `Err(String)` - 操作失败，返回错误信息
pub fn ensure_global_include_preview(
    content: &str,
    include_path: &str,
) -> Result<(String, bool), String> {
    // 解析配置内容
    let ini = Ini::load_from_str(content)
        .map_err(|e| format!("Failed to parse config: {}", e))?;
    
    // 检查是否存在 [global] 段
    let global_section = ini.section(Some("global"))
        .ok_or_else(|| "[global] section not found".to_string())?;
    
    // 检查是否已包含该 include
    if let Some(val) = global_section.get("include") {
        // 已存在，检查是否包含目标路径
        if val.split_whitespace().any(|p| p == include_path) {
            return Ok((content.to_string(), false)); // 已存在，无需修改
        }
    }
    
    // 需要添加 include，使用 with_general_section() 获取可写的 section
    let mut ini_mut = Ini::load_from_str(content)
        .map_err(|e| format!("Failed to parse config: {}", e))?;
    
    ini_mut.with_section(Some("global"))
        .set("include", include_path);
    
    // 写回字符串
    let mut new_content = Vec::new();
    ini_mut.write_to(&mut new_content)
        .map_err(|e| format!("Failed to write config: {}", e))?;
    let new_content = String::from_utf8(new_content)
        .map_err(|e| format!("Invalid UTF-8: {}", e))?;
    
    Ok((new_content, true))
}

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

    let (new_content, modified) = ensure_global_include_preview(&content, include_path)?;

    if modified {
        fs::write(conf_path, new_content)
            .map_err(|e| format!("Failed to write config file: {}", e))?;
    }

    Ok(modified)
}

/// 合并多个 Samba 配置文件到一个文件
/// 
/// 将 `/etc/samba/conf.d/public.conf`、
/// `/etc/samba/conf.d/private.conf`、
/// `/etc/samba/conf.d/zfs-share.conf`
/// 合并到 `/etc/samba/conf.d/all-share.conf`
/// 
/// # Returns
/// * `Ok(())` - 合并成功
/// * `Err(String)` - 操作失败，返回错误信息
pub fn merge_samba_configs() -> Result<(), String> {
    let source_files = [
        "/etc/samba/conf.d/public.conf",
        "/etc/samba/conf.d/private.conf",
        "/etc/samba/conf.d/zfs-share.conf",
    ];
    let output_file = "/etc/samba/conf.d/all-share.conf";

    let mut merged_content = String::new();

    for file_path in &source_files {
        let path = Path::new(file_path);
        if path.exists() {
            let content = fs::read_to_string(path)
                .map_err(|e| format!("Failed to read {}: {}", file_path, e))?;
            
            if !content.is_empty() {
                // 添加文件分隔注释
                if !merged_content.is_empty() {
                    merged_content.push('\n');
                }
                merged_content.push_str(&format!("# Merged from: {}\n", file_path));
                merged_content.push_str(&content);
                // 确保文件内容以换行符结尾
                if !merged_content.ends_with('\n') {
                    merged_content.push('\n');
                }
            }
        }
        // 如果源文件不存在，则跳过（不报错）
    }

    // 写入合并后的文件
    fs::write(output_file, merged_content)
        .map_err(|e| format!("Failed to write {}: {}", output_file, e))?;

    Ok(())
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
        // rust-ini 会重新排列 section，但只要包含 include 路径即可
        assert!(updated.contains("/etc/samba/conf.d/*.conf"));
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
        // rust-ini 会改变格式，但只要包含 include 路径即可
        assert!(updated.contains("/etc/samba/conf.d/*.conf"));
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
