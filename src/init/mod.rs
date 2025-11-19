//! init process

extern crate tokio;
extern crate serde;
extern crate toml;
extern crate nix;

pub mod init_console;
pub mod mount;
pub mod service;
pub mod target;

use std::sync::Mutex;
use crate::init::init_console::{KTError, KTErrorTrace};

pub enum PowerLevel { On = 0, Off = 1, Reboot = 2 }

pub static POWER_LEVEL: Mutex<u8> = Mutex::new(0);
// The default shell will be bash unless the "posix_sh" feature is set
pub(crate) const SHELL: &str = if (cfg!(feature = "posix_sh")) { "/bin/sh" } else { "/bin/bash" };

///
/// # Errors
///
/// * /proc/cmdline couldn't be accessed for whatever reason,
/// * Specified command-line param wasn't found
///
// Get a command-line parameter using the /proc/cmdline file
#[inline] pub fn cmdlineParam(param: &str) -> Result<Option<String>, KTErrorTrace>
{
  use std::{fs, path::PathBuf};
  use crate::{init::init_console::ConvKTError, New};

  // Read the cmdline from procfs
  let kcmdline = fs::read_to_string(PathBuf::from("/proc/cmdline"))
                    .context_trace("/proc/cmdline", KTError::FileNotFound)?;

  // Split the cmdline's parameters by spaces
  for rawCmdlineParam in (kcmdline.split(' '))
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
  Err(KTErrorTrace::with_context(KTError::CmdlineFail, param, ""))
}
