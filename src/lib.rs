// SPDX-License-Identifier: GPL-3.0-or-later
// Common code for ktctl/init in kickit project

#![deny(clippy::missing_errors_doc)]
#![allow(unused_parens)]
#![allow(non_snake_case)]

extern crate zstd;
extern crate thiserror;

use std::{fmt, fmt::Display, num::ParseIntError};
use crate::Release::*;

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
 *   // This *should* never panic
 *   assert_eq!(hex_data("48656c6c6f").unwrap().as_slice(), "Hello".as_bytes());
 *
 *   // ...and neither should this
 *   assert_eq!(hex_data("377abcaf271c").unwrap(), vec![0x37, 0x7a, 0xbc, 0xaf, 0x27, 0x1c]);
 * ```
 */
#[inline] pub fn hex_data(hex: impl Display) -> Result<Data, ParseIntError>
{
  // Open a string on our hex data so we can get a slice of chars from it
  let stringData = hex.to_string();
  // The data vector will be increased for each hex
  let mut data = Data::new();
  // Increases by 2 for each iteration (each hex should be 2 chars)
  let mut start: usize = 0;

  while (start < stringData.len())
  {
    // Read our hex, which should be 2 bytes
    let hex = &stringData[start..start + 2];
    // Add the corresponding byte for the hex using a radix
    data.push(u8::from_str_radix(hex, 16)?);
    start += 2;
  }

  Ok(data)
}

pub fn version() -> String
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
  { $($what: ty { $($variant: path => $fmt: expr),* }),* } =>
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

  /*
   * Display a variant as just its name, for example:
   *
   * ```
   *   use crate::display_enum;
   *
   *   #[derive(Debug)] enum Finished { Ray = 1, Chloe = 2, Jayden = 3 }
   *
   *   // Each variant in `Finished` will be displayed as its name
   *   display_enum! { Finished }
   *
   *   fn main()
   *   {
   *     // This will display "...and finishing first place: Ray"
   *     eprintln!("...and finishing first place: {}", Finished::Ray);
   *   }
   * ```
   *
   * (note): This *depends* on your enum implementing std::fmt::Debug
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
}

/*
 * Concatenate multiple directories together into a PathBuf, for example:
 *
 * ```
 *   // Setup the configuration prefix that can be changed
 *   pub const CONFIG_PREFIX: &str = "/etc";
 *
 *   fn config_exists() -> bool
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
 *   fn has_system_target() -> bool
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

/*
 * Boilerplate for creating a new default, struct or from something, for example:
 *
 * ```
 *   struct Example { a: String, b: u8, c: bool }
 *
 *   fn example()
 *   {
 *     use crate::New;
 *
 *     let hello = New!(String{ "hello world" });
 *     let empty = New!(String);
 *     let example = New!(Example { a = empty, b = 0, c = false });
 *
 *     dbg!(hello, empty, example);
 *   }
 * ```
 */
#[macro_export] macro_rules! New
{
  ($what: ty) => { { <$what>::default() } };
  ($what: ty { $($val: ident = $content: expr),* }) => { $what { $($val: $content)* } };
  ($what: ty { $val: expr }) => { <$what>::from($val) };
}

// Simple way of returning if a condition equates to true
#[macro_export] macro_rules! returnif
{
  ($condition: expr) => { if ($condition) { return } };
  ($condition: expr, $val: path) => { if ($condition) { return $val } };
}

// Detect this at compile-time to avoid headaches
#[cfg(not(target_os = "linux"))] compile_error!("unsupported platform; only Linux is supported");
