/*!
 * Backend power-off operations implemented behind a socket
 */

use std::convert::Infallible;
use crate::init::console::prelude::*;
use nix::sys::reboot::RebootMode;

oncelock! {
  /// Modify service supervisor behavior to allow service process to die
  pub static POWER_OFF_READY: OnceLock<bool>;
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
pub fn force_poweroff(mode: Mode) -> Result<Infallible>
{
  use nix::sys::reboot::reboot;

  // Skip any other important poweroff stuff (like killing services)
  reboot(mode.into()).into_trace(Error::PowerCritical)
}

/**
  * # Errors
  *
  * * Failed to get the `init::SERVICE_WATCHERS` oncelock since it isn't set
  *
  * * Failed to read from /run/kickit/service
  *
  * * Failed to parse the PID file as it isn't numeric
  *
  * * Failed to kill a service
  *
  * * Failed to reboot with `nix::sys::reboot::reboot`
  */
pub fn poweroff(mode: Mode) -> Result<Infallible>
{
  use crate::{init::{init_guard, mount::{mounttab, unmount_flags}, service::STANDARD_SERVICES}};
  use nix::sys::reboot::reboot;
  use std::{fs, path::PathBuf, process};

  macro_rules! any_path
  {
    ($lhs: expr, $($rhs: literal)|*) =>
    {
      $($lhs.as_path() == <std::path::Path>::new($rhs))||*
    };
  }

  // This makes sure that when we kill the services, the service manager doesn't throw an error
  oncelock! { POWER_OFF_READY = true }?;

  // If an error has occurred during service startup, we need to still be able to accept poweroff calls without error
  if let Some(services) = STANDARD_SERVICES.get()
  {
    for service in services
    {
      status!("Killing service: {}", service.cfg.name);
      debug_cc!(&service);

      // This is the only instance where the down method should ever be called
      service.down()?;
    }
  }
  else {
    warn!("Standard services have not been initialized yet");
  }

  for entry in (mounttab()?)
  {
    // Do not touch any other mountpoints outside of kickit's scope if we are not the init process
    continueif! (init_guard() && !entry.dest().starts_with("/run/kickit/"));

    // These are special system filesystems, unmounting them would be bad
    if (!any_path!(entry.dest(), "/dev" | "/dev/pts" | "/proc" | "/sys" | "/sys/fs/cgroup" | "/run"))
    {
      status!("Unmounting filesystem: {}", entry.dest().display());

      if let Err(error) = entry.unmount(None)
      {
        error.warn();
        warn!("Failed to unmount filesystem, it will be lazily unmounted");
        // We don't want to do this but this is a last resort
        entry.unmount(Some(unmount_flags![Lazy])).or_warn();
      }
    }
  }

  if (init_guard())
  {
    // Just kill kickit & nothing else, since we are not the init system
    status!("Stopping kickit now");
    // Remove any leftovers from this init session
    fs::remove_dir_all(PathBuf::from("/run/kickit")).into_trace(Error::Shutdown)?;
    process::exit(0);
  }

  // System call to poweroff/reboot
  reboot(mode.into()).into_trace(Error::PowerCritical)
}
