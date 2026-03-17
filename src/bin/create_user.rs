use std::env;
use std::process::{Command, Stdio};
use std::io::Write;

fn main() {
    let args: Vec<String> = env::args().collect();
    
    if args.len() != 3 {
        eprintln!("Usage: {} <username> <password>", args[0]);
        std::process::exit(1);
    }
    
    let username = &args[1];
    let password = &args[2];
    
    // 验证用户名合法性（只允许字母数字、下划线）
    if !username.chars().all(|c| c.is_alphanumeric() || c == '_') {
        eprintln!("Error: Username must contain only alphanumeric characters or underscores");
        std::process::exit(1);
    }
    
    // 验证用户名长度
    if username.len() < 1 || username.len() > 32 {
        eprintln!("Error: Username length must be between 1 and 32 characters");
        std::process::exit(1);
    }
    
    // 检查用户是否已存在
    if check_user_exists(username) {
        eprintln!("Error: User '{}' already exists", username);
        std::process::exit(1);
    }
    
    // 创建 Linux 系统用户
    println!("Creating Linux system user '{}'...", username);
    match create_linux_user(username, password) {
        Ok(_) => println!("Linux system user created successfully"),
        Err(e) => {
            eprintln!("Error creating Linux user: {}", e);
            std::process::exit(1);
        }
    }
    
    // 创建 Samba 用户
    println!("Creating Samba user '{}'...", username);
    match create_samba_user(username, password) {
        Ok(_) => println!("Samba user created successfully"),
        Err(e) => {
            eprintln!("Error creating Samba user: {}", e);
            // 尝试回滚：删除已创建的 Linux 用户
            let _ = Command::new("userdel")
                .arg(username)
                .output();
            std::process::exit(1);
        }
    }
    
    // 启用 Samba 用户（确保用户是启用状态）
    match enable_samba_user(username) {
        Ok(_) => println!("Samba user enabled successfully"),
        Err(e) => {
            eprintln!("Warning: Failed to enable Samba user: {}", e);
        }
    }
    
    println!("\nUser '{}' created successfully!", username);
    println!("  - Linux system user: created");
    println!("  - Samba user: created and enabled");
}

/// 检查 Linux 用户是否已存在
fn check_user_exists(username: &str) -> bool {
    match Command::new("id").arg(username).output() {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

/// 创建 Linux 系统用户并设置密码
fn create_linux_user(username: &str, password: &str) -> Result<(), String> {
    // 1. 创建系统用户（普通用户，可登录）
    let output = Command::new("useradd")
        .args(["-m", "-s", "/bin/bash", username])
        .output()
        .map_err(|e| format!("Failed to execute useradd: {}", e))?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("useradd failed: {}", stderr));
    }
    
    // 2. 设置用户密码（使用 chpasswd）
    let passwd_input = format!("{}:{}", username, password);
    let mut child = Command::new("chpasswd")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to execute chpasswd: {}", e))?;
    
    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(passwd_input.as_bytes())
            .map_err(|e| format!("Failed to write password: {}", e))?;
    }
    
    let output = child
        .wait_with_output()
        .map_err(|e| format!("Failed to wait for chpasswd: {}", e))?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("chpasswd failed: {}", stderr));
    }
    
    Ok(())
}

/// 创建 Samba 用户
fn create_samba_user(username: &str, password: &str) -> Result<(), String> {
    // 使用 smbpasswd -a 添加用户
    // smbpasswd 从 stdin 读取密码
    let mut child = Command::new("smbpasswd")
        .args(["-a", username, "-s"])  // -s 表示从 stdin 读取密码
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to execute smbpasswd: {}", e))?;
    
    // smbpasswd 需要输入两次密码（密码和确认密码）
    let input = format!("{}\n{}\n", password, password);
    
    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(input.as_bytes())
            .map_err(|e| format!("Failed to write password to smbpasswd: {}", e))?;
    }
    
    let output = child
        .wait_with_output()
        .map_err(|e| format!("Failed to wait for smbpasswd: {}", e))?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!("smbpasswd failed: stderr={}, stdout={}", stderr, stdout));
    }
    
    Ok(())
}

/// 启用 Samba 用户（解除禁用状态）
fn enable_samba_user(username: &str) -> Result<(), String> {
    let output = Command::new("smbpasswd")
        .args(["-e", username])  // -e 表示启用用户
        .output()
        .map_err(|e| format!("Failed to execute smbpasswd -e: {}", e))?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to enable Samba user: {}", stderr));
    }
    
    Ok(())
}
