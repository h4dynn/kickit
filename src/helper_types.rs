//! Types used to help with certain tasks, such as dynamic ownership

use std::{fmt, fmt::{Display, Debug}, mem::MaybeUninit};

/// A synchronizing type for transferring value ownership across threads
/**
  * # Abstract
  *
  * A value that can be moved between different threads, with the new owner being tracked with an
  * identifier. This identifier functions as a dummy type (ZST) or may be used like an enum, for values
  * with volatile ownership. Either way, it must be static to ensure the owner is always identifiable.
  *
  * This type is included in the crate for use in service control ([`service::ServiceBody`]).
  *
  * # Motivation
  *
  * The desired behavior of this wrapper type can be replicated with a simple [`Option`]. However, this
  * can lead to ambiguity in who takes ownership of the value. The sole purpose of this type is to avoid
  * that ambiguity for code clarity.
  *
  * This type can also eliminate the need to use other wrapper types like [`Arc`] for values. It is not
  * a replacement, [`Arc`] values can be accessed at any-time while the value stored by this type can
  * only be accessed when we own it.
  *
  * ## **Thread Safety**
  *
  * It is important to know that this type is not thread-safe in the traditional sense (shared, static
  * access of a value), rather it is thread-safe because it is only held in 1 thread at any given time,
  * and cannot be accessed or modified from a foreign thread without first being taken (so, we depend
  * on [`Send`] implementation). If you need a shared thread-safe type, ditch this entirely and refer
  * to the [`std::sync`] module, providing the types [`Arc`] and [`Mutex`] among others.
  *
  * # Example usage
  *
  * ```
  * # use kickit::{OwnerIdentity, DynamicOwnership};
  * struct Phone
  * {
  *   contacts: Vec<&'static str>
  * }
  *
  * #[derive(Clone, Copy, Debug)]
  * pub struct Fiona;
  *
  * pub struct Ford
  * {
  *   // Fiona takes the phone
  *   pub main_phone: DynamicOwnership<Phone, Fiona>,
  *   secret_phone: Phone
  * }
  *
  * impl Fiona
  * {
  *   fn check_for_cheater(phone: DynamicOwnership<Phone, Self>) -> bool
  *   {
  *     phone.take(Self{}).unwrap().contacts.iter().any(|name| name == "Patty")
  *   }
  * }
  *
  * impl OwnerIdentity for Fiona {}
  *
  * let cheater = Ford
  * {
  *   main_phone: DynamicOwnership::capture(Phone { contacts: vec!["Mom"] }),
  *   secret_phone: Phone { contacts: vec!["Patty"] }
  * };
  *
  * // no cheater here!.........
  * assert!(!Fiona::check_for_cheater(cheater.main_phone));
  * ```
  *
  * [`service::ServiceBody`]: crate::init::service::ServiceBody
  * [`Arc`]: std::sync::Arc
  * [`Mutex`]: std::sync::Mutex
  */
#[derive(Debug)]
pub struct DynamicOwnership<T: Sized + Send, I: OwnerIdentity>
{
  val: MaybeUninit<T>,
  // The readability of `val` is directly dependent on this
  owner: Owner<I>
}

/// Tracked ownership state for the [`DynamicOwnership`] wrapper type
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum Owner<Id: OwnerIdentity>
{
  /// Value hasn't been initialized
  Nobody,
  /// We own this value's instance
  This,
  /// Owned by someone else
  Foreign(Id)
}

/// Trait that acts as a dummy for debugging purposes & matching to describe the new owner of a value
pub trait OwnerIdentity: Debug + Copy + Send + Sync + 'static {}

/// Possible error outcomes of trying to take a value which isn't owned
#[derive(Debug)]
pub enum DynamicOwnershipTakeError<I: OwnerIdentity>
{
  /// Value has not yet been initialized
  /**
    * # Example
    *
    * ```rust,should_panic
    * # use kickit::{DynamicOwnership, OwnerIdentity, DynamicOwnershipTakeError};
    * #[derive(Clone, Copy, Debug, Default)]
    * struct X;
    *
    * impl OwnerIdentity for X {}
    *
    * let mut y: DynamicOwnership<u32, X> = DynamicOwnership::new();
    * // error: "cannot take ownership of a value that hasn't been initialized"
    * y.take_as_default().unwrap();
    * ```
    */
  Uninitialized,
  /// Value has already been taken
  /**
    * # Example
    *
    * ```rust,should_panic
    * # use kickit::{DynamicOwnership, OwnerIdentity, DynamicOwnershipTakeError};
    * #[derive(Clone, Copy, Debug, Default)]
    * struct ForeignOwner;
    *
    * impl OwnerIdentity for ForeignOwner {}
    *
    * let mut y: DynamicOwnership<&'static str, ForeignOwner> = DynamicOwnership::from("text value");
    *
    * {
    *   let _ = y.take_as_default().unwrap();
    * }
    *
    * // error: "cannot take ownership of a value we don't own (owner: `ForeignOwner`)"
    * y.take_as_default().unwrap();
    * ```
    */
  Unowned(I)
}

impl<T: Sized + Send, I: OwnerIdentity> DynamicOwnership<T, I>
{
  /// Default, uninitialized value
  #[must_use]
  pub const fn new() -> Self
  {
    Self { val: MaybeUninit::uninit(), owner: Owner::Nobody }
  }

  /// Initializes a new value from an empty wrapper
  /**
    * # Panics
    *
    * This method expects `self` to be in its uninitialized state (created by [`new`]). It cannot be
    * owned or taken.
    *
    * [`new`]: #method.new
    */
  pub fn init(&mut self, inner: T)
  {
    std::assert_matches!(self.owner, Owner::Nobody, "cannot initialize a value that is owned/taken");

    // Initialize the value in its place
    self.val.write(inner);
    // Just initializing the value means nothing if we don't know that we own it
    self.owner = Owner::This;
  }

  /// Identify the current owner of the value
  pub fn owner(&self) -> Owner<I>
  {
    self.owner
  }

  /// Transfer ownership, akin to using [`std::mem::take`] on an Option except the owner is tracked
  /**
    * # Errors
    *
    * The value must currently be owned by us, it cannot have already been taken, or be uninitialized.
    */
  pub fn take(&mut self, new_owner: I) -> Result<T, DynamicOwnershipTakeError<I>>
  {
    match (self.owner)
    {
      Owner::Nobody => Err(DynamicOwnershipTakeError::Uninitialized),
      Owner::Foreign(id) => Err(DynamicOwnershipTakeError::Unowned(id)),
      Owner::This => Ok(unsafe { std::mem::replace(self, Self::taken_by(new_owner)).val.assume_init() })
    }
  }

  /// Obtain a shared reference to the inner value, if its owned
  pub fn get(&self) -> Option<&T>
  {
    self.is_owned().then_some(unsafe { self.val.assume_init_ref() })
  }

  /// Obtain a shared mutable reference to the inner value, if its owned
  pub fn get_mut(&mut self) -> Option<&mut T>
  {
    self.is_owned().then_some(unsafe { self.val.assume_init_mut() })
  }

  /// Consume this wrapper, dropping the value if we own it
  pub fn disown(mut self)
  {
    if (self.is_owned())
    {
      // Using `std::mem::drop` on a `MaybeUninit` does nothing
      unsafe { self.val.assume_init_drop(); }
    }
    self.owner = Owner::Nobody;
  }

  pub fn is_owned(&self) -> bool
  {
    // Using a match instead of equation eliminates the `PartialEq` impl dependency
    matches!(self.owner, Owner::This)
  }

  // Construction of the replacement value upon relocation (in `take`)
  const fn taken_by(id: I) -> Self
  {
    Self { val: MaybeUninit::uninit(), owner: Owner::Foreign(id) }
  }
}

impl<T: Sized + Send, I: OwnerIdentity> Default for DynamicOwnership<T, I>
{
  fn default() -> Self
  {
    Self::new()
  }
}

impl<T: Sized + Send, I: OwnerIdentity> From<T> for DynamicOwnership<T, I>
{
  /// Construct a new dynamic ownership instance from an existing value
  fn from(inner: T) -> Self
  {
    Self { val: MaybeUninit::new(inner), owner: Owner::This }
  }
}

impl<T: Sized + Send, I: OwnerIdentity + Default> DynamicOwnership<T, I>
{
  /// Use default value of `I` as identification when taking
  /**
    * # Errors
    *
    * Refer to [`take`].
    */
  pub fn take_as_default(&mut self) -> Result<T, DynamicOwnershipTakeError<I>>
  {
    self.take(I::default())
  }
}

impl<I: OwnerIdentity> Display for DynamicOwnershipTakeError<I>
{
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result
  {
    write!(f, "{}", match (self)
    {
      Self::Uninitialized => String::from("cannot take ownership of a value that hasn't been initialized"),
      Self::Unowned(owner) => format!("cannot take ownership of a value we don't own (owner: {owner:?})"),
    })
  }
}
