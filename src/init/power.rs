//! power.rs - Shutdown & reboot safely

use std::{convert::Infallible, ffi::OsString};
use super::{NO_INIT, oncelock, console::{Error, ErrorResult, Result}};
use nix::sys::reboot::RebootMode;

oncelock! {
  // Tells the service watcher to not care if a service is killed, set by `poweroff(_, _)`
  pub static POWER_OFF_READY: bool;
}

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
pub fn forcePoweroff(mode: Mode) -> Result<Infallible>
{
  use nix::sys::reboot::reboot;

  // Skip any other important poweroff stuff (like killing services)
  reboot(mode.into()).into_trace(Error::PowerCritical)
}

/**
  * # Errors
  *
  * * Failed to get the `init::SERVICE_WATCHERS` oncelock since it isn't set,
  * * Failed to read from /run/kickit/service,
  * * Failed to parse the PID file as it isn't numeric,
  * * Failed to kill a service
  * * Failed to reboot with `nix::sys::reboot::reboot`
  */
pub fn poweroff(mode: Mode) -> Result<Infallible>
{
  use crate::{path, continueif, console::{Colour, HandleError, ReturnError}};
  use super::{mount::{unmount, unmountFlags, UnmountFlag::Lazy}, console::{status, warn}};
  use nix::{unistd::Pid, sys::{reboot::reboot, signal::{kill, Signal}}};
  use std::{fs, path::PathBuf, process};

  let noInit = *oncelock!(&NO_INIT)?;
  let pidsAndNames: Vec<(u32, OsString)> =
  {
    let mut inner = Vec::new();

    // List all the services in the runfs directory
    for maybeService in (fs::read_dir(PathBuf::from("/run/kickit/service")).into_trace(Error::RunFsFail)?)
    {
      let name = maybeService.into_trace(Error::Unknown)?.file_name();
      let pidPath = path!("/run/kickit/service", &name, "pid");

      // Test if this is a RunOnce service & if so we don't need to do anything here
      continueif! (path!("/run/kickit/service", &name, "exited").is_file());

      // Read the little-endian ordered PID u32 bytes
      let pid = u32::from_le_bytes(fs::read(pidPath).into_trace(Error::RunFsFail)?
                  .try_into()
                  .map_err(|_| Error::Format.trace("Bad pid contents!").context(name.display()))?);

      inner.push((pid, name));
    }

    inner
  };

  // This makes sure that when we kill the services, the service manager doesn't throw an error
  oncelock! { POWER_OFF_READY = true }?;

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

  for mountInfoString in (fs::read_to_string("/proc/mounts").into_trace(Error::ProcFs)?.lines())
  {
    // Split for each space found & get the 2nd element (1st in this case since indices starts at 0)
    let mountpoint = mountInfoString.split_ascii_whitespace().nth(1)
                        .ok_or(Error::ProcFs.trace("Missing mountpoint specifier in /proc/mounts"))?;

    // Do not touch any other mountpoints outside of kickit's scope if we are not the init process
    continueif! (noInit && !mountpoint.starts_with("/run/kickit/"));

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

  if (noInit)
  {
    // Just kill kickit & nothing else, since we are not the init system
    status!("Stopping kickit now");
    // Remove any leftovers from this init session
    fs::remove_dir_all(PathBuf::from("/run/kickit")).into_trace(Error::Shutdown)?;
    process::exit(0);
  }
  else {
    // Poweroff/reboot!!!!
    reboot(mode.into()).into_trace(Error::PowerCritical)
  }
}
