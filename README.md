<p align="center"><img src="assets/logo.svg" alt="kickit" width="500"/></p>

**A simple & robust init system, written in Rust**

> [!WARNING]
> `kickit` is a work-in-progress. Many features are unimplemented or unstable.[^1]

![Lint](https://github.com/h4dynn/kickit/actions/workflows/clippy.yml/badge.svg?event=push)

# Design

Everything in `kickit` is defined in your target.
These are stored in the `/usr/lib/kickit/target/`
directory.

When `kickit` starts up, it will load all the
services & options specified in the target.
Once a service is loaded it cannot be unloaded,
and new services that are not in the target
cannot be loaded. (you can, however, restart
a service)

# Compatibility

Only Linux is supported. Support for other
platforms is not planned.

# Building

See [Developers.md](https://github.com/h4dynn/kickit/blobs/main/Developers.md)
for more information on how to build kickit

[^1]: See [TO-DO.md](https://github.com/h4dynn/kickit/blob/main/TO-DO.md)
for the current state of the project
