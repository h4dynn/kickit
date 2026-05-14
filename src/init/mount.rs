//! Mount implementation layered over the `nix` crate

use crate::display_enum;
use std::fmt;

// Mount flags & their corresponding bit value (MsFlags)
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum Flag
{
  ReadOnly = 1,
  NoSuid = 2,
  NoDev = 4,
  NoExec = 8,
  Remount = 32,
  Bind = 4096,
  Private = 1 << 18
}

display_enum!
{
  Flag
  {
    ReadOnly => "ro", NoSuid => "nosuid", NoDev => "nodev",
    NoExec => "noexec", Remount => "remount", Bind => "bind",
    Private => "private"
  }
}

// Basic type alias
pub type Opt = String;

#[derive(PartialEq, Eq, Clone, Debug, Default)]
pub struct Flags(pub Vec<Flag>);

#[derive(PartialEq, Eq, Clone, Debug, Default)]
pub struct Opts(pub Vec<Opt>);

impl From<Flags> for nix::mount::MsFlags
{
  fn from(flags: Flags) -> Self
  {
    // The default flags are just 0 (none)
    let mut out_flags: u64 = 0;

    for flag in (flags.0)
    {
      // Combine the flags
      out_flags += flag as u64;
    }

    Self::from_bits_retain(out_flags)
  }
}

impl fmt::Display for Opts
{
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error>
  {
    let mut str_opts = String::new();

    for option in (&self.0)
    {
      str_opts.push_str(option);
      str_opts.push(',');
    }

    if (!str_opts.is_empty())
    {
      write!(f, "{}", str_opts.strip_suffix(',').expect("Options formatting error"))?;
    }
    Ok(())
  }
}

impl Flags
{
  pub fn push(&mut self, flag: Flag)
  {
    self.0.push(flag);
  }
}

#[macro_export]
macro_rules! mountflags
{
  [$($flag: tt),*] =>
  {
    {
      use $crate::init::mount::Flags;
      let mut flags = Flags::default();

      $(
        flags.0.push($flag.into());
      )*

      flags
    }
  };

  [] => { Flags::default() };
}
pub use crate::mountflags as mountflags;

#[macro_export]
macro_rules! mountopts
{
  [$($opt: tt),*] =>
  {
    {
      use $crate::init::mount::Opts;
      let mut opts = Opts::default();

      $(
        opts.0.push($opt.into());
      )*

      opts
    }
  };

  [] => { Options::default() };
}
pub use crate::mountopts as mountopts;

/**
  * # Errors
  *
  * - Couldn't mount the device (`nix::mount::mount()` gives more info)
 **/
// Frontend for nix library's mount function
pub fn mount(from: Option<&str>, to: &str, fsType: Option<&str>, flags: Flags, opts: Option<&Opts>)
  -> Result<(), crate::init::init_console::ErrorTrace>
{
  use crate::init::init_console::{Error, ErrorResult};

  /*
   * We have to do a manual map here because if we map like:
   * `.map(|x| &x as &str)`
   * ..then the compiler throws this error:
   * "returns a value referencing data owned by the current function"
   */
  #[allow(clippy::manual_map)]
  let nixOpts =
  {
    if let Some(realOpts) = opts
    {
      Some(&realOpts.to_string() as &str)
    }
    else {
      None
    }
  };

  nix::mount::mount(from, to, fsType, flags.into(), nixOpts).into_trace(Error::SysFsMount)?;

  Ok(())
}

/**
  * # Errors
  *
  * - Couldn't unmount the device
 **/
pub fn unmount(dest: &str) -> Result<(), crate::init::init_console::ErrorTrace>
{
  use crate::init::init_console::{Error, ErrorResult};

  nix::mount::umount(dest).into_trace(Error::SysFsUnmount)?;
  Ok(())
}

/**
  * # Errors
  *
  * - Couldn't read from /proc/mounts
 **/
// Check if a path is a mountpoint
pub fn mounted<P: AsRef<std::path::Path> + std::fmt::Display>(mountpoint: P)
  -> Result<bool, crate::init::init_console::ErrorTrace>
{
  use std::fs;
  use crate::init::init_console::{Error, ErrorResult};

  // Cycle through potential matches from /proc/mounts
  for candMount in (fs::read_to_string("/proc/mounts").into_trace(Error::FileNotFound)?.split('\n'))
  {
    if (candMount.split(' ').nth(1) == Some(&mountpoint.to_string()))
    {
      return Ok(true)
    }
  }

  // Didn't find mount
  Ok(false)
}
