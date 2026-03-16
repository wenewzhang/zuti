#!/bin/bash
set -e

echo "=== Building zuti deb package with cargo-deb ==="

# 检查 cargo-deb 是否安装
if ! command -v cargo-deb &> /dev/null; then
    echo "Installing cargo-deb..."
    cargo install cargo-deb
fi

# 发布构建
echo "Building release binary..."
cargo build --release

# 创建 debian 目录结构
echo "Preparing debian package files..."
mkdir -p debian

# 复制 systemd 服务文件
cp debian/zuti.service debian/zuti.service.bak 2>/dev/null || true
cp debian/scripts/* debian/ 2>/dev/null || true

# 生成证书生成脚本到 postinst
cat > debian/postinst << 'POSTINST_EOF'
#!/bin/bash
set -e

CERT_DIR="/etc/zuti/certs"
KEY_FILE="${CERT_DIR}/server.key"
CERT_FILE="${CERT_DIR}/server.crt"

# 设置权限
chown -R zuti:zuti /etc/zuti 2>/dev/null || chown -R root:root /etc/zuti
chown -R zuti:zuti /var/lib/zuti 2>/dev/null || chown -R root:root /var/lib/zuti
chown -R zuti:zuti /var/log/zuti 2>/dev/null || chown -R root:root /var/log/zuti

# 生成 SSL 证书（如果不存在）
if [ ! -f "${CERT_FILE}" ] || [ ! -f "${KEY_FILE}" ]; then
    echo "Generating SSL certificate for zuti..."
    
    openssl genrsa -out "${KEY_FILE}" 2048 2>/dev/null
    openssl req -new -x509 -key "${KEY_FILE}" -out "${CERT_FILE}" -days 3650 \
        -subj "/C=CN/ST=State/L=City/O=Zuti/CN=localhost" 2>/dev/null
    
    chmod 600 "${KEY_FILE}"
    chmod 644 "${CERT_FILE}"
    chown zuti:zuti "${KEY_FILE}" "${CERT_FILE}" 2>/dev/null || chown root:root "${KEY_FILE}" "${CERT_FILE}"
    
    echo "SSL certificate generated at ${CERT_DIR}"
else
    echo "SSL certificates already exist, skipping generation."
fi

# 配置 .env 文件
if [ ! -f /etc/zuti/.env ]; then
    if [ -f /etc/zuti/.env.example ]; then
        cp /etc/zuti/.env.example /etc/zuti/.env
        sed -i 's|^CERT_PATH=.*|CERT_PATH=/etc/zuti/certs/server.crt|' /etc/zuti/.env
        sed -i 's|^KEY_PATH=.*|KEY_PATH=/etc/zuti/certs/server.key|' /etc/zuti/.env
    fi
fi

# 启动服务
if command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload
    systemctl enable zuti.service
    systemctl start zuti.service || true
fi

echo ""
echo "=========================================="
echo "zuti installed successfully!"
echo "=========================================="
echo "Configuration: /etc/zuti/.env"
echo "Certificates:  ${CERT_DIR}"
echo "Data:          /var/lib/zuti"
echo ""
echo "Service: systemctl {start|stop|status} zuti"
echo "Logs:    journalctl -u zuti -f"
echo "=========================================="

exit 0
POSTINST_EOF

# 创建 preinst
cat > debian/preinst << 'PREINST_EOF'
#!/bin/bash
set -e
if ! id -u zuti >/dev/null 2>&1; then
    useradd -r -s /bin/false -M -d /var/lib/zuti zuti 2>/dev/null || true
fi
mkdir -p /etc/zuti/certs /var/lib/zuti /var/log/zuti
exit 0
PREINST_EOF

# 创建 prerm
cat > debian/prerm << 'PRERM_EOF'
#!/bin/bash
set -e
if command -v systemctl >/dev/null 2>&1; then
    systemctl stop zuti.service || true
    systemctl disable zuti.service || true
fi
exit 0
PRERM_EOF

# 创建 postrm
cat > debian/postrm << 'POSTRM_EOF'
#!/bin/bash
set -e
if [ "$1" = "purge" ]; then
    rm -rf /var/lib/zuti /var/log/zuti
    echo "zuti data has been removed."
fi
exit 0
POSTRM_EOF

# 设置权限
chmod 755 debian/preinst debian/postinst debian/prerm debian/postrm

# 构建 deb 包
echo "Building deb package..."
cargo deb

# 清理临时文件（保留 service 文件）
rm -f debian/preinst debian/postinst debian/prerm debian/postrm
if [ -f debian/zuti.service.bak ]; then
    mv debian/zuti.service.bak debian/zuti.service
fi

echo ""
echo "=== Build complete ==="
ls -lh target/debian/*.deb 2>/dev/null || ls -lh target/debian/
