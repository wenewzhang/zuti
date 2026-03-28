## Build zuti
```
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

curl --proto '=https' --tlsv1.2 -LsSf https://github.com/diesel-rs/diesel/releases/latest/download/diesel_cli-installer.sh | sh

apt-get install -y libclang-dev llvm-dev clang libpam0g-dev libsqlite3-dev

cargo run --bin zuti
```