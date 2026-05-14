//! power.rs - Shutdown & reboot safely

use std::ffi::OsString;
use super::init_console::{Error, ErrorResult, Result};
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

// Using `Result<!>` is experimental sadly
/**
  * # Errors
  *
  * * Failed to get the `init::SERVICE_WATCHERS` oncelock since it isn't set,
  * * Failed to read from /run/kickit/service,
  * * Failed to parse the PID file as it isn't numeric,
  * * Failed to kill a service
  * * Failed to reboot with `nix::reboot::reboot`
  */
pub fn poweroff(mode: Mode) -> Result<()> //Result<!>
{
  use crate::{path, console::ReturnError};
  use super::{SERVICE_WATCHERS, init_console::status};
  use nix::{unistd::Pid, sys::{reboot::reboot, signal::{kill, Signal}}};
  use std::{fs, path::PathBuf};

  let watchers = SERVICE_WATCHERS.get().ok_or(Error::Unknown.trace("Failed to get service watchers!"))?;
  let pidsAndNames =
  {
    let mut inner = Vec::<(u32, OsString)>::new();

    // List all the services in the runfs directory
    for maybeService in (fs::read_dir(PathBuf::from("/run/kickit/service")).into_trace(Error::RunFsFail)?)
    {
      let name = maybeService.into_trace(Error::Unknown)?.file_name();
      let pid = path!("/run/kickit/service", &name, "pid");

      // Test if this is a RunOnce service & if so we don't need to do anything here
      if (path!("/run/kickit/service", &name, "exited").is_file())
      {
        continue
      }

      // Read the string PID, and then try to read the u32 from it
      let pid: u32 = fs::read_to_string(pid).into_trace(Error::RunFsFail)?
                          .parse().into_trace(Error::Unknown)?;

      inner.push((pid, name));
    }

    inner
  };

  status!("Killing service watchers");
  for watcher in (watchers)
  {
    // Kill the watcher
    watcher.abort();
  }

  for (pid, name) in (pidsAndNames)
  {
    status!("Killing service: {}", name.display());

    // First try killing with SIGQUIT
    if let Err(err) = kill(Pid::from_raw(pid.cast_signed()), Some(Signal::SIGQUIT)).into_trace(Error::Unknown)
    {
      err.warn();
      // If that doesn't work we use SIGKILL
      kill(Pid::from_raw(pid.cast_signed()), Some(Signal::SIGKILL)).into_trace(Error::Unknown)?;
    }
  }

  // Poweroff/reboot!!!!
  reboot(mode.into()).into_trace(Error::Unknown)?;
  // Should never reach this point
  Ok(())
}
