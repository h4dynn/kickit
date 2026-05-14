#!/bin/sh -euf

# ############################################### #
# This script was adapted from Void Linux's runit #
# agetty-ttyX script                              #
# ############################################### #
# Check out the repository here:                  #
# <https://github.com/void-linux/void-runit>      #
# ############################################### #

# busybox
if command -v getty > /dev/null
then {
  readonly GETTY=getty
}
# util-linux
elif command -v agetty > /dev/null
then {
  readonly GETTY=agetty
}
else {
  echo "Failed to find a suitable agetty client!" >&2
  exit 1
}
fi

if [ -r /etc/agetty.conf ]
then {
  # shellcheck source=/dev/null
  . /etc/agetty.conf
}
else {
  # The default configuration options
  readonly TTYS=7
  readonly BAUD_RATE=38400
  readonly TERM_NAME=linux
}
fi

tty=0
while [ $tty -le "$TTYS" ]
do {
  tty=$((tty+1))
  echo "Opening tty ${tty}/${TTYS}" >&2
  # Open agetty in the background
  exec "${GETTY}" "${GETTY_ARGS}" "${tty}" "${BAUD_RATE}" "${TERM_NAME}" &
}
done

# Loop forever
while sleep 100000
do {
  :
}
done
