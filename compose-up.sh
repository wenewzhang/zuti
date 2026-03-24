#!/bin/bash

TOKEN="eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJteWFkbWluIiwiaWF0IjoxNzczNzk5OTQ0LCJleHAiOjE3NzYzOTE5NDQsImp0aSI6IjhkNjNhNGMxLTdhNWYtNDY5OS1iMWYzLWFlOWRiYWZjNzczMiJ9.9_8T9z3CmT9noSz9kHGf1f0EOvAt90bVCaU2Tj7CzJg"
HOST="https://192.168.3.248:8443"

  # 读取 compose 文件内容
  COMPOSE_CONTENT=$(cat <<'EOF'
  services:
    db:
      image: postgres:15
      environment:
        POSTGRES_PASSWORD: secret
      volumes:
        - db_data:/var/lib/postgresql/data
    web:
      image: myapp:latest
      ports:
        - "3000:3000"
      depends_on:
        - db
  volumes:
    db_data:
EOF
)

 jq -n \
    --arg content "$COMPOSE_CONTENT" \
    --arg project "myapp" \
    '{
      content: $content,
      project_name: $project,
      detached: true,
      build: false
    }' | curl -k -X POST "${HOST}/docker/compose_up" \
      -H "Authorization: Bearer ${TOKEN}" \
      -H "Content-Type: application/json" \
      -d @-
