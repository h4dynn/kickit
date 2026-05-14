
> Each goal is ordered from most important to least

# kickit

- [x] Make an io.Power socket, for user power control
      (shutdown/reboot)

- [ ] Implement safe power-off/reboot (stop all services,
      unmount filesystems, etc)

- [ ] Add option to create directories in /run for
      services

- [ ] Require root login for emergency shell for
      security

- [ ] Actually enforce `target.log_level`

- [ ] Cleanup errors for ktctl & init binaries

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
