创建用户
```
curl -k -X POST https://192.168.3.248:8443/users     -H "Content-Type: application/json"     -d '{"name": "myadmin", "type_": "admin", "password":"123321"}'
```

登陆
```
  curl -k -X POST https://192.168.3.248:8443/login \
      -H "Content-Type: application/json" \
      -d '{"username": "myadmin", "password": "123321"}'
```

get_disks
```
  curl -k https://192.168.3.248:8443/get_disks \
    -H "Authorization: Bearer eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJteWFkbWluIiwiaWF0IjoxNzczMzYzNjk5LCJleHAiOjE3NzU5NTU2OTksImp0aSI6IjA0OGM2OWFjLWRkOGYtNGFmZC04YmFmLWNmNTU2MzliZjI0YyJ9.FYk5E-a2MbHQlT-2yUKeqwexmOq8t6J4U0GK2JS2UJY"

```

get free disks
```
  curl -k https://192.168.3.248:8443/get_free_disks \
    -H "Authorization: Bearer eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJteWFkbWluIiwiaWF0IjoxNzczMzYzNjk5LCJleHAiOjE3NzU5NTU2OTksImp0aSI6IjA0OGM2OWFjLWRkOGYtNGFmZC04YmFmLWNmNTU2MzliZjI0YyJ9.FYk5E-a2MbHQlT-2yUKeqwexmOq8t6J4U0GK2JS2UJY"

```

Delete disk
```
  curl -k -X POST \
      -H "Authorization: Bearer eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJteWFkbWluIiwiaWF0IjoxNzczMzYzNjk5LCJleHAiOjE3NzU5NTU2OTksImp0aSI6IjA0OGM2OWFjLWRkOGYtNGFmZC04YmFmLWNmNTU2MzliZjI0YyJ9.FYk5E-a2MbHQlT-2yUKeqwexmOq8t6J4U0GK2JS2UJY" \
      -H "Content-Type: application/json" \
      -d '{"disk_name": "sdb"}' \
      https://192.168.3.248:8443/delete_disk

```
find free disk partition
```
lsblk -fp |awk 'NR>1 && $2=="" {print $0}'
```