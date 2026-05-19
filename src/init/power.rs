//! power.rs - Shutdown & reboot safely

use std::ffi::OsString;
use super::console::{Error, ErrorResult, Result};
use nix::sys::reboot::RebootMode;

pub enum Mode
{
  Shutdown,
  Reboot
}

// The Mode enum does not cover all the modes so this can't be a 2-way conversion
#[allow(clippy::from_over_into)]
impl Into<RebootMode> for Mode
{
  fn into(self) -> RebootMode
  {
    match (self)
    {
      Self::Shutdown => RebootMode::RB_POWER_OFF,
      Self::Reboot => RebootMode::RB_AUTOBOOT
    }
  }
}

// Not recommended- skip poweroff procedures like unmounting & stopping services
/**
  * # Errors
  *
  * * Failed to power-off/reboot with `nix::sys::reboot::reboot`
  */
pub fn forcePoweroff(mode: Mode) -> Result<()>
{
  use nix::sys::reboot::reboot;

  reboot(mode.into()).into_trace(Error::Unknown)?;
  Ok(())
}

// Using `Result<!>` is experimental sadly
/**
  * # Errors
  *
  * * Failed to get the `init::SERVICE_WATCHERS` oncelock since it isn't set,
  * * Failed to read from /run/kickit/service,
  * * Failed to parse the PID file as it isn't numeric,
  * * Failed to kill a service
  * * Failed to reboot with `nix::sys::reboot::reboot`
  */
pub fn poweroff(mode: Mode) -> Result<()> //Result<!>
{
  use crate::{path, console::{Colour, HandleError, ReturnError}, oncelock};
  use super::{POWER_OFF, mount::{unmount, unmountFlags, UnmountFlag::Lazy}, console::{status, warn}};
  use nix::{unistd::Pid, sys::{reboot::reboot, signal::{kill, Signal}}};
  use std::{fs, path::PathBuf};

  let pidsAndNames: Vec<(u32, OsString)> =
  {
    let mut inner = Vec::new();

    // List all the services in the runfs directory
    for maybeService in (fs::read_dir(PathBuf::from("/run/kickit/service")).into_trace(Error::RunFsFail)?)
    {
      let name = maybeService.into_trace(Error::Unknown)?.file_name();
      let pidPath = path!("/run/kickit/service", &name, "pid");

      // Test if this is a RunOnce service & if so we don't need to do anything here
      if (path!("/run/kickit/service", &name, "exited").is_file())
      {
        continue
      }

      // Read the little-endian ordered PID u32 bytes
      let pid = u32::from_le_bytes(fs::read(pidPath).into_trace(Error::RunFsFail)?
                  .try_into()
                  .map_err(|_| Error::Format.trace("Bad pid contents!").context(name.display()))?);

      inner.push((pid, name));
    }

    inner
  };
  let mounts = fs::read_to_string("/proc/mounts").into_trace(Error::Unknown)?;

  // This makes sure that when we kill the services, the service manager doesn't throw an error
  oncelock! { let POWER_OFF = true }?;

  for (pid, name) in (pidsAndNames)
  {
    status!("Killing service: {}", name.display());

    // First try killing with SIGQUIT
    if let Err(err) = kill(Pid::from_raw(pid.cast_signed()), Some(Signal::SIGQUIT)).into_trace(Error::Unknown)
    {
      err.warn();
      // If that doesn't work we use SIGKILL
      kill(Pid::from_raw(pid.cast_signed()), Some(Signal::SIGKILL)).into_trace(Error::Unknown).or_warn();
    }
  }

  for mountInfoString in (mounts.lines())
  {
    // Split for each space found
    let info: Vec<&str> = mountInfoString.split_ascii_whitespace().collect();
    let mountpoint = info[1];

    match (mountpoint)
    {
      // These are special system filesystems, unmounting them would be bad
      "/dev" | "/dev/pts" | "/proc" | "/sys" | "/sys/fs/cgroup" | "/run" => (),
      // Hopefully safe to unmount
      _ =>
      {
        status!("Unmounting filesystem: {mountpoint}");

        if let Err(error) = (unmount(mountpoint, None))
        {
          error.warn();
          warn!("Failed to unmount filesystem, it will be lazily unmounted: {mountpoint}");
          // We don't want to do this but this is a last resort
          unmount(mountpoint, Some(unmountFlags!(Lazy))).or_warn();
        }
      }
    }
  }

  // Poweroff/reboot!!!!
  reboot(mode.into()).into_trace(Error::Unknown)?;
  // Should never reach this point
  Ok(())
}
