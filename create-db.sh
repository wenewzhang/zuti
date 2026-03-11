#!/bin/bash

# 检查系统中是否有 diesel 命令
if ! command -v diesel &> /dev/null; then
    echo "diesel 命令未找到，正在安装 diesel_cli..."
    curl --proto '=https' --tlsv1.2 -LsSf https://github.com/diesel-rs/diesel/releases/download/v2.3.5/diesel_cli-installer.sh | sh
else
    echo "diesel 已安装，正在执行 database setup..."
    diesel database setup
fi
