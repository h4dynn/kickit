//! General implementation for logging, errors and status updates

use std::fmt::Display;

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
pub trait ExtendWithContext<OkType, ErrorType>
{
  /**
    * Add context to an existing trace error
    *
    * # Errors
    * - Result is of error variant
    */
  fn context(self, context: impl Display) -> Result<OkType, ErrorType>;
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
  fn into_trace(self, errorKind: ErrKind) -> Result<OkType, ErrOutput>;
}

#[derive(derive_more::Display)]
pub enum Colour
{
  #[display("\x1b[0m")]
  RESET,
  #[display("\x1b[0;1m")]
  BOLD,
  #[display("\x1b[0;1;31m")]
  RED,
  #[display("\x1b[0;1;33m")]
  ORANGE,
  #[display("\x1b[0;1;92m")]
  GREEN
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
      Err(error) =>
      {
        error.warn();
        None
      }
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
  } =>
  {
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
      fn into_trace(self, errorKind: Error) -> Result<OkType>
      {
        self.map_err(|trace| errorKind.trace(&trace))
      }
    }

    impl<ErrType: Display> ErrorResult<Error, ErrorTrace, (), ErrType> for Option<ErrType>
    {
      fn into_trace(self, errorKind: Error) -> Result<()>
      {
        if let Some(err) = self.map(|trace| errorKind.trace(&trace))
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
      fn into_trace(self, errorKind: Error) -> Result<()>
      {
        Err(errorKind.trace(&self))
      }
    }

    impl ErrorResult<Error, ErrorTrace, (), std::io::Error> for std::io::Error
    {
      fn into_trace(self, errorKind: Error) -> Result<()>
      {
        Err(errorKind.trace(&self))
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
