export token
```
export TOKEN=eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJteWFkbWluIiwiaWF0IjoxNzczNzk5OTQ0LCJleHAiOjE3NzYzOTE5NDQsImp0aSI6IjhkNjNhNGMxLTdhNWYtNDY5OS1iMWYzLWFlOWRiYWZjNzczMiJ9.9_8T9z3CmT9noSz9kHGf1f0EOvAt90bVCaU2Tj7CzJg
```

创建用户
```
curl -k -X POST https://192.168.3.100:8443/admin_user     -H "Content-Type: application/json"     -d '{"name": "myadmin", "type_": "admin", "password":"123321"}'
```

登陆
```
  curl -k -X POST https://192.168.3.100:8443/login \
      -H "Content-Type: application/json" \
      -d '{"username": "myadmin", "password": "123321"}'
```

get_disks
```
  curl -k https://192.168.3.100:8443/get_disks \
    -H "Authorization: Bearer $TOKEN"

```

get free disks
```
  curl -k https://192.168.3.100:8443/get_free_disks \
    -H "Authorization: Bearer $TOKEN"

```

Delete disk
```
  curl -k -X POST \
      -H "Authorization: Bearer $TOKEN" \
      -H "Content-Type: application/json" \
      -d '{"disk_name": "nvme0n1"}' \
      https://192.168.3.100:8443/delete_disk

```

Part disk
```
  curl -k -X POST \
      -H "Authorization: Bearer $TOKEN" \
      -H "Content-Type: application/json" \
      -d '{"disk_name": "nvme0n1", "size":"80%"}' \
      https://192.168.3.100:8443/part_disk
```

find free disk partition
```
curl -k -H "Authorization: Bearer $TOKEN"  \
      https://192.168.3.100:8443/get_free_parts

```

create pool

```
  curl -k -X POST https://192.168.3.100:8443/create_pool \
   -H "Authorization: Bearer $TOKEN"  \
    -H "Content-Type: application/json" \
    -d '{"pool_name":"datapool","pool_type":"raid1","devices":["sda3","sdb3","nvme0n1p1"]}'

```

desttory pool
```
  curl -k -X POST https://192.168.3.100:8443/destroy_pool \
   -H "Authorization: Bearer $TOKEN"  \
  -H "Content-Type: application/json" \
  -d '{"pool_name": "mypool"}'

```

add user
```
  curl -k -X POST https://192.168.3.100:8443/add_user \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $TOKEN" \
    -d '{
      "username": "share_user",
      "password": "123321",
      "user_type": "share"
    }'

```

logout
```
  curl -k -X POST https://192.168.3.100:8443/logout \
      -H "Authorization: Bearer $TOKEN"

```

smb public share
```


  curl -k -X POST https://192.168.3.248:8443/smb/public_share \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{
      "directory": "/etcc",
      "browseable": "yes",
      "read_only": "no",
      "guest_ok": "yes"
    }'

```

smb auth share
```
  curl  -k -X POST https://192.168.3.248:8443/smb/auth_share \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{
      "directory": "/datapool/xl",
      "browseable": "yes",
      "read_only": "yes",
      "valid_users": ["reader1", "reader2", "writer1", "writer2"],
      "write_list": ["writer1", "writer2"]
    }'

```

samba user
```
  curl  -k -X POST https://192.168.3.248:8443/smb/add_user \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{
      "username": "reader1",
      "password": "123321"
    }'
```

delete samba user
```
 curl  -k -X POST https://192.168.3.248:8443/smb/delete_user \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{
      "username": "shareuser1"
    }'

```

list user
```
  curl  -k -X POST https://192.168.3.248:8443/smb/list_users \
    -H "Authorization: Bearer $TOKEN"
```

list zfs pool
```
  curl  -k -X POST https://192.168.3.248:8443/smb/list_zfs_pools \
    -H "Authorization: Bearer $TOKEN"
```

create zfs pool
```
  curl -k -X POST https://192.168.3.248:8443/smb/create_zfs_share \
      -H "Content-Type: application/json" \
      -H "Authorization: Bearer $TOKEN" \
      -d '{
          "share_name": "goodshare",
          "pool_name": "datapool",
          "quota": "1G",
          "samba_user": "reader1"
      }'

```

list zfs shares
```
  curl -k -X POST https://192.168.3.248:8443/smb/list_zfs_shares \
      -H "Authorization: Bearer $TOKEN"

```

list directory shares
```
  curl -k -X POST https://192.168.3.248:8443/smb/list_dir_shares \
      -H "Authorization: Bearer $TOKEN"

```

remove directory share
```
  curl -k -X POST https://192.168.3.248:8443/smb/remove_dir_share \
      -H "Content-Type: application/json" \
      -H "Authorization: Bearer $TOKEN" \
      -d '{
          "share_name": "myshare"
      }'
```

remove directory share
```
  curl -k -X POST https://192.168.3.248:8443/smb/remove_zfs_share \
      -H "Content-Type: application/json" \
      -H "Authorization: Bearer $TOKEN" \
      -d '{
          "dataset": "datapool/goodshare"
      }'
```

Docker list images
```
  curl -k -X GET https://192.168.3.248:8443/docker/get_images \
      -H "Authorization: Bearer $TOKEN"
```

Docker pull image
```
  curl -k -X POST https://192.168.3.248:8443/docker/pull_image/start \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"image_name": "postgres:16.13-trixie"}'
```

Docker pull image progress
```
  curl -k -X GET https://192.168.3.248:8443/docker/pull_image/task/0b319bfc-128d-408c-97b2-2b965e20d7f9 \
    -H "Authorization: Bearer $TOKEN" 
```

delete image by id
```
  curl -k -X DELETE https://192.168.3.248:8443/docker/delete_image/1a1e63136420 \
    -H "Authorization: Bearer $TOKEN" 
```


create container
```
  curl -k -X POST https://192.168.3.248:8443/docker/create_container \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{
      "image": "docker.io/library/nginx:trixie-perl",
      "name": "my-nginx",
      "env": {"NGINX_HOST": "example.com"},
      "ports": [
        {"host_port": "8080", "container_port": "80/tcp"}
      ],
      "volumes": [
        {"host_path": "/data/nginx", "container_path": "/usr/share/nginx/html", "read_only": false}
      ],
      "restart_policy": "always",
      "auto_start": true
    }'

```

## Registry setting
  1. POST /docker/setting/registry - 添加/更新镜像源

  请求：
```
  curl -k -X POST https://192.168.3.248:8443/docker/setting/registry \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{
      "prefix": "docker.io",
      "location": "docker.io",
      "insecure": false
    }'
```
  2. GET /docker/setting/registry - 获取所有镜像源
```
  curl -k https://192.168.3.248:8443/docker/setting/registry \
    -H "Authorization: Bearer $TOKEN"
```
  3. DELETE /docker/setting/registry - 删除镜像源
```
  curl -k -X DELETE https://192.168.3.248:8443/docker/setting/registry \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{
      "prefix": "docker.io",
      "location": "docker.nju.edu.cn"
    }'

```

Mirror
```
  curl -k -X POST "${HOST}/docker/setting/mirror" \
    -H "Authorization: Bearer ${TOKEN}" \
    -H "Content-Type: application/json" \
    -d '{"location":"mirror.baidubce.com"}'

  curl -k -X POST "${HOST}/docker/setting/mirror" \
    -H "Authorization: Bearer ${TOKEN}" \
    -H "Content-Type: application/json" \
    -d '{"location":"docker.mirrors.sjtug.sjtu.edu.cn"}'

  # 3. 查看配置
  curl -k "${HOST}/docker/setting/mirror" \
    -H "Authorization: Bearer ${TOKEN}"

  # 4. 删除有问题的 mirror（只传 location）
  curl -k -X DELETE "${HOST}/docker/setting/mirror" \
    -H "Authorization: Bearer ${TOKEN}" \
    -H "Content-Type: application/json" \
    -d '{"location":"mirror.baidubce.com"}'

```


Volume setting
```
export TOKEN=eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJteWFkbWluIiwiaWF0IjoxNzczNzk5OTQ0LCJleHAiOjE3NzYzOTE5NDQsImp0aSI6IjhkNjNhNGMxLTdhNWYtNDY5OS1iMWYzLWFlOWRiYWZjNzczMiJ9.9_8T9z3CmT9noSz9kHGf1f0EOvAt90bVCaU2Tj7CzJg
  curl -k https://192.168.3.248:8443/docker/containers \
    -H "Authorization: Bearer $TOKEN"
export  HOST="https://192.168.3.248:8443"

  # 添加数据卷
  curl -k -X POST "${HOST}/docker/setting/volume" \
    -H "Authorization: Bearer ${TOKEN}" \
    -H "Content-Type: application/json" \
    -d '{"name": "data", "host_path": "/data", "description": "Data storage"}'

  # 添加配置卷
  curl -k -X POST "${HOST}/docker/setting/volume" \
    -H "Authorization: Bearer ${TOKEN}" \
    -H "Content-Type: application/json" \
    -d '{"name": "config", "host_path": "/etc/myapp", "description": "Config files"}'

  # 添加日志卷
  curl -k -X POST "${HOST}/docker/setting/volume" \
    -H "Authorization: Bearer ${TOKEN}" \
    -H "Content-Type: application/json" \
    -d '{"name": "logs", "host_path": "/var/log/myapp", "description": "Log files"}'

  echo "All volumes created!"

```

Podman compose
```
  curl -k -X POST https://192.168.3.248:8443/docker/compose_up \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"content":"services:\n  db:\n    image: postgres:15\n    environment:\n      POSTGRES_PASSWORD: secret\n    volumes:\n      - db_data:/var/lib/postgresql/data\n  web:\n    image: my
  app:latest\n    ports:\n      - \"3000:3000\"\n    depends_on:\n      - db\nvolumes:\n  db_data:","project_name":"myapp","detached":true,"build":false}'




  curl -k https://192.168.3.248:8443/docker/compose_list \
    -H "Authorization: Bearer $TOKEN"


  curl -k -X DELETE https://192.168.3.248:8443/docker/compose_down \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{
      "project_name": "myapp"
    }'

  删除项目及卷

  curl -k -X DELETE https://192.168.3.248:8443/docker/compose_down \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{
      "project_name": "myapp",
      "volumes": true
    }'

  删除项目、卷和镜像

  curl -k -X DELETE https://192.168.3.248:8443/docker/compose_down \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{
      "project_name": "myapp",
      "volumes": true,
      "remove_images": true
    }'

  curl -k -X DELETE "https://192.168.3.248:8443/docker/compose/delete/myapp" \
    -H "Authorization: Bearer ${TOKEN}"


```

pod
```
  # 列出 pods
  curl -k "https://192.168.3.248:8443/podman/pod/list" \
    -H "Authorization: Bearer ${TOKEN}"

  # 启动 pod
  curl -k -X POST "https://192.168.3.248:8443/podman/pod/start/my-pod" \
    -H "Authorization: Bearer ${TOKEN}"

  # 停止 pod
  curl -k -X POST "https://192.168.3.248:8443/podman/pod/stop/my-pod" \
    -H "Authorization: Bearer ${TOKEN}"

  # 重启 pod
  curl -k -X POST "https://192.168.3.248:8443/podman/pod/restart/my-pod" \
    -H "Authorization: Bearer ${TOKEN}"

  # 删除 pod (强制)
  curl -k -X DELETE "https://192.168.3.248:8443/podman/pod/remove/my-pod" \
    -H "Authorization: Bearer ${TOKEN}"

```

ZFS set bootfs & bootfs
```
zpool set bootfs=one-pool/ROOT/zuti-260225_NEW one-pool
root@onenas:~# findmnt -n -o SOURCE /

  curl -k "https://192.168.3.248:8443/zfs/bootfs" \
    -H "Authorization: Bearer ${TOKEN}"

  curl -k -X POST "https://192.168.3.248:8443/zfs/set_bootfs" \
      -H "Authorization: Bearer ${TOKEN}" \
      -H "Content-Type: application/json" \
      -d '{
      "dataset": "one-pool/ROOT/zuti260326",
      "pool": "one-pool"
      }'    

```

  1. 重启系统 API

  • 端点: POST /zfs/reboot
  • 权限: 需要 JWT 认证 + 管理员权限
  • 命令: systemctl reboot
```
  curl -k -X POST "https://192.168.3.248:8443/zfs/reboot" -H "Authorization: Bearer ${TOKEN}"
```
  2. 关闭系统 API

  • 端点: POST /zfs/shutdown
  • 权限: 需要 JWT 认证 + 管理员权限
  • 命令: systemctl poweroff
```
  curl -k -X POST "https://192.168.3.248:8443/zfs/shutdown" \
      -H "Authorization: Bearer ${TOKEN}"
```

Clone

```
  curl -k -X POST "https://192.168.3.248:8443/zfs/clone" \
      -H "Authorization: Bearer ${TOKEN}" \
      -H "Content-Type: application/json" \
      -d '{
          "new_name": "zuti-260225_NEW",
          "dataset": "one-pool/ROOT/zuti-260225"
      }'
```