#!/bin/sh -euf
# Get xbps-src from Void Linux's repo

echo "Fetching xbps-src..." >&2
curl -#Lo- https://github.com/void-linux/void-packages/archive/refs/heads/master.tar.gz | gunzip |  tar --strip-components=1 -x void-packages-master/xbps-src void-packages-master/common void-packages-master/etc
chmod +x xbps-src
