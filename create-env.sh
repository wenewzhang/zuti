#!/bin/bash

# 生成 32 字节的随机字符串（base64 编码）
RANDOM_SECRET=$(openssl rand -base64 32)

# 复制 .env.example 到 .env
cp .env.example .env

# 替换 JWT_SECRET 的值
sed -i "s/your_secret_key_change_in_production/${RANDOM_SECRET}/g" .env

echo ".env 文件已生成，JWT_SECRET 已设置为随机值"
