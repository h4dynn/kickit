//! Functions & traits to reduce boilerplate code and/or improve object-oriented stylizations

use std::{fmt::Display, collections::VecDeque, mem::MaybeUninit};

/// Alias trait for types that can be referenced to a str ([`String`], [`Box<str>`], [`&str`])
pub trait AnyStr: AsRef<str>
{
  /// Reference to universal &str type
  fn as_str_ref(&self) -> &str { self.as_ref() }
}

impl<T: AsRef<str>> AnyStr for T {}

/*
 * Data manipulation helpers
 * =========================
 */

/// Attest to a type's size, converting to [`None`] variant if empty
pub trait OptionizeEmpty: Sized
{
  fn empty_none(self) -> Option<Self>;
}

impl OptionizeEmpty for &str
{
  fn empty_none(self) -> Option<Self>
  {
    (!self.is_empty()).then_some(self)
  }
}

/**
  * A delimited iterator, which will iterate over each inner item with the delimiter ppended, except for the
  * last item. The delimiter is traditionally a single character, but since we are outputting, we use a
  * string, supporting multiple characters.
  *
  * # Example
  *
  * ```
  * use kickit::delim_iter;
  *
  * const SUPPORTED_ARCH: [&str; 3] = ["x86_64", "aarch64", "i386"];
  *
  * eprintln!("supported architectures: {}", delim_iter(SUPPORTED_ARCH.iter(), ", ").collect::<Vec<_>>());
  * ```
  */
#[derive(Clone)]
pub struct DelimIter<Inner: Iterator<Item: Display>>
{
  // If we are on the 1st item, we don't want to print the delimiter
  first: bool,
  // Typically just 1 character but because of the way this is implemented, it may be multiple
  delim: &'static str,
  // The item to iterate over
  inner: Inner
}

impl<I: Iterator<Item: Display>> Iterator for DelimIter<I>
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

/// Initialize a delimited iterator as an adaptor of an existing iterator
pub fn delim_iter<I: Iterator<Item: Display>>(inner: I, delim: &'static str) -> DelimIter<I>
{
  DelimIter {
    // Falsified following 1st iteration
    first: true,
    delim,
    // Main iterator
    inner
  }
}

/// Move elements from the front/back of a vector into output array
/**
  * Remove all of `SIZE` bytes from the front/back of the vector, moving them into an array. If you are
  * dumping from the front, a [`VecDeque`] will be more suitable than a [`Vec`], since it is double-ended.
  *
  * # Safety
  *
  * This doesn't check if your array is big enough to index through the length provided. You need to
  * implement those checks yourself before using this, otherwise index will fail, and throw an
  * out-of-bounds access panic.
  */
pub trait DumpVec<T>: Sized + Clone
{
  /// Remove all of `SIZE` bytes from the front of the vector
  fn dump_front<const SIZE: usize>(&mut self) -> [T; SIZE];
}

// Use VecDeque for front dumps. Standard vectors are inefficient for this, as they lack a front end
impl<T: Sized + Clone> DumpVec<T> for VecDeque<T>
{
  /**
    * We use [`MaybeUninit`] here since we COULD just use a `[T; SIZE]`, depend on `T` implementing
    * `Default` and use the default value but that creates unnecessary initialisations & adds an
    * extra unneeded dependency.
    *
    * # Usage
    *
    * ```
    * use std::collections::VecDeque;
    *
    * let mut abcdef = VecDeque::from(['a', 'b', 'c', 'd', 'e', 'f']);
    * // Dump a, b and c only into an array
    * let abc: [char; 3] = abcdef.dump_front();
    *
    * assert_eq!(abc, ['a', 'b', 'c']);
    * assert_eq!(abcdef.as_slice(), &['d', 'e', 'f']);
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
     * We unfortunately cannot `transmute()` here since Rust's implementation of the function is
     * compiler hard-coded, and cannot determine the size of generics (even though we can assert that
     * `size_of::<[MaybeUninit<T>; SIZE]>() == size_of::<[T; SIZE]>()`)
     * <https://github.com/rust-lang/rust/issues/61956>
     *
     * Pointer safety:
     *
     * * This pointer is valid for reads (as per <https://doc.rust-lang.org/std/ptr/index.html#safety>)
     *
     * * `T` is sized, having an alignment
     *
     * * Value has been initialized, by the above for loop
     */
    unsafe { std::ptr::read(<*const [MaybeUninit<T>; SIZE]>::cast(&raw const dump)) }

    // Experimental method <https://github.com/rust-lang/rust/issues/96097
    #[cfg(feature = "rustc_nightly")]
    unsafe { MaybeUninit::array_assume_init(dump) }
  }
}
