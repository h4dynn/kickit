//! init process

extern crate tokio;
extern crate serde;
extern crate toml;
extern crate nix;

pub mod init_console;
pub mod mount;
pub mod service;
pub mod target;
pub mod socket;
pub mod power;

use std::sync::OnceLock;
use self::init_console::{Error, Result, ExtendWithContext};

// Once the target is sourced its configuration will be stored here
pub static TARGET: OnceLock<self::target::Target> = OnceLock::new();
pub static TARGET_NAME: OnceLock<String> = OnceLock::new();
// If set to true, master log entries won't be shown on console
pub static QUIET: OnceLock<bool> = OnceLock::new();
// Tells the service watcher to not care if a service is killed, set by `poweroff(_, _)`
pub static POWER_OFF: OnceLock<bool> = OnceLock::new();

// The default shell will be bash unless the "posix_sh" feature is set
pub(crate) const SHELL: &str =
{
  // Some distros such as Alpine will use more lightweight shells, such as ash
  if (cfg!(feature = "posix_sh"))
  {
    "/bin/sh"
  }
  else {
    "/bin/bash"
  }
};

/**
  * # Errors
  *
  * - /proc/cmdline couldn't be accessed for whatever reason,
  * - Specified command-line param wasn't found
 **/
// Get a command-line parameter using the /proc/cmdline file
#[inline]
pub fn cmdlineParam(param: &str) -> Result<Option<String>>
{
  use std::{fs, path::PathBuf};
  use crate::init::init_console::ErrorResult;

  // Read the cmdline from procfs
  let cmdline = fs::read_to_string(PathBuf::from("/proc/cmdline"))
                    .into_trace(Error::FileNotFound).context("/proc/cmdline")?;

  // Split the cmdline's parameters by spaces
  for rawCmdlineParam in (cmdline.split(' '))
  {
    // Split by equals so that if the param has a value we can return it
    let mut cmdlineParam: Vec<&str> = rawCmdlineParam.trim_end_matches('\n').split('=').collect();

    if (cmdlineParam[0] == param)
    {
      // If there is no value for this param so return none
      return if (cmdlineParam.len() == 1)
      {
        Ok(None)
      }
      else {
        // Remove the param's name, just get its value
        cmdlineParam.remove(0);
        // Collect the values back into a String
        Ok(Some(cmdlineParam.into_iter().collect()))
      }
    }
  }

  // Command-line parameter was not found at all
  Err(Error::Cmdline.trace(param))
}
