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

use std::{fmt::Display, iter::Iterator, collections::VecDeque, mem::MaybeUninit, ops::Deref};

pub type Data = Vec<u8>;
pub type DequeData = VecDeque<u8>;
// A non-resizable String equivilant
pub type BoxedStr = Box<str>;

/*
 * A delimited iterator, which will iterate over each inner item with the delimiter
 * appended, except for the last item
 */
#[derive(Clone)]
pub struct DelimIter<T: Display, Inner: Iterator<Item = T>>
{
  // If we are on the 1st item, we don't want to print the delimiter.
  first: bool,
  delim: &'static str,
  inner: Inner
}

#[cfg(feature = "stable")]
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(not(feature = "stable"))]
pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (unstable)");

// Where the important files for kickit live
pub const PREFIX: &str = "/usr/lib/kickit";

// This is just a nice way to do nothing with a value, equivilant to `let _ = fn();`
pub trait TrashUnused
{
  fn trash(&self) {}
}

// Convert a Vector that may be empty to an Option
pub trait OptionEmptyVec: Sized
{
  fn empty_none(self) -> Option<Self>;
}

/*
 * Remove all of `SIZE` bytes from the front/back of the vector, moving them
 * into an array. If you are dumping from the front, a `VecDeque` will be
 * more suitable than a `Vec`, since it is double-ended.
 *
 * SAFETY: This doesn't check if your array is big enough to index through
 * the length provided. You need to implement those checks yourself before
 * using this!
 */
pub trait DumpVec<T>: Sized
{
  // Remove all of SIZE bytes from the front of the vector
  fn dump_front<const SIZE: usize>(&mut self) -> [T; SIZE];
}

pub fn delim_iter<T: Display, I: Iterator<Item = T>>(inner: I, delim: &'static str) -> DelimIter<T, I>
{
  DelimIter { delim, inner, first: true }
}

impl<T: Display, Inner: Iterator<Item = T>> Iterator for DelimIter<T, Inner>
{
  type Item = String;

  fn next(&mut self) -> Option<String>
  {
    let current = self.inner.next()?;

    if (self.first)
    {
      self.first = false;
      // No formatting needed here!
      Some(current.to_string())
    }
    else {
      Some(format!("{}{}", self.delim, current))
    }
  }
}

impl<T, S: Deref<Target = Vec<T>>> OptionEmptyVec for S
{
  fn empty_none(self) -> Option<Self>
  {
    tern! { (*self).is_empty() => None, _ => Some(self) }
  }
}

// A VecDeque is the way to go for this, since they are designed to be able to remove from the front
impl<T> DumpVec<T> for VecDeque<T>
{
  /*
   * We use a `MaybeUninit` here since we COULD just use a `[T; SIZE]`, depend on T implementing
   * Default and use the default value but that's creates unnecessary initialisations + adds an
   * extra unneeded dependency
   *
   * Usage:
   * ```
   *   let mut abcdef = VecDeque::from(['a', 'b', 'c', 'd', 'e', 'f']);
   *   // Dump a, b and c only into an array
   *   let abc: [char; 3] = abcdef.dump_front();
   *
   *   assert_eq!(abc, ['a', 'b', 'c']);
   *   assert_eq!(abcdef.as_slice(), &['d', 'e', 'f']);
   * ```
   */
  fn dump_front<const SIZE: usize>(&mut self) -> [T; SIZE]
  {
    // All values in here are guaranteed to become initialised by the for loop
    let mut dump: [MaybeUninit<T>; SIZE] = [const { MaybeUninit::uninit() }; SIZE];

    // Index through each position, which causes a panic if there's not enough in the vector
    for index in (&mut dump)
    {
      // Move out the value from the front of the vector over to the array
      index.write(self.pop_front().unwrap());
    }

    /*
     * We unfortunately cannot `transmute()` here since it doesn't accept generics, so
     * we gotta rely on pointers instead <https://github.com/rust-lang/rust/issues/61956>
     */
    unsafe { dump.as_ptr().cast::<[T; SIZE]>().read() } //unsafe { transmute::<_, [T; SIZE]>(dump) }
  }
}

impl<S> TrashUnused for S {}

// Get the name of the current binary (e.g. ktctl)
#[macro_export]
macro_rules! binary
{
  () => {
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
    $(impl $(<$($gen: ident $(: $($dep: path),*)?),*>)? Deref<Target = $inner: ty> for $name: ident;)+
  } => {
    $(
      impl $(<$($gen $(: $($dep),+)?),+>)? std::ops::Deref for $name $(<$($gen),+>)?
      {
        type Target = $inner;
        // The compiler will automatically handle proper dereferencing after this
        fn deref(&self) -> &$inner
        {
          &self.0
        }
      }
      // and then we implement derefencing as mutable too!
      impl $(<$($gen $(: $($dep),+)?),+>)? std::ops::DerefMut for $name $(<$($gen),+>)?
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
      use std::path::PathBuf;
      let mut tempPath = PathBuf::new();

      $(
        tempPath.push($sub);
      )*
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
  } => {
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

// Get an enum's variant from its string form
#[macro_export]
macro_rules! enum_from_str
{
  ($str: expr => $($variant: ident)|*) =>
  {
    match ($str)
    {
      $(
        stringify!($variant) => Some(Self::$variant),
      )*
      _ => None
    }
  };
}

// A C-style ternary expression with different syntax due to Rust macro fragments (? can't come after expr)
#[macro_export]
macro_rules! tern
{
  { $eval: expr => $cond: expr, _ => $fallback: expr } =>
  {
    if ($eval)
    {
      $cond
    }
    else {
      $fallback
    }
  };
  { $firstEval: expr => $firstCond: expr, $($eval: expr => $cond: expr),* } =>
  {
    if ($firstEval)
    {
      Some($firstCond)
    }
    $(else if ($eval)
    {
      Some($cond)
    })*
    else {
      None
    }
      .unwrap_or_default()
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
