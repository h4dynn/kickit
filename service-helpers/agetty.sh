#!/bin/sh -euf

# ############################################### #
# This script was adapted from Void Linux's runit #
# agetty-ttyX script                              #
# ############################################### #
# Check out the repository here:                  #
# <https://github.com/void-linux/void-runit>      #
# ############################################### #

# busybox
if command -v getty &> /dev/null
then {
  export GETTY=getty
}
# util-linux
elif command -v agetty &> /dev/null
then {
  export GETTY=agetty
}
else {
  echo "Failed to find a suitable agetty client!" >&2
  exit 1
}
fi

if [ -r /etc/agetty.conf ]
then {
  . /etc/agetty.conf
}
else {
  # The default configuration options
  export TTYS=7
  export BAUD_RATE=38400
  export TERM_NAME=linux
}
fi

for ((tty = 0; $tty < $TTYS; tty++))
do {
  echo "Opening tty ${tty}/${TTYS}" >&2
  exec "${GETTY}" "${GETTY_ARGS[@]}" \
                 "${tty}" "${BAUD_RATE}" "${TERM_NAME}" &
}
done

# Loop forever
while sleep 100000
do {
  :
}
done
