#!/bin/bash

cargo build --release
sudo systemctl stop zuti
sudo cp -a target/release/zuti /usr/bin/.
sudo cp -a debian/zuti.service /usr/lib/systemd/system/zuti.service
sudo systemctl daemon-reload
sudo systemctl start zuti
sudo journalctl -xeu zuti
