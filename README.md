<p align="center"><img src="assets/logo.svg" alt="kickit" width="500"/></p>

**A robust & simple init system, written in Rust**

> [!WARNING]
> `kickit` is a work-in-progress. Many features are unimplemented or unstable.[^1]

![Lint](https://github.com/h4dynn/kickit/actions/workflows/clippy.yml/badge.svg?event=push)

# Design

Everything in `kickit` is defined in your target. These are
stored in the `/usr/lib/kickit/target/` directory.

When `kickit` starts up, it will load all the services & options
specified in the target. Once a service is loaded it cannot be
unloaded, and new services cannot be loaded (you can, however,
restart a service)

This is how the boot process would look like on a computer with
`kickit` installed:

```
    [The kernel loads kickit from the root filesystem.]
                         |
                         |
                         v
  [kickit loads the target from `/usr/lib/kickit/target`.]
                         |
                         |
                         v
   [kickit sources the services provided in the target,
             checking for errors in each.]
                         |
                         |
                         v
         [kickit mounts the system filesystems.]
                         |
                         |
                         v
  [kickit loads its sockets, allowing for user input/output.]
                         |
                         |
                         v
              [kickit loads the services.]
                         |
                         |
                         v
  [The display manager service starts, allowing the
                   user to logon.]
```

`kickit` can also be used alongside another init, such as
`systemd`. Use the service files found in the
[compat folder](https://github.com/h4dynn/kickit/tree/main/compat)
for installation.

# Why?

`systemd` is great at what at it does, but it also does **too**
much i.e., has too many features, alot of them remaining unused
and stagnant.

This project aims to create an init system that is somewhat
similar to `systemd`, whilst minimalising bloat (like
bloatless inits such as `runit` or `openrc`).

In theory, minimising bloat should also hopefully speed things
up, but this cannot be proven yet (like through a chart) so
this is just a hypothesis, not a point (but I also want to try
to benchmark this later on- when the project is more stable).

# Progress

`kickit` has 3 channels:

* **Stable** - Every feature is tested (only in a compatible
  environment) and there are no known bugs at the time of
  release,

* **Testing** - Not tested, but code has been to checked to
  minimise any possible bugs - be prepared to encounter &
  report any bugs on these builds,

* **Unstable** - Untested, unchecked code

Currently, only unstable releases are available as `kickit`
hasn't reached its final goal of:

* (1) Bootability - currently, `kickit` is very unpredictable
      when it comes to different boot environments, this needs
      to be heavily improved on,

* (2) Usability - `kickit` feature-set isn't yet fully complete.

Once these goals are met, testing releases will be published &
ready for dev or hobbyist testing.

# Compatibility

Only Linux is supported. Support for other Unix platforms is not
planned, but would be welcomed via a contribution.

# Building

See [Building.md](https://github.com/h4dynn/kickit/blob/main/docs/Building.md)
for more information on how to build kickit

[^1]: See [TO-DO.md](https://github.com/h4dynn/kickit/blob/main/TO-DO.md)
for the current state of the project

# Thanks to...

* [Rust](https://rust-lang.org/) - for creating such an amazing
  language

* [The nix crate](https://crates.io/crates/nix) - for providing
  Linux
  bindings in Rust

* [Tokio](https://tokio.rs/) - for making async in Rust easy

* [The toml crate](https://crates.io/crates/toml) - for allowing
  easy use of TOML configuration

* [The zstd crate](https://crates.io/crates/zstd) - for providing
  Zstandard bindings in Rust

* [The chrono crate](https://crates.io/crates/chrono) - for
  providing timezone-compatible time unit conversion

* [The thiserror crate](https://crates.io/crates/thiserror) - for
  providing a derive macro for error types

* [The Rust forum](https://users.rust-lang.org/) - for helping
  with very-hard-to-write code
