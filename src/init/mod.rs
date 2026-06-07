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

// Some distros such as Alpine will use more lightweight shells, such as ash
#[cfg(feature = "posix_sh")]
pub(crate) const SHELL: &str = "/bin/sh";

// The default shell will be bash unless the "posix_sh" feature is set
#[cfg(not(feature = "posix_sh"))]
pub(crate) const SHELL: &str = "/bin/bash";

/**
  * # Errors
  *
  * - /proc/cmdline couldn't be accessed for whatever reason,
  * - Specified command-line param wasn't found
  */
// Get a command-line parameter using the /proc/cmdline file
#[inline]
pub fn cmdlineParam(param: &str) -> Result<Option<String>>
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
    if let Some((key, value)) = cmdlineParam.trim_end_matches('\n').split_once('=') && (key == param)
    {
      return Ok(Some(value.to_owned()))
    }
    // No value, just bool parameter
    else if (cmdlineParam == param)
    {
      return Ok(None)
    }
  }

  // Command-line parameter was not found at all
  Err(Error::Cmdline.trace(param))
}
