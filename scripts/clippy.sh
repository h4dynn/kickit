#!/bin/bash -eufx
# Make clippy as annoying as is possible & deny warnings
cargo clippy -- -W clippy::all -W clippy::pedantic -D warnings
