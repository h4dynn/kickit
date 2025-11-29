# Building & Testing kickit

## Building

**Basics**

1. If you are building for testing purposes, you will
   want the `debug` / `developer` profile. This is the
   default, you don't need to add any extra arguments
   to `cargo build` for this.

2. If not, you want the `release` profile, so add the
   `--release` argument to `cargo build`.

3. If you are testing `kickit` on a machine that is
   currently running an existing init & you want
   to run `kickit` alongside it, you can start
   the init with the `--no-init` argument.

### (0) Preparation

* Make sure you have `cargo` installed and it is the
  latest available version. See the
  [Rust website](https://rust-lang.org/tools/install/)
  for how to install for your platform.

### (1) Building

1. Clone the repository (no submodules are needed):

```bash
git clone https://github.com/h4dynn/kickit.git
cd kickit
```

2. Build it!:

> **release** profile

```bash
cargo build --release
```

> **debug** profile

```bash
cargo build
```

## Running / Testing

**Environment**

1. Create the `/usr/lib/kickit` directory, and then
   create the `target` and `service` subdirectories.

2. Create a target in `/usr/lib/kickit/target/`,
   for example `system.toml`. The filename must
   always with the `toml` extension. See the
   [example target](https://github.com/h4dynn/kickit/blob/main/docs/target_example.toml)
   for how to format your target & the
   relevant options. (note: If you are using
   the `debug` build, the target must be
   called `test`)

3. (optional) Create all the service files you
   need in `/usr/lib/kickit/service/`. See the
   [example config](https://github.com/h4dynn/kickit/blob/main/docs/service_example.toml)
   for more info.

### Testing

Run this command in the directory of the `kickit`
repository:

```bash
sudo ./target/debug/kickit --no-init
```

The `--no-init` flag makes sure we don't override
the current init.

### Running

1. If your target isn't called `system`, make sure
   to edit your kernel cmdline to add the
   `init.target=XXX` parameter. This tells `kickit`
   which target we want to use.

2. Either push the `kickit` executable (in
   `target/release/kickit`) to `/usr/sbin/init`
   (**be careful to not overwrite any existing
   init program!**) OR push it to
   `/usr/lib/kickit/kickit` and add the kernel
   command-line parameter `init=/usr/lib/kickit/kickit`.

3. Reboot & kickit should (hopefully) be running fine!
