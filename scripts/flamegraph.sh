#!/bin/bash -euf

if ! cargo install --list | grep -Ew "^flamegraph v[0-9]+.[0-9]+.[0-9]+:$" &> /dev/null
then {
  echo "Flamegraph is not installed. Installing it now via cargo..."
  cargo install flamegraph
}
fi

if ! command -v perf &> /dev/null
then {
  echo -e "\e[1;31m(error):\e[0m perf is not installed, install it via your package manager"
  exit 1
}
fi

sudo env HOME="$HOME" cargo flamegraph --dev -- --no-init
