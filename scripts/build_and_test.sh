#!/bin/bash -eufx
# Create a debug build & run it

cargo build -F bypass_init_check

# Delete our init directory from previous runs
if [[ -e /run/kickit ]]
then {
  sudo rm -rfv /run/kickit
}
fi

# Run it!
sudo ./target/debug/kickit --no-init
