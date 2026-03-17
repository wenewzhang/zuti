  502  zfs create -o sharesmb=on -o compression=lz4 one-pool/windows_share
  503  zfs set quota=1G pool/windows_share
  504  zfs set quota=1G one-pool/windows_share

  513  zfs set mountpoint=/one-pool/windows_share one-pool/windows_share

  515  systemctl status zfs-share

  530  chown -R share_user:share_user /one-pool/windows_share/

  535    sudo chmod -R 755 /one-pool/windows_share

  540  systemctl status  nmbd
  541  systemctl status  nmbd

  544    zfs get sharesmb one-pool/windows_share
  545  smbpasswd -a share_user
