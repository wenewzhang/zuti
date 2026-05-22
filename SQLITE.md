sqlite schema
```
diesel migration generate add_memo_to_users
Creating migrations/2026-05-22-120751-0000_add_memo_to_users/up.sql
Creating migrations/2026-05-22-120751-0000_add_memo_to_users/down.sql
[jimmy@jimmy-macmini71 zuti]$ cat migrations/2026-05-22-120751-0000_add_memo_to_users/up.sql 
[jimmy@jimmy-macmini71 zuti]$   diesel migration run --database-url db.sqlite
Running migration 2026-05-22-120751-0000_add_memo_to_users

```
