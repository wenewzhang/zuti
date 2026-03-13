#!/bin/bash
all_disks=$(lsblk -d -n -o NAME | grep -E "^(sd|nvme|hd|vd)")

zfs_disks=$(zpool status -v | grep -E "^\s+(sd|hd|vd)" | awk '{print $1}' | sed 's/[0-9]\+$//')
echo $all_disks
echo "----"
echo $zfs_disks
echo "----"
echo "$all_disks" | while read disk; do
      if ! echo "$zfs_disks" | grep -qw "$disk"; then
          echo "/dev/$disk"
      fi
done

