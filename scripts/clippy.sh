#!/bin/bash -eufx
cargo clippy -- -W clippy::all -W clippy::pedantic -D warnings
