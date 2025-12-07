//! init process

extern crate tokio;
extern crate serde;
extern crate toml;
extern crate nix;

pub mod init_console;
pub mod mount;
pub mod service;
pub mod target;

use std::sync::OnceLock;
use crate::init::init_console::{KTError, KTErrorTrace};

#[derive(Eq, PartialEq, Copy, Clone, Debug)]
pub enum PowerLevel { Off, Reboot }

pub static POWER_LEVEL: OnceLock<PowerLevel> = OnceLock::new();
pub static UP_SERVICES: OnceLock<Vec<String>> = OnceLock::new();

// The default shell will be bash unless the "posix_sh" feature is set
#[cfg(feature = "posix_sh")]
pub(crate) const SHELL: &str = "/bin/sh";

#[cfg(not(feature = "posix_sh"))]
pub(crate) const SHELL: &str = "/bin/bash";

/**
  * # Errors
  *
  * - /proc/cmdline couldn't be accessed for whatever reason,
  * - Specified command-line param wasn't found
 **/
// Get a command-line parameter using the /proc/cmdline file
#[inline]
pub fn cmdlineParam(param: &str) -> Result<Option<String>, KTErrorTrace>
{
  use std::{fs, path::PathBuf};
  use crate::init::init_console::KTErrorResult;

  // Read the cmdline from procfs
  let cmdline = fs::read_to_string(PathBuf::from("/proc/cmdline"))
                    .context_trace("/proc/cmdline", KTError::FileNotFound)?;

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
  Err(KTErrorTrace::with_context(KTError::Cmdline, param, ""))
}

impl TryFrom<u8> for PowerLevel
{
  type Error = ();

  fn try_from(byte: u8) -> Result<Self, ()>
  {
    use crate::socket::Power;

    match (byte)
    {
      Power::SHUTDOWN => Ok(Self::Off),
      Power::REBOOT => Ok(Self::Reboot),
      _ => Err(())
    }
  }
}
