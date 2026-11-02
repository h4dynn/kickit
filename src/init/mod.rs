/*!
 * Components of the backend init system
 */

extern crate tokio;
extern crate serde;
extern crate toml;
extern crate nix;

pub mod console;
pub mod mount;
pub mod service;
pub mod target;
pub mod socket;
pub mod power;

use crate::AnyStr;

// The ID of the "kickit-shell" user account, which an emergency shell is opened on
pub const EMERGENCY_SHELL_UID: u32 = 490;
// Path of the shell to execute in emergency, some distros may use lightweight shells like ash instead
pub const EMERGENCY_SHELL: &str = "/bin/bash";

/// Guard to prevent kickit from performing init tasks
/**
  * # Tasks
  *
  * Examples include mounting pseudo filesystems, setting the hostname, probing udev devices, mounting
  * fstab entries), see the `kickit` binary source for all the operations prevented by the guard.
  *
  * # Condition
  *
  * This uses [`std::process::id`] to determine the PID, the init system's PID is always `1` as it is
  * the 1st process executed by the kernel.
  */
#[must_use]
pub fn init_guard() -> bool
{
  std::process::id() != 1
}

/// Resolve kernel command-line flag- provided as single-word (e.g. `quiet`) or suffixed with `=true`
pub fn cmdline_flag(id: impl AnyStr) -> bool
{
  if let Ok(cmdline) = std::fs::read_to_string("/proc/cmdline")
  {
    // Remove newline at the end (screws with evaluation) & read parameters delimited by whitespace
    cmdline.trim_end_matches('\n').split(' ').any(|c| c == id.as_ref() || c == [id.as_ref(), "=true"].concat())
  }
  else {
    false
  }
}

/// Attempt to source kernel command-line option in string form
pub fn cmdline_value(id: impl AnyStr) -> Option<String>
{
  if let Ok(cmdline) = std::fs::read_to_string("/proc/cmdline")
  {
    for param in (cmdline.trim_end_matches('\n').split(' '))
    {
      // Split string into two values from the first '=' if provided
      if let Some((param_id, val)) = param.split_once('=') && param_id == id.as_ref()
      {
        // ID matches & value is available
        return Some(String::from(val))
      }
    }
  }

  // Option not found in kernel cmdline
  None
}
