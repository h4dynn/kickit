//! Target configuration file sourcing

use std::sync::OnceLock;
use crate::{init::init_console::KTErrorTrace, path, file_path};

// KTTargetSource is used for toml::from_str
#[derive(serde::Deserialize, PartialEq, Eq, Clone, Debug)]
struct KTTargetSource { pub services: Option<Vec<String>>, pub log_level: Option<u8>,
                        pub hostname: Option<String>, pub debug_dump: Option<bool> }

// The final returned target
#[derive(PartialEq, Eq, Clone, Debug, Default)]
pub struct KTTarget { pub name: String, pub services: Vec<String>,
                      pub logLevel: u8, pub hostname: String,
                      pub debugDump: bool }

// This gets a value when the target is sourced
pub static TARGET_NAME: OnceLock<String> = OnceLock::new();

///
/// # Errors
/// * The matching configuration file for the target doesn't exist,
/// * The configuration file couldn't be parsed (usually for bad syntax),
/// * No services were provided in the configuration
///
pub fn source(name: String) -> Result<self::KTTarget, KTErrorTrace>
{
  use crate::{init::init_console::{KTError, KTErrorTrace, ConvKTError}};
  use std::fs;

  // Read toml contents from target config to string
  let targetToml = fs::read_to_string(file_path!(path!(crate::PREFIX, "target"), &name, "toml"))
                    .trace(KTError::FileNotFound)?;

  // Source the configuration
  let sourcedTarget: KTTargetSource = toml::from_str(&targetToml).trace(KTError::TargetParseFail)?;

  // Set our target values or the default if not specified in sourced config
  let services = match (sourcedTarget.services)
  {
    Some(s) => Ok(s),
    None    => Err(KTErrorTrace::new(KTError::TargetMissingValue,
                                      &format!("services[] missing in {name}.toml")))
  }?;
  let logLevel = sourcedTarget.log_level.unwrap_or(1);
  let hostname = sourcedTarget.hostname.unwrap_or(String::from("localhost"));
  let debugDump = sourcedTarget.debug_dump.unwrap_or(false);

  if (sourcedTarget.debug_dump.is_some() && !cfg!(debug_assertions))
  {
    use crate::{warn, console::Colour};
    // This will have no effect on release builds
    warn!("debug dump is enabled in target '{name}', but you are using a release build");
  }

  Ok(KTTarget { name, services, logLevel, hostname, debugDump })
}
