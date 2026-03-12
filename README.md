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
  curl -k https://127.0.0.1:8443/get_disks \
    -H "Authorization: Bearer eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJteWFkbWluIiwiaWF0IjoxNzczMjc4NzQ5LCJleHAiOjE3NzU4NzA3NDksImp0aSI6ImE2YzE2Y2VmLTU5ZWQtNDY5ZS1iYWNhLTQxOGJkZGY0YmIwYSJ9.-YiTJQ0HnsPhoB_A7aQaZIpK484ZWi2nRw1uFOmJimM"

```