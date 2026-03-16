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
cp "certs/server.crt" "${BUILD_DIR}/etc/zuti/certs/" 2>/dev/null || echo "Warning: server.crt not found"
cp "certs/server.key" "${BUILD_DIR}/etc/zuti/certs/" 2>/dev/null || echo "Warning: server.key not found"
cp "migrations/"* "${BUILD_DIR}/usr/share/zuti/migrations/" 2>/dev/null || true
cp ".env.example" "${BUILD_DIR}/etc/zuti/" 2>/dev/null || true
cp "debian/zuti.service" "${BUILD_DIR}/lib/systemd/system/"

# 创建 control 文件
cat > "${BUILD_DIR}/DEBIAN/control" << EOF
Package: zuti
Version: ${VERSION}
Section: admin
Priority: optional
Architecture: ${ARCH}
Maintainer: Your Name <wenewboy@gmail.com>
Description: Zuti - OneNas Storage management web service
 A web service for disk and storage pool management.
 Built with Rust and Actix-web.
Depends: libc6, libssl3 | libssl1.1, libpam0g
EOF

# 创建维护脚本
cat > "${BUILD_DIR}/DEBIAN/preinst" << 'EOF'
#!/bin/bash
set -e
if ! id -u zuti >/dev/null 2>&1; then
    useradd -r -s /bin/false -M -d /var/lib/zuti zuti 2>/dev/null || true
fi
mkdir -p /etc/zuti/certs /var/lib/zuti /var/log/zuti
exit 0
EOF

cat > "${BUILD_DIR}/DEBIAN/postinst" << 'EOF'
#!/bin/bash
set -e
chown -R zuti:zuti /etc/zuti 2>/dev/null || chown -R root:root /etc/zuti
chown -R zuti:zuti /var/lib/zuti 2>/dev/null || chown -R root:root /var/lib/zuti
chown -R zuti:zuti /var/log/zuti 2>/dev/null || chown -R root:root /var/log/zuti

if [ ! -f /etc/zuti/.env ]; then
    cp /etc/zuti/.env.example /etc/zuti/.env 2>/dev/null || true
fi

if command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload
    systemctl enable zuti.service
    systemctl start zuti.service || true
fi
exit 0
EOF

cat > "${BUILD_DIR}/DEBIAN/prerm" << 'EOF'
#!/bin/bash
set -e
if command -v systemctl >/dev/null 2>&1; then
    systemctl stop zuti.service || true
    systemctl disable zuti.service || true
fi
exit 0
EOF

cat > "${BUILD_DIR}/DEBIAN/postrm" << 'EOF'
#!/bin/bash
set -e
if [ "$1" = "purge" ]; then
    if id -u zuti >/dev/null 2>&1; then
        userdel zuti 2>/dev/null || true
    fi
    rm -rf /var/lib/zuti /var/log/zuti
fi
exit 0
EOF

# 设置脚本权限
chmod 755 "${BUILD_DIR}/DEBIAN/preinst"
chmod 755 "${BUILD_DIR}/DEBIAN/postinst"
chmod 755 "${BUILD_DIR}/DEBIAN/prerm"
chmod 755 "${BUILD_DIR}/DEBIAN/postrm"

# 构建 deb 包
echo "Building .deb package..."
dpkg-deb --build "${BUILD_DIR}"

# 移动最终包
mkdir -p target/debian
mv "target/deb-build/${PKG_NAME}.deb" "target/debian/"

echo "=== Build complete ==="
echo "Package: target/debian/${PKG_NAME}.deb"
ls -lh "target/debian/${PKG_NAME}.deb"

# 显示包信息
dpkg -I "target/debian/${PKG_NAME}.deb"
