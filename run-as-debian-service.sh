#!/bin/bash

cargo build --release
systemctl stop zuti
cp -a target/release/zuti /usr/bin/.
cp -a debian/zuti.service /usr/lib/systemd/system/zuti.service
systemctl daemon-reload
systemctl start zuti
journalctl -xeu zuti