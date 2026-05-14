// SPDX-License-Identifier: GPL-3.0-or-later
// Common code for ktctl/init in kickit project

#![deny(clippy::missing_errors_doc)]
#![allow(unused_parens)]
#![allow(non_snake_case)]

extern crate ruzstd;
extern crate thiserror;

pub mod init;
pub mod ktctl;
pub mod console;
pub mod state;
pub mod socket;

use std::{fmt, fmt::Display, num::ParseIntError};

#[derive(Eq, PartialEq, Copy, Clone, Debug, Default)]
pub enum Release { Stable, Testing, #[default] Unstable }

#[derive(Eq, PartialEq, Copy, Clone, Debug, Default)]
pub struct Version(u8, u8, u8, u8);

/*
 * This alias makes things alot less repetative, e.g.:
 *
 * `let myVeryImportByteData: Vec<u8> = Vec::new();`
 *
 * just becomes:
 *
 * `let myVeryImportByteData = Data::new();`
 */
pub type Data = Vec<u8>;

pub const RELEASE: Release = Release::Unstable;
// Version is formatted as [Top].[Rel].[Lower]-[Increment]
pub const VERSION: Version = Version(0, 1, 1, 2);
// Where the important files for kickit live
pub const PREFIX: &str = "/usr/lib/kickit";

display_enum! { Release }

impl Display for Version
{
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result
  {
    write!(f, "{}.{}.{}-{}", self.0, self.1, self.2, self.3)
  }
}

/**
  * # Errors
  *
  * - Couldn't convert input substring (radix) to a byte because it isn't valid hex
  */
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
#[inline] pub fn hex_data<Stringify: Display>(hexRaw: Stringify) -> Result<Data, ParseIntError>
{
  // Open a string on our hex data so we can get a slice of chars from it
  let hex = hexRaw.to_string();
  // The data vector will be increased for each hex
  let mut data = Data::with_capacity(hex.len() / 2);

  /*
   * Convert from a String to a vector of characters & then
   * split that into character chunks of 2
   */
  for single in (hex.chars().collect::<Vec<char>>().chunks(2))
  {
    // Collect the 2 characters as a String
    data.push(u8::from_str_radix(&single.iter().collect::<String>() as &str, 16)?);
  }

  Ok(data)
}

pub const PRETTY_VERSION: fn() -> String = ||
[
  // Display version as a string
  crate::VERSION.to_string(),

  if (crate::RELEASE == crate::Release::Stable)
  {
    // Nothing needs to be added here
    String::new()
  }
  else {
    // Add the current release at the end if unstable
    format!(" ({})", crate::RELEASE)
  }
]
  // And join both of those together without a seperator
  .join("");

// Get the name of the current binary (e.g. ktctl)
#[macro_export] macro_rules! binary
{
  () =>
  {
    {
      std::env::current_exe()
        .unwrap_or(env!("CARGO_PKG_NAME").into())
        .file_name()
        .unwrap_or(std::ffi::OsStr::new(env!("CARGO_PKG_NAME")))
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
      fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result
      {
        // Access all variants of the enum
        use $what::*;
        // Add each variant & their corresponding display text
        write!(f, "{}", match (self) { $($variant => $fmt,)* })
      }
    })*
  };
  /*
   * Display a variant as its name, for example:
   *
   * ```
   *   use crate::display_enum;
   *
   *   #[derive(Debug)] enum Mood { Happy, Sad, Angry }
   *
   *   // Each variant will be displayed as their name
   *   display_enum! { Mood }
   *
   *   fn main()
   *   {
   *     eprintln!("Today I am feeling {}", Mood::Happy);
   *   }
   * ```
   */
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
      let mut tempPath = std::path::PathBuf::new();
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
      $crate::path!($parent, $file).with_extension($ext)
    }
  };
}
/*
 * A macro for the absolute boilerplate that is setting a OnceLock, usage example:
 *
 * ```
 *   use std::sync::OnceLock;
 *   use nix::unistd::getuid;
 *   use crate::{letOnceLock, console::HandleError};
 *
 *   const USER_ID: OnceLock<u32> = OnceLock::new();
 *
 *   fn main()
 *   {
 *     letOnceLock! { let USER_ID = getuid() }.handle();
 *     eprintln!("your user's ID is: {}", USER_ID.get().unwrap());
 *   }
 * ```
 */
#[macro_export] macro_rules! oncelock
{
  { let $oncelock: path = $val: expr } =>
  {
    /*
     * Check OnceLock doesn't already have a value- it shouldn't because
     * this should be the first and only time our method is called
     */
    if ($oncelock.get().is_some())
    {
      Err(Error::Unknown.trace("OnceLock already has a value!"))
    }
    else if ($oncelock.set($val).is_err())
    {
      Err(Error::Unknown.trace("Failed to set a OnceLock!"))
    }
    else {
      Ok(())
    }
  };
}

// Detect this at compile-time to avoid headaches
#[cfg(not(target_os = "linux"))] compile_error!("unsupported platform; only Linux is supported");
