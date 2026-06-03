//! Mount implementation layered over the `nix`

use crate::{display_enum, wrap, OptionEmptyVec};
use super::console::{Result, Error, ErrorTrace, ErrorResult};

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
  Flag {
    ReadOnly => "ro", NoSuid => "nosuid", NoDev => "nodev", NoExec => "noexec",
    Remount => "remount", Bind => "bind", Private => "private"
  }
}

pub type Opt = String;

// Mount flags (casted as u64), added together
#[derive(PartialEq, Eq, Copy, Clone, Debug, Default)]
pub struct Flags(u64);

// Custom filesystem-specific options
#[derive(PartialEq, Eq, Clone, Debug, Default)]
pub struct Opts(Vec<Opt>);

// For optional use with `unmount`, which in turn calls `nix::mount::umount2`
#[derive(PartialEq, Eq, Copy, Clone, Debug, Default)]
pub struct UnmountFlags(i32);

wrap! {
  // Dereference to the first & only item in the tuple
  impl Deref<Target = u64> for Flags;
  impl Deref<Target = Vec<Opt>> for Opts;
  impl Deref<Target = i32> for UnmountFlags;
}

impl From<Flags> for nix::mount::MsFlags
{
  fn from(flags: Flags) -> Self
  {
    Self::from_bits_retain(*flags)
  }
}

impl From<UnmountFlags> for nix::mount::MntFlags
{
  fn from(flags: UnmountFlags) -> Self
  {
    Self::from_bits_retain(*flags)
  }
}

impl TryFrom<&str> for Flag
{
  type Error = ErrorTrace;

  fn try_from(flag: &str) -> Result<Flag>
  {
    use Flag::{ReadOnly, NoSuid, NoDev, NoExec, Remount, Bind, Private};

    Ok(
      match (flag)
      {
        "ro" => ReadOnly, "nosuid" => NoSuid, "nodev" => NoDev, "noexec" => NoExec,
        "remount" => Remount, "bind" => Bind, "private" => Private,
        _ => Err(Error::Unknown.trace(format!("Unrecognised mount flag: {flag}")))?
      }
    )
  }
}

impl Display for Opts
{
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result
  {
    let mut stringOpts = String::new();

    for option in (&self.0)
    {
      stringOpts.push_str(option);
      // Options are seperated by commas
      stringOpts.push(',');
    }

    if (!stringOpts.is_empty())
    {
      // Remove the end trailing comma if it's there
      write!(f, "{}", stringOpts.strip_suffix(',').expect("Mount options formatting error"))?;
    }
    Ok(())
  }
}

#[macro_export]
macro_rules! mountFlags
{
  [$($flag: expr),*] =>
  {
    {
      use $crate::init::mount::Flags;
      let mut flags = Flags::default();

      $(
        *flags |= $flag as u64;
      )*

      flags
    }
  };

  [] => { Flags::default() };
}
pub use mountFlags as flags;

#[macro_export]
macro_rules! unmountFlags
{
  [$($flag: tt),*] =>
  {
    {
      use $crate::init::mount::UnmountFlags;
      let mut flags = UnmountFlags::default();

      $(
        *flags |= $flag as i32;
      )*

      flags
    }
  };

  [] => { UnmountFlags::default() };
}
pub use unmountFlags as unmountFlags;

#[macro_export]
macro_rules! mountOpts
{
  [$($opt: tt),*] =>
  {
    {
      use $crate::init::mount::Opts;
      let mut opts = Opts::default();

      $(
        (*opts).push($opt.into());
      )*

      opts
    }
  };

  [] => { Options::default() };
}
pub use mountOpts as opts;

/**
  * # Errors
  *
  * - Couldn't mount the device (`nix::mount::mount()` gives more info)
 **/
// Frontend for nix library's mount function
pub fn mount(from: Option<&str>, to: &str, fsType: Option<&str>, flags: Flags, opts: Option<&Opts>)
  -> Result<()>
{
  use nix::mount::mount;

  #[allow(clippy::manual_map)]
  let nixOpts =
  {
    if let Some(realOpts) = opts
    {
      Some(&realOpts.join(",") as &str)
    }
    else {
      None
    }
  };

  mount(from, to, fsType, flags.into(), nixOpts).into_trace(Error::SysFsMount)
}

/**
  * # Errors
  *
  * - Couldn't unmount the device
 **/
pub fn unmount(dest: &str, maybeFlags: Option<UnmountFlags>) -> Result<()>
{
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

/**
  * # Errors
  *
  * * Failed to open the /etc/fstab file
  */
pub fn mountFstabEntries() -> Result<()>
{
  use crate::{path, continueif, init::console::status};
  use std::{fs::File, path::PathBuf, io::{BufReader, BufRead}};

  /*
   * When searching by UUID, partition UUID, ID or label, there is multiple paths where
   * the kernel may symlink to, so we cover all bases with this macro
   */
  macro_rules! lookup
  {
    ($ty: ident = $id: ident) =>
    {
      {
        // Most of the time it will be here
        if (path!("/dev", "disk", format!("by-{}", stringify!($ty)), $id).is_symlink())
        {
          Ok(format!("/dev/disk/by-{}/{}", stringify!($ty), $id))
        }
        // On some devices (like arm tablets) it will be here instead
        else if (path!("/dev", "block", format!("by-{}", stringify!($ty)), $id).is_symlink())
        {
          Ok(format!("/dev/block/by-{}/{}", stringify!($ty), $id))
        }
        else {
          Err(Error::Unknown.trace(&format!("Lookup {}={} failed, no target was found!", stringify!($ty), $id)))
        }
      }
    };
  }

  // Open the fstab configuration
  let fstab = File::open(PathBuf::from("/etc/fstab")).into_trace(Error::FileNotFound)?;

  for maybeEntry in (BufReader::new(fstab).lines())
  {
    let stringEntry = maybeEntry.into_trace(Error::Unknown)?;

    // Ignore comments...
    continueif! (stringEntry.starts_with('#'));

    // Split up by spaces, so we can parse each entry's information
    let entry: Vec<&str> = stringEntry.split_ascii_whitespace().collect();

    if (entry.len() != 6)
    {
      return Err(Error::FstabParse.trace(format!("Expected a line with 6 elements, but found {}", entry.len())))
    }

    // The raw source may be a lookup, so we change it in the `source` value
    let (rawSource, dest) = (entry[0], entry[1]);
    let source = {
      // Let the libc mount function infer the source on its own
      if (matches!(rawSource, "auto" | "none"))
      {
        None
      }
      else {
        // Split the source by lookup only once
        Some(if let Some((lookup, id)) = rawSource.split_once('=')
        {
          &match (lookup)
          {
            "PARTUUID" => lookup!(partuuid = id), "UUID" => lookup!(uuid = id),
            "LABEL" => lookup!(label = id), "ID" => lookup!(id = id),
            _ => Err(Error::FstabParse.trace(format!("Unknown lookup type: {lookup}")))
          }?
        }
        else {
          // No changes needed here!
          rawSource
        })
      }
    };
    let fsType = {
      // Only pass the filesystem type if its valid
      if (matches!(entry[2], "auto" | "none"))
      {
        // Let the mount function infer the filesystem's type
        None
      }
      else {
        Some(entry[2])
      }
    };
    let (flags, opts) =
    {
      // Flags - global settings for all filesystems no matter the type
      let mut flags = Flags::default();
      // Options - filesystem-type exclusive options
      let mut opts = Opts::default();

      // Read each flag in its string form (e.g. ReadOnly = "ro"), seperated by commas
      for option in (entry[3].split(','))
      {
        if let Ok(flag) = Flag::try_from(option)
        {
          // Add the flag as its u64
          *flags += flag as u64;
        }
        // Ignore the "defaults" flag - it isn't recognised by the libc mount call
        else if (option != "defaults")
        {
          (*opts).push(option.to_owned());
        }
      }

      if (source == Some("/"))
      {
        // The rootfs will already be mounted so we need the remount flag
        *flags += Flag::Remount as u64;
      }

      // `.empty_none()` turns an vector into a Option, where if its empty None is produced
      (flags, opts.empty_none())
    };

    if let Some(src) = source
    {
      status!("Mounting: {src} -> {dest}");
    }

    mount(source, dest, fsType, flags, opts.as_ref())?;
  }
  Ok(())
}
