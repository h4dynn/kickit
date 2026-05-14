//! Target configuration file sourcing

use crate::{init::init_console::Result, path, file_path};

// KTTargetSource is used for toml::from_str
#[derive(serde::Deserialize, PartialEq, Eq, Clone, Debug)]
struct TargetSource
{
  pub services: Option<Vec<String>>,
  pub log_level: Option<u8>,
  pub hostname: Option<String>,
  pub debug_dump: Option<bool>
}

// The final returned target
#[derive(PartialEq, Eq, Clone, Debug, Default)]
pub struct Target
{
  pub name: String,
  pub services: Vec<String>,
  pub logLevel: u8,
  pub hostname: String,
  pub debugDump: bool
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
  use crate::{init::init_console::{Error, ErrorTrace, ErrorResult}};
  use std::fs;

  // Read toml contents from target config to string
  let targetToml = fs::read_to_string(file_path!(path!(crate::PREFIX, "target"), &name, "toml"))
                    .trace(Error::FileNotFound)?;

  // Source the configuration
  let sourcedTarget: TargetSource = toml::from_str(&targetToml).trace(Error::TargetParse)?;

  let services = sourcedTarget.services.ok_or(ErrorTrace::new(Error::TargetMissingValue,
                                                  &format!("services[] missing in {name}.toml")))?;

  // Set our target values or the default if not specified in sourced config
  let logLevel = sourcedTarget.log_level.unwrap_or(1);
  let hostname = sourcedTarget.hostname.unwrap_or(String::from("localhost"));
  let debugDump = sourcedTarget.debug_dump.unwrap_or(false);

  if (sourcedTarget.debug_dump == Some(true) && !cfg!(debug_assertions))
  {
    use crate::{init::init_console::warn, console::Colour};
    // This will have no effect on release builds
    warn!("debug dump is enabled in target '{name}', but you are using a release build");
  }

  Ok(Target { name, services, logLevel, hostname, debugDump })
}
