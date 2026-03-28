## Build zuti
```
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

apt-get install -y libclang-dev llvm-dev clang

cargo run --bin zuti
```