## 1-7 June 2026

* Overhaul ktctl service information gathering with new
  cached configuration

* Add namespace-based sandboxing support for services
  with new executable `warden`

* Improve socket error handling

* Add proper support for services with the `Forking`
  pattern

* Remove useless `log_level` option in service config

## 20-21 May 2026

* Add timeout for services with `RunOnce` pattern

* Improve the service watcher's implementation

* Fix optional services throwing fatal errors

## 14-19 May 2026

* Add support for /etc/fstab entry mounting

* Add XBPS package for Void Linux

* Use lazy unmount as a fallback when powering off

* Unmount filesystems on power-off

* Add safe, socket-based power-off/reboot with `ktctl`

* Respect the `quiet` kernel command-line argument

## 12-13 May 2026

* Move zstd compression impl from `zstd-rs` to `ruzstd`

* Refactor sockets (remove messy service socket, use tokio)

* General code format cleanup

## 30 Nov - 6 Dec 2025

* Add implementation for accessing master log from `ktctl`

* Improve & clenaup socket implementation

* Cleanup some `ktctl` code

## 23-29 Nov 2025

[4923e2d](https://github.com/h4dynn/kickit/commit/4923e2d683e3f850568416a55ce55d349c7c5003)

* Re-export macros to fix their respective paths

* Add shutdown/reboot support via power socket

* Cleanup `ktctl` socket i/o
