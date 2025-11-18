//! Mount implementation layered over the `nix` crate

use crate::display_enum;
use std::fmt;

// Mount flags & their corresponding bit value (MsFlags)
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum MountFlag { ReadOnly = 1, NoSuid = 2, NoDev = 4, NoExec = 8, Remount = 32, Bind = 4096 }

display_enum!
{
  MountFlag
  {
    ReadOnly => "ro", NoSuid => "nosuid", NoDev => "nodev",
    NoExec => "noexec", Remount => "remount", Bind => "bind"
  }
}

// Basic type alias
pub type MountOption = String;

#[derive(PartialEq, Eq, Clone, Debug, Default)]
pub struct MountFlags { pub inner: Vec<MountFlag> }

#[derive(PartialEq, Eq, Clone, Debug, Default)]
pub struct MountOptions { pub inner: Vec<MountOption> }

impl From<MountFlags> for nix::mount::MsFlags
{
  fn from(flags: MountFlags) -> Self
  {
    // The default flags are just 0 (none)
    let mut out_flags: u64 = 0;

    for flag in (flags.inner)
    {
      // Combine the flags with the BitOr operator
      out_flags |= flag as u64;
    }

    Self::from_bits_retain(out_flags)
  }
}

impl fmt::Display for MountOptions
{
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error>
  {
    let mut str_opts = String::new();

    for option in (&self.inner)
    {
      str_opts.push_str(option);
      str_opts.push(',');
    }

    write!(f, "{}", if (str_opts.is_empty()) { "" }
    else { str_opts.strip_suffix(',').expect("MountOptions formatting error") })
  }
}

impl MountFlags { pub fn push(&mut self, what: MountFlag) { self.inner.push(what) } }

#[macro_export]
macro_rules! mountflags
{
  ($($flag: tt),*) =>
  {
    {
      let mut mfs = $crate::init::mount::MountFlags::default();

      $(mfs.inner.push($flag.into());)*

      mfs
    }
  };

  () => { MountFlags::default() };
}

#[macro_export]
macro_rules! mountopts
{
  ($($opt: tt),*) =>
  {
    {
      use $crate::init::mount::MountOptions;
      let mut opts = MountOptions::default();

      $(opts.inner.push($opt.into());)*

      opts
    }
  };

  () => { MountOptions::default() };
}

// Frontend for nix library's mount function
///
/// # Errors
/// * Couldn't mount the device (`nix::mount::mount()` gives more info)
///
pub fn mount(from: &str, to: &str, fsType: &str, flags: MountFlags, opts: &MountOptions)
  -> Result<(), crate::init::init_console::KTErrorTrace>
{
  use crate::init::init_console::{KTError, ConvKTError};

  nix::mount::mount(Some(from), to, Some(fsType), flags.into(), Some(&*opts.to_string()))
    .trace(KTError::SysFsMountFail)?;

  Ok(())
}

///
/// # Errors
/// * Couldn't unmount the device
///
pub fn unmount(dest: &str) -> Result<(), crate::init::init_console::KTErrorTrace>
{
  use crate::init::init_console::{KTError, ConvKTError};

  nix::mount::umount(dest).trace(KTError::SysFsUnmountFail)?;
  Ok(())
}

// Check if a path is a mountpoint
///
/// # Errors
/// * Couldn't read from /proc/mounts
///
pub fn mounted<P: AsRef<std::path::Path> + std::fmt::Display>(mountpoint: P)
  -> Result<bool, crate::init::init_console::KTErrorTrace>
{
  use std::fs;
  use crate::init::init_console::{KTError, ConvKTError};

  // Cycle through potential matches from /proc/mounts
  for candMount in (fs::read_to_string("/proc/mounts").trace(KTError::FileNotFound)?.split('\n'))
  {
    if (candMount.split(' ').nth(1) == Some(&mountpoint.to_string()))
    {
      return Ok(true)
    }
  }

  // Didn't find mount
  Ok(false)
}
