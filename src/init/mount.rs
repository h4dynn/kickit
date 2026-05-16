//! Mount implementation layered over the `nix` crate

use crate::display_enum;
use super::console::Result;
use std::{fmt, fmt::Display, path::Path};

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

pub enum UnmountFlag
{
  // Unmount even if busy, pretty damn dangerous (nfs only)
  Force = 1,
  // Ignore checks & unmount anyway
  Lazy = 2,
  // Mark the mountpoint as expired
  Expire = 4,
  // Don't follow symlinks to their source
  NoFollow = 8
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

#[derive(PartialEq, Eq, Copy, Clone, Debug, Default)]
pub struct Flags(pub u64);

#[derive(PartialEq, Eq, Clone, Debug, Default)]
pub struct Opts(pub Vec<Opt>);

#[derive(PartialEq, Eq, Copy, Clone, Debug, Default)]
pub struct UnmountFlags(pub i32);

impl From<Flags> for nix::mount::MsFlags
{
  fn from(flags: Flags) -> Self
  {
    Self::from_bits_retain(flags.0)
  }
}

impl From<UnmountFlags> for nix::mount::MntFlags
{
  fn from(flags: UnmountFlags) -> Self
  {
    Self::from_bits_retain(flags.0)
  }
}

impl Display for Opts
{
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result
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

#[macro_export]
macro_rules! mountflags
{
  [$($flag: tt),*] =>
  {
    {
      use $crate::init::mount::Flags;
      let mut flags = Flags::default();

      $(
        flags.0 += $flag as u64;
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

#[macro_export]
macro_rules! unmountflags
{
  [$($flag: tt),*] =>
  {
    {
      use $crate::init::mount::UnmountFlags;
      let mut flags = UnmountFlags::default();

      $(
        flags.0 += $flag as i32;
      )*

      flags
    }
  };

  [] => { UnmountFlags::default() };
}
pub use crate::unmountflags as unmountflags;

/**
  * # Errors
  *
  * - Couldn't mount the device (`nix::mount::mount()` gives more info)
 **/
// Frontend for nix library's mount function
pub fn mount(from: Option<&str>, to: &str, fsType: Option<&str>, flags: Flags, opts: Option<&Opts>)
  -> Result<()>
{
  use crate::init::console::{Error, ErrorResult};

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
pub fn unmount(dest: &str, maybeFlags: Option<UnmountFlags>) -> Result<()>
{
  use crate::init::console::{Error, ErrorResult};

  if let Some(flags) = maybeFlags
  {
    nix::mount::umount2(dest, flags.into())
  }
  else {
    nix::mount::umount(dest)
  }
    .into_trace(Error::SysFsUnmount)?;

  Ok(())
}

/**
  * # Errors
  *
  * - Couldn't read from /proc/mounts
 **/
// Check if a path is a mountpoint
pub fn mounted(mountpoint: impl AsRef<Path> + Display) -> Result<bool>
{
  use std::fs;
  use crate::init::console::{Error, ErrorResult};

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
