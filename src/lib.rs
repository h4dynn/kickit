// SPDX-License-Identifier: GPL-3.0-or-later
// Common code for ktctl/init in kickit project

#![deny(clippy::missing_errors_doc)]
#![allow(unused_parens)]
#![allow(non_snake_case)]

extern crate ruzstd;
extern crate thiserror;
extern crate derive_more;

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
#[derive(Clone,)]
pub struct DelimVecIter<T: Display>
{
  delim: char,
  // Where we are in the vector, make sure we don't go past its index max
  current: usize,
  vec: Vec<T>
}

#[cfg(feature = "stable")]
pub const RELEASE: Release = Release::Stable;

#[cfg(not(feature = "stable"))]
pub const RELEASE: Release = Release::Unstable;

// Where the important files for kickit live
pub const PREFIX: &str = "/usr/lib/kickit";

#[derive(Eq, PartialEq, Copy, Clone, Debug, Default, derive_more::Display)]
pub enum Release
{
  Stable, Testing, #[default] Unstable
}

// Convert a Vector that may be empty to an Option
pub trait OptionEmptyVec: Sized
{
  fn empty_none(self) -> Option<Self>;
}

// This is just a nice way to do nothing with a value
pub trait TrashUnused: Sized
{
  fn trash(self) {}
}

pub trait DumpVec: Sized
{
  /*
   * Remove all of `SIZE` bytes from the front of the vector, moving them into
   * an array
   *
   * SAFETY: This doesn't check if your array is big enough to index through
   * the length provided. You need to implement those checks yourself before
   * using this!
   */
  fn front_dump<const SIZE: usize>(&mut self) -> [u8; SIZE];
  // Same as the above but move from the end of the vector
  fn back_dump<const SIZE: usize>(&mut self) -> [u8; SIZE];
}

#[must_use]
pub fn version() -> String
{
  use crate::{tern, Release::Stable};

  [
    // Display version as a string
    env!("CARGO_PKG_VERSION").to_owned(),
    tern! {
      crate::RELEASE == Stable => String::new(),
      else => format!(" ({})", crate::RELEASE)
    }
  ]
    // And join both of those together without a seperator
    .join("")
}

impl<T, S: Deref<Target = Vec<T>>> OptionEmptyVec for S
{
  fn empty_none(self) -> Option<Self>
  {
    tern! {
      (*self).is_empty() => None,
      else => Some(self)
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

// A VecDeque is the way to go for this, since they are designed to be able to remove from the front
impl DumpVec for std::collections::VecDeque<u8>
{
  fn front_dump<const SIZE: usize>(&mut self) -> [u8; SIZE]
  {
    let mut dump = [0u8; SIZE];

    for index in (&mut dump)
    {
      *index = self.pop_front().unwrap();
    }

    dump
  }

  fn back_dump<const SIZE: usize>(&mut self) -> [u8; SIZE]
  {
    let mut dump = [0u8; SIZE];

    for index in (&mut dump)
    {
      *index = self.pop_back().unwrap();
    }

    dump
  }
}

impl DumpVec for Vec<u8>
{
  fn front_dump<const SIZE: usize>(&mut self) -> [u8; SIZE]
  {
    let mut dump = [0u8; SIZE];

    for index in (&mut dump)
    {
      *index = self.remove(0);
    }

    dump
  }

  fn back_dump<const SIZE: usize>(&mut self) -> [u8; SIZE]
  {
    let mut dump = [0u8; SIZE];

    for index in (&mut dump)
    {
      *index = self.pop().unwrap();
    }

    dump
  }
}

impl<S: Sized> TrashUnused for S {}

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

// A C-style ternary expression with different syntax due to Rust macro fragments (? can't come after expr)
#[macro_export]
macro_rules! tern
{
  { $eval: expr => $cond: expr, $($eeval: expr => $econd: expr,)* else => $fallback: expr } =>
  {
    if ($eval)
    {
      $cond
    }
    $(
      else if ($eeval)
      {
        $econd
      }
    )*
    else {
      $fallback
    }
  };
}

#[macro_export]
macro_rules! continueif
{
  ($eval: expr) =>
  {
    if ($eval)
    {
      continue;
    }
  };
}

#[macro_export]
macro_rules! breakif
{
  ($eval: expr $(=> $ret: expr)?) =>
  {
    {
      if ($eval)
      {
        break $($ret)?;
      }
    }
  };
}

// Detect this at compile-time to avoid headaches
#[cfg(not(target_os = "linux"))]
compile_error!("unsupported platform; only Linux is supported");
