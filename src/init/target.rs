//! Target configuration file sourcing

use crate::init::console::prelude::*;

oncelock! {
  /// Globally shared target configuration, sourced early in kickit's execution
  pub static TARGET: OnceLock<Target>;
}

// Used for toml::from_str, the optional values have defaults to fallback to
#[derive(serde::Deserialize, PartialEq, Eq, Clone, Debug)]
struct Config
{
  pub services: Option<Vec<String>>,
  pub hostname: Option<String>,
  pub debug_dump: Option<bool>,
  pub service_timeout: Option<u64>
}

// The final returned target
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct Target
{
  // The name of the currently used target
  pub name: String,
  // All the services that will be ran
  pub services: Vec<String>,
  // The system's hostname, will be localhost if not set
  pub hostname: String,
  // Store kickit's run assets in permanent storage for debugging
  pub debug_dump: bool,
  // How long we are willing to wait for a RunOnce service to start
  pub service_timeout: u64
}

mod Defaults
{
  // Default options
  pub const HOSTNAME: &str = "localhost";
  pub const SERVICE_TIMEOUT_SECONDS: u64 = 5;
}

/**
  * # Errors
  *
  * * The matching configuration file for the target doesn't exist
  *
  * * The configuration file couldn't be parsed (usually for bad syntax)
  *
  * * No services were provided in the configuration
 **/
pub fn source(name: String) -> Result<Target>
{
  use std::fs;

  // Read toml contents from target config to string
  let targetToml = fs::read_to_string(path!(crate::PREFIX, "target", format!("{name}.toml")))
                      .into_trace(Error::TargetNotFound).subject(&name)?;

  // Source the configuration
  let target: Config = toml::from_str(&targetToml).into_trace(Error::TargetParse)?;

  let services = target.services.ok_or(Error::TargetMissingValue.trace(format!("services missing from {name}")))?;

  // Set our target values or the default if not specified in sourced config
  let hostname = target.hostname.unwrap_or(String::from(Defaults::HOSTNAME));
  let debug_dump = target.debug_dump.unwrap_or_default();
  let service_timeout = target.service_timeout.unwrap_or(Defaults::SERVICE_TIMEOUT_SECONDS);

  if (target.debug_dump == Some(true) && !cfg!(debug_assertions))
  {
    warn!("`debug_dump` configuration option has no effect on release builds");
  }

  Ok(Target { name, services, hostname, debug_dump, service_timeout })
}
