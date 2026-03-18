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


  curl -k -X POST https://192.168.3.248:8443/smb_public_share \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{
      "directory": "/datapool/myshare",
      "browseable": "yes",
      "read_only": "no",
      "guest_ok": "yes"
    }'

```