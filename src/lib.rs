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

use std::{fmt::Display, iter::Iterator, ops::Deref};

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

/*
 * A delimited vector iterator, which will iterate over each inner item with the delimiter
 * appended, except for the last item
 */
#[derive(Clone)]
pub struct DelimVecIter<T: Display>
{
  delim: char,
  // Where we are in the vector, make sure we don't go past its index max
  current: usize,
  vec: Vec<T>
}

pub const RELEASE: Release = Release::Unstable;
// Where the important files for kickit live
pub const PREFIX: &str = "/usr/lib/kickit";

display_enum!
{
  #[derive(Eq, PartialEq, Copy, Clone, Debug, Default)]
  pub enum Release
  {
    Stable, Testing, #[default] Unstable
  }
}

// Convert a Vector that may be empty to an Option
pub trait OptionEmptyVec: Sized
{
  fn empty_none(self) -> Option<Self>;
}

#[must_use]
pub fn PRETTY_VERSION() -> String
{
  [
    // Display version as a string
    env!("CARGO_PKG_VERSION").to_owned(),

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
    .join("")
}

impl<T, S: Deref<Target = Vec<T>>> OptionEmptyVec for S
{
  fn empty_none(self) -> Option<Self>
  {
    if ((*self).is_empty())
    {
      None
    }
    else {
      Some(self)
    }
  }
}

impl<T: Display> Iterator for DelimVecIter<T>
{
  type Item = String;

  fn next(&mut self) -> Option<String>
  {
    let reply =
    {
      match (self.current)
      {
        // This is the last item in the vector, so don't append the delimiter
        end if (end == self.vec.len() - 1) => Some(self.vec[self.current].to_string()),
        // Reached the end of the vector
        overflow if (overflow >= self.vec.len()) => None,
        // This is a regular item, append the delimiter
        _ => Some(format!("{}{}", self.vec[self.current], self.delim))
      }
    };

    // Move onto the next
    self.current += 1;
    reply
  }
}

impl<T: Display> DelimVecIter<T>
{
  #[must_use]
  pub const fn new(vec: Vec<T>, delim: char) -> Self
  {
    Self { delim, current: 0, vec }
  }
}

// Get the name of the current binary (e.g. ktctl)
#[macro_export]
macro_rules! binary
{
  () =>
  {
    {
      use std::env::current_exe;

      // Get the file that has been executed
      current_exe()
        .unwrap_or(env!("CARGO_PKG_NAME").into())
        .file_name()
        .unwrap_or(std::ffi::OsStr::new(env!("CARGO_PKG_NAME")))
        .display()
        .to_string()
    }
  };
}
/*
 * Create a wrapped value - a struct with a single-object tuple, that can
 * be dereferenced to the object
 */
#[macro_export(local_inner_macros)]
macro_rules! wrap
{
  {
    $(impl $(<$($gen: ident: $($dep: ident),*),*>)? Deref<Target = $inner: ty> for $name: ident;)+
  } =>
  {
    $(
      impl $(<$($gen: $($dep),+),+>)? std::ops::Deref for $name $(<$($gen),+>)?
      {
        type Target = $inner;
        // The compiler will automatically handle proper dereferencing after this
        fn deref(&self) -> &$inner
        {
          &self.0
        }
      }
      // and then we implement derefencing as mutable too!
      impl $(<$($gen: $($dep),+),+>)? std::ops::DerefMut for $name $(<$($gen),+>)?
      {
        fn deref_mut(&mut self) -> &mut $inner
        {
          &mut self.0
        }
      }
    )+
  };
}
// Implement std::fmt::Display for enumeration in a nicely formatted way
#[macro_export]
macro_rules! display_enum
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
  { $($name: ty { $($variant: ident => $fmt: expr),* }),* } =>
  {
    $(impl std::fmt::Display for $name
    {
      fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result
      {
        // Add each variant & their corresponding display text
        write!(f, "{}", match (self) { $(Self::$variant => $fmt,)* })
      }
    })*
  };
  /*
   * Display a variant as its name, for example:
   *
   * ```
   *   use crate::display_enum;
   *
   *   // Each variant will be displayed as their name
   *   display_enum!
   *   {
   *     #[derive(Thoughts, Behavior)]
   *     pub enum Feeling
   *     {
   *       Happy,
   *       Sad,
   *       Angry
   *     }
   *   }
   *
   *   fn main()
   *   {
   *     // output: "Today I am feeling Happy"
   *     eprintln!("Today I am feeling {}", Feeling::Happy);
   *   }
   * ```
   */
  { $(#[$ea: meta])? $vis: vis enum $name: ident { $($(#[$va: meta])? $var: ident),* } } =>
  {
    $(#[$ea])?
    $vis enum $name
    {
      $(
        $(#[$va])?
        $var
      ),*
    }

    impl std::fmt::Display for $name
    {
      fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result
      {
        write!(f, "{}", match (self)
        {
          $(
            // Convert the variant token provided directly in the macro to a string
            Self::$var => stringify!($var)
          ),*
        })
      }
    }
  };
  /*
   * Display a variant as its representing value, for example:
   *
   * ```
   *   use crate::display_enum;
   *
   *   #[derive(Debug)]
   *   enum Member
   *   {
   *     Ray = 130,
   *     Chloe = 622,
   *     Jayden = 905
   *   }
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
  { $(name: ty as $repr: ty),* } =>
  {
    $(impl std::fmt::Display for $name
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
#[macro_export]
macro_rules! path
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
#[macro_export(local_inner_macros)]
macro_rules! file_path
{
  ($parent: expr, $file: expr, $ext: expr) =>
  {
    {
      path!($parent, $file).with_extension($ext)
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
#[macro_export]
macro_rules! oncelock
{
  {
    $(
      $vis: vis static $name: ident: $ty: ty;
    )+
  } =>
  {
    $(
      $vis static $name: std::sync::OnceLock<$ty> = std::sync::OnceLock::new();
    )+
  };
  { $oncelock: ident = $val: expr } =>
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
  (&$oncelock: ident .unwrap_or($fallback: expr)) =>
  {
    $oncelock.get_or_init(|| { $fallback })
  };
  (&mut $oncelock: ident ?? $fallback: expr) =>
  {
    $oncelock.get_mut_or_init(|| { $fallback })
  };
  (&mut $oncelock: expr) =>
  {
    if let Some(inner) = $oncelock.get_mut()
    {
      Ok(inner)
    }
    else {
      Err(Error::Unknown.trace("OnceLock value has not been set yet!"))
    }
  };
  (&$oncelock: expr) =>
  {
    if let Some(inner) = $oncelock.get()
    {
      Ok(inner)
    }
    else {
      Err(Error::Unknown.trace("OnceLock value has not been set yet!"))
    }
  };
}

// Detect this at compile-time to avoid headaches
#[cfg(not(target_os = "linux"))] compile_error!("unsupported platform; only Linux is supported");
