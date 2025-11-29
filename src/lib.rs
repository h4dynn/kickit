// SPDX-License-Identifier: GPL-3.0-or-later
// Common code for ktctl/init in kickit project

#![deny(clippy::missing_errors_doc)]
#![allow(unused_parens)]
#![allow(non_snake_case)]

extern crate zstd;
extern crate thiserror;

use std::{fmt, fmt::Display, num::ParseIntError};
use crate::Release::Unstable;

pub mod init;
pub mod ktctl;
pub mod console;
pub mod state;
pub mod socket;

#[derive(Eq, PartialEq, Copy, Clone, Debug, Default)]
pub enum Release { Stable, Testing, #[default] Unstable }

#[derive(Eq, PartialEq, Copy, Clone, Debug, Default)]
pub struct Version(u8, u8, u8);

pub type Data = Vec<u8>;

pub const RELEASE: Release = Unstable;
// Version is formatted as [Top].[Rel].[Lower]
pub const VERSION: Version = Version(0, 1, 1);
// Where the important files for kickit live
pub const PREFIX: &str = "/usr/lib/kickit";

display_enum! { Release }

impl Display for Version
{
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result
  {
    write!(f, "{}.{}.{}", self.0, self.1, self.2)
  }
}

///
/// # Errors
///
/// * Couldn't convert input substring to a byte because it isn't valid hex
///
/*
 * Convert a string of hex to a vector of bytes, for example:
 *
 * ```
 *   // This should never panic
 *   assert_eq!(hex_data("48656c6c6f").unwrap().as_slice(), "Hello".as_bytes());
 *
 *   // ...and neither should this
 *   assert_eq!(*hex_data("377abcaf271c").unwrap(), [0x37, 0x7a, 0xbc, 0xaf, 0x27, 0x1c]);
 * ```
 */
#[inline] pub fn hex_data(h: impl Display) -> Result<Data, ParseIntError>
{
  // Open a string on our hex data so we can get a slice of chars from it
  let hex = h.to_string();
  // The data vector will be increased for each hex
  let mut data = Data::with_capacity(hex.len() / 2);

  /*
   * I know this is a messy way to do this but its better than
   * using 'str::from_utf8()' since we don't `.unwrap()` here
   */
  for single in (hex.as_bytes().chunks(2)
                    .map(|chunk| { let mut out = String::new();
                                    for c in (chunk) { out.push(*c as char) }
                                    out }))
  {
    data.push(u8::from_str_radix(&single as &str, 16)?);
  }

  Ok(data)
}

#[must_use] pub fn version() -> String
{
  use crate::Release::Stable;

  [
    // Display version as a string
    crate::VERSION.to_string(),

    // Add the current release at the end if unstable
    if (crate::RELEASE == Stable)
    {
      String::new()
    }
    else {
      format!(" ({})", crate::RELEASE)
    }
  ]
    // And join both of those together without a seperator
    .join("")
}

// Get the name of the current binary (e.g. ktctl)
#[macro_export] macro_rules! binary
{
  () =>
  {
    {
      use std::{env, ffi::OsStr};

      env::current_exe()
        .unwrap_or(env!("CARGO_PKG_NAME").into())
        .file_name()
        .unwrap_or(OsStr::new(env!("CARGO_PKG_NAME")))
        .display()
        .to_string()
    }
  };
}

// Implement std::fmt::Display for enumeration in a nicely formatted way
#[macro_export] macro_rules! display_enum
{
  /*
   * Implement some displayable text for each variant of an enum, for example:
   *
   * ```
   *   use crate::display_enum;
   *   enum Animal { Cat, Gecko, Hamster }
   *
   *   display_enum!
   *   {
   *     Animal {
   *       Cat => "cute",
   *       Gecko => "very cute",
   *       Hamster => "cute & tiny"
   *     }
   *   }
   *
   *   fn main()
   *   {
   *     // This will display "geckos are very cute"
   *     eprintln!("geckos are {}", Animal::Gecko);
   *   }
   * ```
   */
  { $($what: ty { $($variant: pat => $fmt: expr),* }),* } =>
  {
    $(impl std::fmt::Display for $what
    {
      fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error>
      {
        // Access all variants of the enum
        use $what::*;
        // Add each variant & their corresponding display text
        write!(f, "{}", match (self) { $($variant => $fmt,)* })
      }
    })*
  };

  { $($what: ty),* } =>
  {
    $(impl std::fmt::Display for $what
    {
      fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error>
      {
        /*
         * Use the pretty-debug implementation for enum's as a display because
         * the debug (:?) method provides the variant's name
         */
        write!(f, "{self:?}")
      }
    })*
  };

  /*
   * Display a variant as its representing value, for example:
   *
   * ```
   *   use crate::display_enum;
   *
   *   #[derive(Debug)] enum Member { Ray = 130, Chloe = 622, Jayden = 905 }
   *
   *   // Each variant will be displayed as their ID
   *   display_enum! { Member as u128 }
   *
   *   fn main()
   *   {
   *     // This will display "Chloe's member ID is: 622"
   *     eprintln!("Chloe's member ID is: {}", Member::Chloe);
   *   }
   * ```
   */
  { $(what: ty as $repr: ty),* } =>
  {
    $(impl std::fmt::Display for $what
    {
      fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error>
      {
        write!(f, "{}", self as $repr)
      }
    })*
  };
}

/*
 * Concatenate multiple files/directories together into a PathBuf, for example:
 *
 * ```
 *   // Setup the configuration prefix that can be changed
 *   pub const CONFIG_PREFIX: &str = "/etc";
 *
 *   fn configExists() -> bool
 *   {
 *     use std::fs;
 *     use crate::path;
 *
 *     // Concatenate together, so in this case '/etc/my_program.conf'
 *     fs::metadata(path!(CONFIG_PREFIX, "my_program.conf")).is_ok()
 *   }
 * ```
 */
#[macro_export] macro_rules! path
{
  ($($sub: expr),*) =>
  {
    {
      use std::path::PathBuf;
      let mut tempPath = PathBuf::new();

      $(tempPath.push($sub);)*

      tempPath
    }
  };
}

/*
 * Create a file PathBuf which ends in an extension, for example:
 *
 * ```
 *   fn hasSystemTarget() -> bool
 *   {
 *     use std::fs;
 *     use crate::{file_path, path};
 *
 *     // Concatenate directory + file name + extension together
 *     fs::metadata(file_path!(path!(crate::PREFIX, "target"), "system", "toml")).is_ok()
 *   }
 * ```
 */
#[macro_export] macro_rules! file_path
{
  ($parent: expr, $file: expr, $ext: expr) =>
  {
    {
      use std::path::PathBuf;
      let mut tempPath = PathBuf::from($parent);

      tempPath.push($file);
      tempPath.with_extension($ext)
    }
  };
}

// Simple way of returning if a condition equates to true
#[macro_export] macro_rules! returnif
{
  ($condition: expr) => { if ($condition) { return } };
  ($condition: expr, $val: path) => { if ($condition) { return $val } };
}

#[macro_export]
macro_rules! letOnceLock
{
  { let $oncelock: path = $val: expr } =>
  {
    /*
     * Check OnceLock doesn't already have a value- it shouldn't because
     * this should be the first and only time our method is called
     */
    if ($oncelock.get().is_some())
    {
      Err(KTErrorTrace::new(KTError::Unknown, "OnceLock already has a value!"))
    }
    else if ($oncelock.set($val).is_err())
    {
      Err(KTErrorTrace::new(KTError::Unknown, "Failed to set a OnceLock!"))
    }
    else {
      Ok(())
    }
  };
}

// Detect this at compile-time to avoid headaches
#[cfg(not(target_os = "linux"))] compile_error!("unsupported platform; only Linux is supported");
