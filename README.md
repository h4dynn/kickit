<img src="assets/logo.svg" alt="kickit" style="display:block;max-width:100%;margin:auto">

**A simple & robust init system, written in Rust**

> [!WARNING]
> `kickit` is a work-in-progress. Many features are unimplemented or unstable.

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
