#!/usr/bin/env bash
# Cross-build the portal for the router (aarch64 musl) and drop it where
# install.sh will pick it up. Pure Rust, so rust-lld links it with no C
# toolchain and no container.
set -euo pipefail
cd "$(dirname "$0")"

# Match your router: `ssh <router> uname -m`.
#   aarch64  -> aarch64-unknown-linux-musl   (GL-MT6000, most modern ARM64)
#   mips     -> mips-unknown-linux-musl      (older Atheros/MediaTek)
#   x86_64   -> x86_64-unknown-linux-musl    (x86 appliances, VMs)
TARGET="${TARGET:-aarch64-unknown-linux-musl}"

rustup target add "$TARGET" >/dev/null 2>&1 || true
# Pure Rust with no C dependencies, so rust-lld links it: no cross toolchain,
# no container. TLS is delegated to uclient-fetch on the router.
cargo build --release --target "$TARGET"
mkdir -p ../usr/sbin
cp "target/$TARGET/release/classnet-portal" ../usr/sbin/classnet-portal
echo "built $(ls -lh ../usr/sbin/classnet-portal | awk '{print $5}') -> usr/sbin/classnet-portal"
