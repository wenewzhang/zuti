创建用户
```
curl -k -X POST https://192.168.3.248:8443/users     -H "Content-Type: application/json"     -d '{"name": "myadmin", "type_": "admin", "password":"123321"}'
```