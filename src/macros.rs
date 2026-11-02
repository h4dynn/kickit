/*!
 * # Macros
 *
 * Various macros of differing functions & usages, used globally across kickit.
 */

/*
 * Object definition & behavior control macros
 * ===========================================
 */

/// Continue to next iteration if condition is met
#[macro_export]
macro_rules! continueif
{
  ($cond: expr) =>
  {
    if ($cond)
    {
      continue;
    }
  };
}

/// Break iteration if condition is met, optionally with a return value (loop exclusive)
#[macro_export]
macro_rules! breakif
{
  ($cond: expr $(=> $ret: expr)?) =>
  {
    if ($cond)
    {
      break $($ret)?;
    }
  };
}

/// Module definitions with support for automatically using all items in a module
/**
  * # Motivation
  *
  * Complex code in `kickit` is stored in divided module files, preventing from a singular source file
  * from becoming too large. This macro is used to automatically `use` all items in these modules (with
  * a simple glob, `use mod::*`). It includes a "pseudo attribute", localized for use with this macro
  * only, called `internal` for this.
  *
  * # Example
  *
  * ```
  * //! My super cool and complex module
  *
  * modules! {
  *   // Without a visibility, items are used privately in this scope
  *   #[internal]
  *   pub(crate) mod my_super_cool_objects;
  *
  *   // You can also include modules without the attribute
  *   pub mod sub_module;
  *
  *   // Or add a visibility for the use statement
  *   #[internal(pub)]
  *   mod more_objects;
  * }
  * ```
  */
#[macro_export(local_inner_macros)]
macro_rules! modules
{
  {
    $(
      $(#[$($attr: tt)*])*
      $vis: vis mod $id: ident;
    )+
  } => {
    $(
      $(modules!{@Attribute $id: $($attr)*})*
      $vis mod $id;
    )+
  };

  // Private use (default)
  {@Attribute $id: ident: internal} => { use $id::*; };
  // Explicit visibility for use
  {@Attribute $id: ident: internal ($export_vis: vis)} => { $export_vis use $id::*; };

  // TO-DO: since this expansion is seperate to the definition, we have to fallback to macro attributes
  {@Attribute $id: ident: $other_attr: ident} => { $other_attr!{$id} };
}

/// Define traditional C-style flags in Rust fashion
/**
  *
  * This implementation is incredibly similar to the `bitflags` crate. Instead, here we represent
  * single flags in an enum to be idiomatic, and a struct for concatenated flags.
  *
  * # Motivation & comparisons
  *
  * External crates such as [`nix`] have macros like `libc_bitflags` (private) for these C-style
  * flags. For [`nix`], this uses constant values defined under a zero-sized structure. This works
  * very well for FFI, however, we can also represent these flags as an enum, and a structure of
  * concatenated flags.
  *
  * Representing like this allows for more of an idiomatic, Rust-like definition whilst retaining
  * C compatibility.
  *
  * # Example
  *
  * ```
  * # use kickit::{c_style_flags, c_style_flags_construct};
  * c_style_flags! {
  *   pub enum Emotion: u8
  *   {
  *     Happy = 20,
  *     Sad = 1,
  *     Angry = 100
  *   }
  *
  *   // Dereferences to u8
  *   pub struct Emotions([Emotion] as u8);
  * }
  *
  * macro_rules! emotions
  * {
  *   [$($e: tt),*] =>
  *   {
  *     // Helper macro is provided 
  *     c_style_flags_construct!(<Emotions>::from([$($e),*]: [Emotion] as u8))
  *   };
  * }
  *
  * fn main()
  * {
  *   // Sad (1) and Angry (100) are concatenated with bitwise OR operator
  *   let sad_and_angry = emotions![Sad, Angry];
  *   let bittersweet = emotions![Happy, Sad];
  *   let everything = emotions![Happy, Sad, Angry];
  *
  *   // 100 | 1 = 101
  *   assert_eq!(*sad_and_angry, 101);
  *   // 20 | 1 = 21
  *   assert_eq!(*bittersweet, 21);
  *   // 20 | 1 | 100 = 117
  *   assert_eq!(*everything, 117);
  * }
  * ```
  *
  * You can see that having one unified number instead of multiple elements in an array can be a more
  * efficient way to represent flags.
  *
  * This is used for interopability with `libc` libraries ([`nix`]).
  *
  * # Limitations
  *
  * This macro does not implement parsing each flag individually after a `Flags` value has been
  * constructed. This is because the types defined by this macro are used for `libc` libraries, where
  * they are provided as arguments, so `libc` does the interpreting for us.
  */
#[macro_export(local_inner_macros)]
macro_rules! c_style_flags
{
  {
    $(#[$attr: meta])*
    $vis: vis enum $id: ident: $int_ty: ty
    {
      $(
        $(#[$v_attr: meta])*
        $variant: ident = $v_int: expr
      ),*
    }

    $(#[$flags_attr: meta])*
    $flags_vis: vis struct $flags_id: ident $(([$_id: ty] as $_int_ty: ty))?;
  } => {
    $(#[$attr])*
    #[repr($int_ty)]
    $vis enum $id
    {
      $(
        $(#[$v_attr])*
        $variant = $v_int
      ),*
    }

    $(#[$flags_attr])*
    // Auto-generate documentation header, this will come last if additional documentation is given
    #[doc = std::concat!("\n\nConcatenated flags of [`", std::stringify!($id), "`]")]
    #[doc = std::concat!("\n\n**Generated by the [`c_style_flags!`] macro**")]
    // Transparency allows transmuting to integer type
    #[repr(transparent)]
    $flags_vis struct $flags_id ($int_ty);

    impl $flags_id
    {
      pub fn push(&mut self, flag: $id)
      {
        self.0 |= flag as $int_ty;
      }
    }
  };
}

/**
  * Create a wrapped value - a struct with a single-object tuple, that can be dereferenced to the object
  * (with additional optional `DerefMut` impl)
  *
  * # Example
  *
  * ```
  * pub struct Username(BoxedStr);
  *
  * wrap! {
  *   impl (Deref<Target = BoxedStr>, DerefMut) for Username;
  * }
  * ```
  */
#[macro_export(local_inner_macros)]
macro_rules! wrap
{
  // Implement standard immutable dereferencing
  {
    $(
      impl $(<$($generic: ident $(: $($imp: path),*)?),*>)? Deref<Target = $inner: ty> for $name: ident;
    )+
  } => {
    $(
      impl $(<$($generic $(:$($imp),+)?),+>)? std::ops::Deref for $name $(<$($generic),+>)?
      {
        type Target = $inner;

        // The compiler will automatically handle proper dereferencing after this
        fn deref(&self) -> &$inner { &self.0 }
      }
    )+
  };
  // Implement for mutable dereferencing
  {
    $(
      impl $(<$($generic: ident $(: $($imp: path),*)?),*>)? (Deref<Target = $inner: ty>, DerefMut) for $name: ident;
    )+
  } => {
    $(
      wrap! { impl $(<$($generic $(:$($imp),*)?),*>)? Deref<Target = $inner> for $name; }

      // and then we implement derefencing as mutable too!
      impl $(<$($generic $(:$($imp),+)?),+>)? std::ops::DerefMut for $name $(<$($generic),+>)?
      {
        fn deref_mut(&mut self) -> &mut $inner { &mut self.0 }
      }
    )+
  };
}
/// Simplified methods for accessing & initializing [`OnceLock`] values
/**
  * # Example
  *
  * ```
  * # use kickit::{oncelock, init::console::prelude::*};
  * use nix::unistd::getuid;
  *
  * oncelock! {
  *   static USER_ID: OnceLock<u32>;
  * }
  *
  * fn main()
  * {
  *   // Initialize the value
  *   oncelock! { USER_ID = getuid() }.handle();
  *   // Obtain reference to the newly initialized value
  *   eprintln!("your user's ID is: {}", oncelock!(&USER_ID).unwrap());
  *
  *   // ... and USER_ID can be used globally afterwards
  * }
  * ```
  *
  * [`OnceLock`]: std::sync::OnceLock
  */
#[macro_export]
macro_rules! oncelock
{
  // Define a new uninitialized OnceLock
  {
    $(
      $(#[$attr: meta])*
      $vis: vis static $name: ident: OnceLock<$ty: ty>;
    )+
  } => {
    $(
      $(#[$attr])*
      $vis static $name: std::sync::OnceLock<$ty> = std::sync::OnceLock::new();
    )+
  };
  // Set a OnceLock's value
  { $id: ident = $val: expr } => // -> Result<()>
  {
    $id.set($val).map_err(|_| Error::Unknown.trace("value has already been initialized").subject(stringify!($id)))
  };
  // Obtain a reference to the OnceLock's value, if initialized, otherwise fallback to provided value
  (&$id: ident .unwrap_or($fallback: expr)) => // -> &T
  {
    $id.get().unwrap_or(&$fallback)
  };
  // Obtain a mutable reference to the OnceLock's value, if initialized, otherwise fallback to provided value
  (&mut $id: ident .unwrap_or($fallback: expr)) => // -> &mut T
  {
    $id.get_mut().unwrap_or(&mut $fallback)
  };
  // Try to obtain a reference to OnceLock's value, which may or may not be initialized
  (&$id: expr) => // -> Result<&T>
  {
    $id.get().ok_or(Error::Unknown.trace("value has not been initialized yet")).subject(stringify!($id))
  };
  // Try to obtain a mutable reference to OnceLock's value, which may or may not be initialized
  (&mut $id: expr) => // -> Result<&mut T>
  {
    $id.get_mut().ok_or(Error::Unknown.trace("value has not been initialized yet")).subject(stringify!($id))
  };
}

/*
 * Data construction, manipulation & interpretation macros
 * =======================================================
 */

/// Construct a [`PathBuf`] from its parents
/**
  * # Example
  *
  * ```
  * # use std::fs;
  * # use kickit::path;
  *
  * // Setup the configuration prefix that can be changed
  * const CONFIG_PREFIX: &str = "/etc";
  *
  * let config_path = path!(CONFIG_PREFIX, "my_program.conf");
  * // Concatenate together, so in this case '/etc/my_program.conf'
  * assert!(fs::exists(config_path).is_ok(), "config doesn't exist!");
  * ```
  *
  * [`PathBuf`]: std::path::PathBuf
  */
#[macro_export]
macro_rules! path
{
  ($($t: expr),*) =>
  {{
    let mut out = <std::path::PathBuf>::new();

    $(
      out.push($t);
    )*

    out
  }};
}

/// Construct an enum matched from its variant identifier in string form
#[macro_export]
macro_rules! enum_from_variant_str
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

/// Extract value from heavily layered types after matching multiple patterns
/**
  * # Motivation
  *
  * This condenses `if let`-style conditions which return a singular value into 1 macro call, the aim is
  * to reduce boilerplate & help maintain code readability.
  *
  * # Example
  *
  * ```ignore (code service snippet, missing external objects)
  * // Due to code complexity, resolving to the code integer requires a long `if let` statement
  * if let ServiceBody::RunOnce { code, .. } = &service.body && let Some(status) = code.get() &&
  *     let Some(code_int) = status.code()
  * {
  *   Some(code_int)
  * }
  * else {
  *   None
  * }
  *
  * // ...which can be collapsed into this pattern tree
  * pat_layer_tree!(&service.body => ServiceBody::RunOnce { code } => |Some| OnceLock::get => |Some| ExitStatus::code)
  * ```
  *
  * # Syntax
  *
  * Multiple possible patterns are supported, represented in a custom-way (while attempting to be as close
  * to Rust's native syntax, however).
  *
  * ## Struct & enum tuple
  *
  * Exact same pattern used in `if let` statements.
  *
  * ```
  * # use kickit::pat_layer_tree;
  * let lucky_number: Result<Option<u8>, ()> = Ok(Some(7));
  *
  * // Extract `7` from Ok & Some variant. If of any other variant, `None` is returned.
  * assert_eq!(pat_layer_tree!(lucky_number => Ok => Some), Some(7));
  * ```
  *
  * ## Struct & enum field
  *
  * Exact same pattern used in `if let` statements.
  *
  * ```
  * # use kickit::pat_layer_tree;
  * struct FavoriteDrink
  * {
  *   liquor: bool,
  *   name: &'static str
  * }
  *
  * let russian = Some(FavoriteDrink { liquor: true, name: "vodka" });
  * let brit = Some(FavoriteDrink { liquor: false, name: "cider" });
  *
  * assert_eq!(pat_layer_tree!(russian => Some => FavoriteDrink { name }), Some("vodka"));
  * assert_eq!(pat_layer_tree!(brit => Some => FavoriteDrink { name }), Some("cider"));
  * ```
  *
  * ## Closure
  *
  * ```
  * # use kickit::pat_layer_tree;
  * # use std::sync::OnceLock;
  *
  * #[derive(Debug)]
  * struct Process
  * {
  *   id: u32,
  *   name: &'static str
  * }
  * 
  * static PROCESS: OnceLock<Process> = OnceLock::new();
  *
  * // value not yet initialized
  * assert!(pat_layer_tree!(&PROCESS => |Some| OnceLock::get => Process { id }).is_none());
  *
  * PROCESS.set(Process { id: std::process::id(), name: "closure_test" }).unwrap();
  * // now the value has been set!
  * eprintln!("pid: {}", pat_layer_tree!(&PROCESS => |Some| OnceLock::get => Process { id }).unwrap());
  * ```
  *
  * # Implementation
  *
  * This macro is an [incremental TT muncher]. This is so that we can differentiate between the last
  * ("tail") item and the starting ("head") items one at a time, which also allows us to parse different
  * patterns in 1 macro call instead of being limited to one pattern for each call.
  *
  * ## Expansion
  *
  * Input:
  *
  * ```
  * # use kickit::pat_layer_tree;
  * let happy: Result<Option<bool>, ()> = Ok(Some(true));
  *
  * let _ = pat_layer_tree!(happy => Ok => Some);
  * ```
  *
  * Output:
  *
  * ```
  * let happy: Result<Option<bool>, ()> = Ok(Some(true));
  *
  * let _ = {
  *   if let Ok(__inner) = happy
  *   {
  *     if let Some(__final) = __inner
  *     {
  *       Some(__final)
  *     }
  *     else {
  *       None
  *     }
  *   }
  *   else {
  *     None
  *   }
  * };
  * ```
  *
  * **Note:** value names `__inner` and `__final` are used here for clarity, different names are used for
  * assigning in the macro implementation
  *
  * [incremental TT muncher]: <https://lukaswirth.dev/tlborm/decl-macros/patterns/tt-muncher.html>
  */
#[macro_export(local_inner_macros)]
macro_rules! pat_layer_tree
{
  /*
   * Major pattern which covers all possible inputs, which are then parsed by the following expansion
   * patterns below this one.
   *
   * `$any` can be matched to tuple for enum variant & single-object tuple structs (e.g. `Some`,
   * `<mod::EnumVariant>::Foo`) or function name for closure (e.g. `OnceLock::get`).
   */
  {$parent: expr => $(|$object: tt|)? $any: path $({$field: ident})? => $($tail: tt)*} =>
  {{
    // Assign inner value as we accept `$parent` as an expr for more advanced inputs (like Fn calls)
    let __pat_layer_tree_cur = $parent;

    if let Some(__pat_layer_tree_next) = pat_layer_tree!(__pat_layer_tree_cur => $(|$object|)? $any $({$field})?)
    {
      // Handle the next branches of the tree
      pat_layer_tree!(__pat_layer_tree_next => $($tail)*)
    }
    else {
      None
    }
  }};

  // Inspect the output of calling `$fn_once` on `$parent` & then match it to an enum variant or struct tuple
  ($parent: expr => |$object: path| $fn_once: expr) =>
  {
    pat_layer_tree!(@let __branch, $object (__branch) = $fn_once ($parent))
  };
  // Inspect the output of calling `$fn_once` on `$parent` & then match it to an enum variant or struct's field
  ($parent: expr => |$object: path { $field: ident }| $fn_once: expr) =>
  {
    pat_layer_tree!(@let __branch, $object { $field: __branch, .. } = $fn_once ($parent))
  };
  // Match `$parent` to an enum variant or struct's field
  ($parent: expr => $object: path { $field: ident }) =>
  {
    pat_layer_tree!(@let __branch, $object { $field: __branch, .. } = $parent)
  };
  // Match `$parent` to an enum variant or a tuple struct
  ($parent: expr => $object: path) =>
  {
    pat_layer_tree!(@let __branch, $object (__branch) = $parent)
  };

  (@let $__branch: ident, $($t: tt)*) =>
  {{
    if let $($t)*
    {
      Some($__branch)
    }
    else {
      None
    }
  }};
}

/// A C-style ternary expression with different syntax for Rust macro fragments (? can't come after expr)
#[macro_export]
macro_rules! tern
{
  // $eval ? $ret : $default
  { $eval: expr => $ret: expr, _ => $default: expr } =>
  {
    if ($eval)
    {
      $ret
    }
    else {
      $default
    }
  };
}

/*
 * Collapsed function call macros
 * ==============================
 */

/// Get the name of the current binary (wrapper for [`std::env::current_exe`])
#[macro_export]
macro_rules! binary
{
  () => {
    {
      // Get the filename of the executable (argv0)
      std::env::current_exe()
        .unwrap_or(env!("CARGO_PKG_NAME").into())
        .file_name()
        .unwrap_or(<std::ffi::OsStr>::new(env!("CARGO_PKG_NAME")))
        .display()
        .to_string()
    }
  };
}
