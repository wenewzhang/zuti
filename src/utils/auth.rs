use pam::Client;

/// 验证用户密码（使用 PAM 认证）
/// 
/// # Arguments
/// * `username` - 用户名（如 "root" 或其他系统用户）
/// * `password` - 密码
/// 
/// # Returns
/// * `true` - 验证成功
/// * `false` - 验证失败
pub fn verify_password(username: &str, password: &str) -> bool {
    // 创建 PAM 客户端，使用系统默认的认证服务
    let mut client = match Client::with_password("system-auth") {
        Ok(client) => client,
        Err(_) => {
            // 如果 system-auth 不可用，尝试其他常见服务名
            match Client::with_password("login") {
                Ok(client) => client,
                Err(_) => return false,
            }
        }
    };

    // 设置用户名和密码
    client.conversation_mut().set_credentials(username, password);

    // 执行认证
    match client.authenticate() {
        Ok(_) => true,
        Err(_) => false,
    }
}
