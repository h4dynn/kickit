
> Each goal is ordered from most important to least

# kickit

- [x] Make an io.Power socket, for user power control
      (shutdown/reboot)

- [x] Implement safe power-off/reboot (stop all services,
      unmount filesystems, etc)

- [x] Add option to create directories in /run for
      services

- [ ] Using sandbox on services fills up mount entries in
      /proc/mounts which is ugly, perhaps there's some way
      to stop this (how does bubblewrap do it?)

- [ ] Require root login for emergency shell for
      security

- [ ] ~~Actually enforce `target.log_level`~~

- [x] Cleanup errors for ktctl & init binaries

- [ ] Add partial compatibility with `systemd`
      (programs that depend on systemd)

# ktctl

- [x] Properly implement /run/kickit/io.Core input/output

- [ ] Add option to restart services

- [x] Implement way to access init master log

- [ ] Improve slightly messy/clumsy code

# Documentation

- [ ] Add info on actually installing it

- [ ] Add info for contributors/devs
