//! Target configuration file sourcing

use crate::{oncelock, init::console::Result, path, file_path};

oncelock! {
  // Once the target is sourced its configuration will be stored here
  pub static TARGET: Target;
  // Used to tell ktctl which target is active (in Core socket)
  pub static TARGET_NAME: String;
}

// KTTargetSource is used for toml::from_str
#[derive(serde::Deserialize, PartialEq, Eq, Clone, Debug)]
struct TargetSource
{
  pub services: Option<Vec<String>>,
  pub log_level: Option<u8>,
  pub hostname: Option<String>,
  pub debug_dump: Option<bool>,
  pub service_timeout: Option<u64>
}

// The final returned target
#[derive(PartialEq, Eq, Clone, Debug, Default)]
pub struct Target
{
  pub name: String,
  pub services: Vec<String>,
  pub logLevel: u8,
  pub hostname: String,
  pub debugDump: bool,
  pub serviceTimeout: u64
}

/**
  * # Errors
  *
  * - The matching configuration file for the target doesn't exist,
  * - The configuration file couldn't be parsed (usually for bad syntax),
  * - No services were provided in the configuration
 **/
pub fn source(name: String) -> Result<Target>
{
  use crate::{init::console::{Error, ErrorResult}};
  use std::fs;

  // Read toml contents from target config to string
  let targetToml = fs::read_to_string(file_path!(path!(crate::PREFIX, "target"), &name, "toml"))
                    .into_trace(Error::TargetNotFound)?;

  // Source the configuration
  let target: TargetSource = toml::from_str(&targetToml).into_trace(Error::TargetParse)?;

  let services = target.services.ok_or(Error::TargetMissingValue.trace(format!("services missing from {name}")))?;

  // Set our target values or the default if not specified in sourced config
  let logLevel = target.log_level.unwrap_or(1);
  let hostname = target.hostname.unwrap_or(String::from("localhost"));
  let debugDump = target.debug_dump.unwrap_or(false);
  let serviceTimeout = target.service_timeout.unwrap_or(5);

  if (target.debug_dump == Some(true) && !cfg!(debug_assertions))
  {
    use crate::{init::console::warn, console::Colour};
    // This will have no effect on release builds
    warn!("debug dump is enabled in target '{name}', but you are using a release build");
  }

  Ok(Target { name, services, logLevel, hostname, debugDump, serviceTimeout })
}
