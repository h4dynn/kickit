#!/bin/bash -eufx
# Build release version of kickit

# Set RUSTFLAGS if not already set
: "${RUSTFLAGS:=}"

# This is so Rust links our binaries to libkickit.so
RUSTFLAGS="${RUSTFLAGS} -C prefer-dynamic" cargo build --release
