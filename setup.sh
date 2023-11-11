rustup update
rustup toolchain install nightly --component rust-src
rustup component add llvm-tools-preview
rustup target add x86_64-unknown-none