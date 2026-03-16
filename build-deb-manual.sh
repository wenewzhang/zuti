#!/bin/bash
set -e

# 手动构建 deb 包（不需要 cargo-deb）

VERSION="0.1.0"
ARCH="amd64"
PKG_NAME="zuti_${VERSION}_${ARCH}"
BUILD_DIR="target/deb-build/${PKG_NAME}"

echo "=== Manual deb package build ==="

# 清理旧构建
rm -rf "target/deb-build"
mkdir -p "${BUILD_DIR}/DEBIAN"
mkdir -p "${BUILD_DIR}/usr/bin"
mkdir -p "${BUILD_DIR}/etc/zuti/certs"
mkdir -p "${BUILD_DIR}/lib/systemd/system"
mkdir -p "${BUILD_DIR}/usr/share/zuti/migrations"
mkdir -p "${BUILD_DIR}/var/lib/zuti"
mkdir -p "${BUILD_DIR}/var/log/zuti"

# 构建 release 二进制
echo "Building release binary..."
cargo build --release

# 复制文件
echo "Copying files..."
cp "target/release/zuti" "${BUILD_DIR}/usr/bin/"
# 证书将在安装时由 openssl 生成，不在此处复制
cp "migrations/"* "${BUILD_DIR}/usr/share/zuti/migrations/" 2>/dev/null || true
cp ".env.example" "${BUILD_DIR}/etc/zuti/" 2>/dev/null || true
cp "debian/zuti.service" "${BUILD_DIR}/lib/systemd/system/"

mkdir -p db
diesel database setup --database-url=sqlite://./db/db.sqlite
# 创建 control 文件
cat > "${BUILD_DIR}/DEBIAN/control" << EOF
Package: zuti
Version: ${VERSION}
Section: admin
Priority: optional
Architecture: ${ARCH}
Maintainer: Wenew Zhang<wenewboy@gmail.com>
Description: Zuti - Storage management web service
 A web service for disk and storage pool management.
 Built with Rust and Actix-web.
Depends: libc6, libssl3 | libssl1.1, libpam0g, openssl
EOF

# 创建维护脚本 - preinst
cat > "${BUILD_DIR}/DEBIAN/preinst" << 'EOF'
#!/bin/bash
set -e
if ! id -u zuti >/dev/null 2>&1; then
    useradd -r -s /bin/false -M -d /var/lib/zuti zuti 2>/dev/null || true
fi
mkdir -p /etc/zuti/certs /var/lib/zuti /var/log/zuti
exit 0
EOF

# 创建维护脚本 - postinst（包含证书生成）
cat > "${BUILD_DIR}/DEBIAN/postinst" << 'EOF'
#!/bin/bash
set -e

CERT_DIR="/etc/zuti/certs"
KEY_FILE="${CERT_DIR}/server.key"
CERT_FILE="${CERT_DIR}/server.crt"

# 设置目录权限
chown -R root:root /etc/zuti 2>/dev/null || chown -R root:root /etc/zuti
chown -R root:root /var/lib/zuti 2>/dev/null || chown -R root:root /var/lib/zuti
chown -R root:root /var/log/zuti 2>/dev/null || chown -R root:root /var/log/zuti

# 生成 SSL 证书（如果不存在）
if [ ! -f "${CERT_FILE}" ] || [ ! -f "${KEY_FILE}" ]; then
    echo "Generating SSL certificate for zuti..."
    
    # 生成私钥
    openssl genrsa -out "${KEY_FILE}" 2048 2>/dev/null
    
    # 生成自签名证书（有效期 3650 天 = 10 年）
    openssl req -new -x509 -key "${KEY_FILE}" -out "${CERT_FILE}" -days 3650 \
        -subj "/C=CN/ST=State/L=City/O=Zuti/CN=localhost" 2>/dev/null
    
    # 设置证书权限
    chmod 600 "${KEY_FILE}"
    chmod 644 "${CERT_FILE}"
    chown root:root "${KEY_FILE}" "${CERT_FILE}" 2>/dev/null || chown root:root "${KEY_FILE}" "${CERT_FILE}"
    
    echo "SSL certificate generated at:"
    echo "  Key:  ${KEY_FILE}"
    echo "  Cert: ${CERT_FILE}"
else
    echo "SSL certificates already exist, skipping generation."
fi

# 如果 .env 不存在，复制示例文件并修改证书路径
if [ ! -f /etc/zuti/.env ]; then
    if [ -f /etc/zuti/.env.example ]; then
        cp /etc/zuti/.env.example /etc/zuti/.env
        # 确保证书路径指向正确位置
        sed -i 's|^CERT_PATH=.*|CERT_PATH=/etc/zuti/certs/server.crt|' /etc/zuti/.env
        sed -i 's|^KEY_PATH=.*|KEY_PATH=/etc/zuti/certs/server.key|' /etc/zuti/.env
    else
        # 创建默认 .env 文件
        cat > /etc/zuti/.env << 'ENVFILE'
DATABASE_URL=sqlite:///var/lib/zuti/zuti.db
CERT_PATH=/etc/zuti/certs/server.crt
KEY_PATH=/etc/zuti/certs/server.key
HOST=0.0.0.0
PORT=8443
ENVFILE
    fi
fi

# 重新加载 systemd 并启动服务
if command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload
    systemctl enable zuti.service
    systemctl start zuti.service || true
fi

echo ""
echo "=========================================="
echo "zuti has been installed successfully!"
echo "=========================================="
echo "Configuration file: /etc/zuti/.env"
echo "Certificates: ${CERT_DIR}"
echo "Data directory: /var/lib/zuti"
echo ""
echo "Service commands:"
echo "  sudo systemctl start zuti"
echo "  sudo systemctl stop zuti"
echo "  sudo systemctl status zuti"
echo ""
echo "To view logs:"
echo "  sudo journalctl -u zuti -f"
echo "=========================================="

exit 0
EOF

# 创建维护脚本 - prerm
cat > "${BUILD_DIR}/DEBIAN/prerm" << 'EOF'
#!/bin/bash
set -e
if command -v systemctl >/dev/null 2>&1; then
    systemctl stop zuti.service || true
    systemctl disable zuti.service || true
fi
exit 0
EOF

# 创建维护脚本 - postrm
cat > "${BUILD_DIR}/DEBIAN/postrm" << 'EOF'
#!/bin/bash
set -e
if [ "$1" = "purge" ]; then
    echo "Removing zuti data..."
    if id -u zuti >/dev/null 2>&1; then
        userdel zuti 2>/dev/null || true
    fi
    rm -rf /var/lib/zuti /var/log/zuti
    # 可选：删除证书（注释掉以保留证书）
    # rm -rf /etc/zuti/certs
    echo "zuti data has been removed."
    echo "Note: Configuration files in /etc/zuti/ are preserved."
    echo "      Run 'sudo rm -rf /etc/zuti' to remove them."
fi
exit 0
EOF

# 创建维护脚本 - config（用于 debconf 交互配置，可选）
cat > "${BUILD_DIR}/DEBIAN/config" << 'EOF'
#!/bin/bash
set -e

# 如果需要添加交互式配置，可以在这里使用 debconf
# 目前仅作为占位符

exit 0
EOF

# 设置脚本权限
chmod 755 "${BUILD_DIR}/DEBIAN/preinst"
chmod 755 "${BUILD_DIR}/DEBIAN/postinst"
chmod 755 "${BUILD_DIR}/DEBIAN/prerm"
chmod 755 "${BUILD_DIR}/DEBIAN/postrm"
chmod 755 "${BUILD_DIR}/DEBIAN/config"

# 构建 deb 包
echo "Building .deb package..."
dpkg-deb --build "${BUILD_DIR}"

# 移动最终包
mkdir -p target/debian
mv "target/deb-build/${PKG_NAME}.deb" "target/debian/"

echo ""
echo "=== Build complete ==="
echo "Package: target/debian/${PKG_NAME}.deb"
ls -lh "target/debian/${PKG_NAME}.deb"

echo ""
echo "=== Package info ==="
dpkg -I "target/debian/${PKG_NAME}.deb"
