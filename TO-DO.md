
> Each goal is ordered from most important to least

# kickit

- [x] Make an io.Power socket, for user power control
      (shutdown/reboot)

- [x] Implement safe power-off/reboot (stop all services,
      unmount filesystems, etc)

- [x] Add option to create directories in /run for
      services

- [ ] Sandbox's `NewUser` flag needs uid/gid map
      implementation to work properly

- [ ] Using sandbox on services fills up mount entries in
      /proc/mounts which is ugly, perhaps there's some way
      to stop this (how does bubblewrap do it?)

- [x] ~~Require root login for emergency shell for
      security~~ Drop users to an unprivileged shell by
      default on emergencies

- [x] Cleanup errors for ktctl & init binaries

- [ ] ~~Actually enforce `target.log_level`~~

# ktctl

- [x] Properly implement /run/kickit/io.Core input/output

- [x] Implement way to access init master log

- [x] Improve slightly messy/clumsy code

- [ ] ~~Add option to restart services~~

# Documentation

- [x] Add info on actually installing it

- [ ] Add info for contributors/devs
