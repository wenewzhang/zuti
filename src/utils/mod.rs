pub mod admin;
pub mod auth;
pub mod conf;
pub mod consts;
pub mod token_cache;

use std::process::Command;

pub use admin::{is_share_or_admin, verify_admin_access, verify_share_access};
pub use auth::{check_system_user, verify_password};
pub use conf::ensure_global_include;

/// 检查外部命令是否可用
/// 
/// 遍历 EXTERNAL_COMMANDS 列表中的每个命令，使用 `which` 检查是否可用
/// 
/// # Returns
/// - `Ok(())`: 所有命令都可用
/// - `Err(String)`: 有命令不可用，返回错误信息
pub fn check_external_commands() -> Result<(), Vec<String>> {
    let mut missing_commands = Vec::new();
    
    for cmd in consts::EXTERNAL_COMMANDS {
        match Command::new("which").arg(cmd).output() {
            Ok(output) if output.status.success() => {
                // 命令存在
            }
            _ => {
                log::warn!("External command '{}' not found", cmd);
                missing_commands.push(cmd.to_string());
            }
        }
    }
    
    if missing_commands.is_empty() {
        log::info!("All {} external commands are available", consts::EXTERNAL_COMMANDS.len());
        Ok(())
    } else {
        Err(missing_commands)
    }
}
