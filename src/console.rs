//! General implementation for logging, errors and status updates

use std::{fmt, fmt::Display, ops::Deref};

/*
 * Implementation for throwing an error with a trace (ErrorTrace) or
 * without a trace (Error)
 */
pub trait ReturnError
{
  // The ! return type shows that fatal() should never end up returning
  fn fatal(self) -> !;
  // For non-fatal errors only
  fn warn(self);
}

pub trait HandleError: Sized
{
  type OkType;
  type ErrorType: ReturnError;

  // handle() functions like unwrap(): Return contents if OK or fatal if not
  fn handle(self) -> Self::OkType;

  // Warn on error, if an error occurs `None` is returned, if not then `Some` instead
  fn or_warn(self) -> Option<Self::OkType>;
}

/*
 * Addon more context to an error for diagnostics, this is usually the name of something,
 * i.e., if an error occurs with a specific service we add the service's name as context
 */
pub trait ExtendWithContext<OkType, ErrType>
{
  /**
    * Add context to an existing trace error
    *
    * # Errors
    * - Result is of error variant
    */
  fn context(self, context: impl Display) -> Result<OkType, ErrType>;
}

/*
 * Convert a non-standard error input into an `ErrorTrace`. This is usually done on a
 * `Result<OkType, ...>` but can also be done on a `Option<...>` where we assume the
 * option's `Some` variant contains the error
 */
pub trait ErrorResult<ErrKind: ReturnError, ErrOutput: ReturnError, OkType, ErrType: Display>
{
  /**
    * Convert an error to a trace without context
    *
    * # Errors
    * - Data type contains an error (e.g. Result is Err(..))
    */
  fn into_trace(self, kind: ErrKind) -> Result<OkType, ErrOutput>;
}

pub trait Colourize: Display + Sized
{
  /*
   * Create a new coloured instance from an existing displayable type, this is one of
   * two ways you can go:
   *
   * `FmtString::from("hello world!").bold()` -> Creates a non-coloured instance, then
   *   colours it (inefficient)
   *
   * or
   * 
   * `"hello world!".bold()` -> Creates a new coloured instance (better)
   */
  fn colour(self, colour: Colour) -> FmtString
  {
    FmtString { colours: vec![colour], inner: self.to_string(), dont_reset: false }
  }

  fn bold(self) -> FmtString
  {
    self.colour(Colour::Bold)
  }
}

#[derive(Copy, Clone, Debug, derive_more::Display)]
pub enum Colour
{
  #[display("\x1b[0m")]
  Reset,
  #[display("\x1b[1m")]
  Bold,
  #[display("\x1b[1;31m")]
  Red,
  #[display("\x1b[1;33m")]
  Orange,
  #[display("\x1b[1;92m")]
  Green
}

#[derive(Debug, Default)]
pub struct FmtStr<'inner>
{
  colours: &'inner [Colour],
  inner: &'inner str,
  dont_reset: bool
}

#[derive(Clone, Debug, Default)]
pub struct FmtString
{
  colours: Vec<Colour>,
  inner: String,
  dont_reset: bool
}

impl Display for FmtStr<'_>
{
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result
  {
    write!(f, "{}", self.resolve())
  }
}

impl Display for FmtString
{
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result
  {
    write!(f, "{}", self.as_ref().resolve())
  }
}

impl FmtString
{
  #[must_use]
  pub const fn new() -> Self
  {
    Self { colours: Vec::new(), inner: String::new(), dont_reset: false }
  }

  // This will create a new instance which can be inefficient, try `.push_colour()` on an existing instance
  #[must_use]
  pub fn colour(mut self, colour: Colour) -> Self
  {
    self.push_colour(colour);
    self
  }

  #[must_use]
  pub fn bold(self) -> Self
  {
    self.colour(Colour::Bold)
  }

  // Reference to an existing instance
  #[must_use]
  pub const fn as_ref(&self) -> FmtStr<'_>
  {
    FmtStr { colours: self.colours.as_slice(), inner: self.inner.as_str(), dont_reset: self.dont_reset }
  }

  // Change whether the end of the formatted String will have a reset (\e[0m)
  #[must_use]
  pub fn reset(self, reset: bool) -> Self
  {
    FmtString { colours: self.colours, inner: self.inner, dont_reset: !reset }
  }

  pub fn push_str(&mut self, string: &str)
  {
    self.inner.push_str(string);
  }

  pub fn push_colour(&mut self, colour: Colour)
  {
    self.colours.push(colour);
  }

  // INFO: this does not clear the colours!
  pub fn clear(&mut self)
  {
    self.inner.clear();
  }
}

impl FmtStr<'static>
{
  #[must_use]
  pub const fn new() -> Self
  {
    Self { colours: &[], inner: "", dont_reset: false }
  }
}

impl FmtStr<'_>
{
  // Copy into a new owned formatted string instance
  #[must_use]
  pub fn to_owned(self) -> FmtString
  {
    FmtString { colours: self.colours.to_vec(), inner: self.inner.to_owned(), dont_reset: self.dont_reset }
  }

  pub fn resolve(&self) -> String
  {
    // Transform all colours into their string data
    let mut out: Vec<String> = self.colours.iter().map(ToString::to_string).collect();
    // The main actual content
    out.push(self.inner.to_string());

    // This is going to be false 99% of the time except for some niche cases (change with `.reset(..)`)
    if (!self.dont_reset)
    {
      out.push(Colour::Reset.to_string());
    }

    // Concatenate all output together into a singular string
    out.join("")
  }
}

// Create a no-colour formatted string
impl<'inner> From<&'inner str> for FmtStr<'inner>
{
  fn from(inner: &'inner str) -> Self
  {
    Self { colours: &[], inner, dont_reset: false }
  }
}

impl From<String> for FmtString
{
  fn from(inner: String) -> Self
  {
    Self { colours: Vec::new(), inner, dont_reset: false }
  }
}

// Get just the inner string, no colours
impl Deref for FmtStr<'_>
{
  type Target = str;

  fn deref(&self) -> &str
  {
    self.inner
  }
}

impl Deref for FmtString
{
  type Target = String;

  fn deref(&self) -> &String
  {
    &self.inner
  }
}

/*
 * What to do when a Error is found
 * Implemented for Result<anything, error> and Option<error>
 */
impl<S, F: ReturnError> HandleError for Result<S, F>
{
  type OkType = S;
  type ErrorType = F;

  fn handle(self) -> Self::OkType
  {
    match (self)
    {
      Ok(ok) => ok,
      Err(error) => error.fatal()
    }
  }
  fn or_warn(self) -> Option<Self::OkType>
  {
    match (self)
    {
      Ok(ok) => Some(ok),
      Err(error) => { error.warn(); None }
    }
  }
}

// Assume here that the Option carries an error
impl<F: ReturnError> HandleError for Option<F>
{
  type OkType = ();
  type ErrorType = F;

  fn handle(self) -> Self::OkType
  {
    if let Some(error) = self
    {
      error.fatal()
    }
  }
  fn or_warn(self) -> Option<Self::OkType>
  {
    if let Some(error) = self
    {
      error.warn();
    }
    None
  }
}

impl<Displayable: Display + Sized> Colourize for Displayable {}

// An error type, this is just a lot of boilerplate so we can simplify this with a macro
#[macro_export]
macro_rules! error
{
  {
    $(#[$attr: meta])*
    $vis: vis enum Error $(<$($generic: ident $(: $($depend: ty),*)?),*>)?
    {
      $(
        $(#[$variantAttr: meta])*
        $variant: ident $(<$($variantGeneric: ident $(: $($variantDepend: ty),*)?),*>)?
      ),*
    }
  } => {
    pub use std::result::Result as StdResult;
    // Just a Result where the error type is of ErrorTrace
    pub type Result<S> = StdResult<S, ErrorTrace>;

    $(#[$attr])*
    $vis enum Error $(<$($generic $(: $($depend),*)?),*>)?
    {
      $(
        $(#[$variantAttr])*
        $variant $(<$($variantGeneric $(: $($variantDepend),*)?),*>)?
      ),*
    }

    // This stores an error (in kind) and an error trace/"context" (in trace)
    #[derive(PartialEq, Eq, Clone, Debug)]
    #[must_use]
    $vis struct ErrorTrace
    {
      pub kind: Error,
      pub context: Option<String>,
      pub trace: String
    }

    impl<OkType, ErrType: Display> ErrorResult<Error, ErrorTrace, OkType, ErrType> for StdResult<OkType, ErrType>
    {
      fn into_trace(self, kind: Error) -> Result<OkType>
      {
        self.map_err(|trace| kind.trace(&trace))
      }
    }

    impl<ErrType: Display> ErrorResult<Error, ErrorTrace, (), ErrType> for Option<ErrType>
    {
      fn into_trace(self, kind: Error) -> Result<()>
      {
        if let Some(err) = self.map(|trace| kind.trace(&trace))
        {
          Err(err)
        }
        else {
          Ok(())
        }
      }
    }

    impl ErrorResult<Error, ErrorTrace, (), String> for String
    {
      fn into_trace(self, kind: Error) -> Result<()>
      {
        Err(kind.trace(&self))
      }
    }

    impl ErrorResult<Error, ErrorTrace, (), std::io::Error> for std::io::Error
    {
      fn into_trace(self, kind: Error) -> Result<()>
      {
        Err(kind.trace(&self))
      }
    }

    // Strip a traceful error down to a traceless error
    impl From<ErrorTrace> for Error
    {
      fn from(trace: ErrorTrace) -> Self
      {
        trace.kind
      }
    }

    // ...and vice versa
    impl From<Error> for ErrorTrace
    {
      fn from(kind: Error) -> Self
      {
        kind.trace("")
      }
    }

    impl Error
    {
      pub fn trace(self, trace: impl Display) -> ErrorTrace
      {
        ErrorTrace { kind: self, context: None, trace: trace.to_string() }
      }
    }

    impl ErrorTrace
    {
      pub fn context(self, context: impl Display) -> Self
      {
        Self { kind: self.kind, context: Some(context.to_string()), trace: self.trace }
      }
    }

    // If this `Result` is of `Err` variant, add some context to it
    impl<OkType> ExtendWithContext<OkType, ErrorTrace> for Result<OkType>
    {
      fn context(self, context: impl Display) -> Result<OkType>
      {
        match (self)
        {
          Ok(ok) => Ok(ok),
          Err(err) => Err(ErrorTrace { kind: err.kind, context: Some(context.to_string()), trace: err.trace })
        }
      }
    }
  };
}
pub use crate::error as error;

#[macro_export]
macro_rules! guard
{
  ($eval: expr => $error: expr) =>
  {
    if ($eval)
    {
      return Err($error)
    }
  }
}
/*
 * Re-export our macro, so it matches our current module path
 * (i.e. it exports to crate::console::guard as well as crate::guard)
 * (why doesn't rust do this automatically already???)
 */
pub use crate::guard as guard;
