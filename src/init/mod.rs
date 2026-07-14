//! init process

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

use crate::{oncelock, console::ExtendWithContext};
use self::console::{Error, Result};

oncelock! {
  // Some when `--no-init` arg is provided to kickit, safeguard to limit kickit's behaviour
  pub static PID: Option<u32>;
}

// The ID of the "kickit-shell" user account, which an emergency shell is opened on
pub const EMERGENCY_SHELL_UID: u32 = 490;
// Path of the emergency shell to execute, some distros may not have bash installed by default
pub const EMERGENCY_SHELL: &str = "/bin/bash";

/**
  * # Errors
  *
  * - /proc/cmdline couldn't be accessed for whatever reason,
  * - Specified command-line param wasn't found
  */
// Get a command-line parameter using the /proc/cmdline file
#[inline]
pub fn cmdline(param: impl AsRef<str>) -> Result<Option<String>>
{
  use std::{fs, path::PathBuf};
  use crate::console::ErrorResult;

  // Read the cmdline from procfs
  let cmdline = fs::read_to_string(PathBuf::from("/proc/cmdline"))
                    .into_trace(Error::FileNotFound).context("/proc/cmdline")?;

  // Split the cmdline's parameters by spaces
  for cmdlineParam in (cmdline.split(' '))
  {
    // Parameter has a value to give
    if let Some((key, value)) = cmdlineParam.trim_end_matches('\n').split_once('=') && (key == param.as_ref())
    {
      return Ok(Some(value.to_owned()))
    }
    // No value, just bool parameter
    else if (cmdlineParam == param.as_ref())
    {
      return Ok(None)
    }
  }

  // Command-line parameter was not found at all
  Err(Error::Cmdline.trace(param.as_ref()))
}
