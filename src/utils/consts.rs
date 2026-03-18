/// Debian 系统根目录与子目录列表
/// 这些目录包含系统关键文件，禁止用户操作
pub const FORBID_DIRECTORY: &[&str] = &[
    // 系统启动相关
    "/boot",
    // 设备文件
    "/dev",
    // 系统配置
    "/etc",
    // 系统库文件
    "/lib",
    "/lib32",
    "/lib64",
    "/libx32",
    // 失物招领目录
    "/lost+found",
    // 挂载点
    // 可选软件包
    "/opt",
    // 进程信息
    "/proc",
    // 超级用户主目录
    "/root",
    // 运行时变量
    "/run",
    // 系统服务数据
    "/srv",
    // 系统信息
    "/sys",
    // 临时文件
    "/tmp",
    "/var",
    // 系统程序
    "/bin",
    "/sbin",
    // /usr 目录结构
    "/usr",
    "/var",
];
