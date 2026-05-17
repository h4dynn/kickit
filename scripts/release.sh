#!/bin/bash -eufx
# Build release version of kickit
RUSTFLAGS='-C prefer-dynamic' cargo build --release
