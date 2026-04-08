四种用户
1.admin
2.view 
3.share
4.samba  不可在web ui登陆,仅用于samba共享

```
场景	推荐配置
Windows SMB 共享	acltype=nfsv4, aclinherit=passthrough, aclmode=passthrough
纯 Linux 开发环境	acltype=posix, aclinherit=restricted, aclmode=discard

```

primarycache quota mountpoint recordsize atime relatime readonly aclmode aclinherit acltype canmount logbias sync compression checksum