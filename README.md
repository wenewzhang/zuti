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