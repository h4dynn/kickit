//! Error handling, warning & status implementations for ktctl

use thiserror::Error;
use std::fmt::Display;
use crate::{binary, console::{Colour, ReturnError, ExtendWithContext}};

#[derive(PartialEq, Eq, Clone, Error, Debug)]
#[must_use]
pub enum Error
{
  #[error("Operation not permitted")] OperationNotPermitted,
  #[error("Too much arguments provided for operation")] TooManyArgs,
  #[error("Missing argument for operation")] MissingArgument,
  #[error("Unrecognised operation provided")] InvalidOperation,
  #[error("Invalid arguments provided")] InvalidArguments,
  #[error("kickit is not running")] InitNotRunning,
  #[error("Failed to access kickit work directory")] AccessRunFsFail,
  #[error("Failed to access kickit resources")] AccessResource,
  #[error("An unrecognised service was provided")] BadService,
  #[error("Invalid file encoding (expected UTF-8)")] Format,
  #[error("Failed to access log file from service")] LogAccessFail,
  #[error("Failed to parse init work data")] RunFsParseFail,
  #[error("Failed to access a socket")] SocketAccessFail,
  #[error("Socket gave invalid response")] SocketResponse,
  #[error("Error when sending request to socket")] Socket
}

#[derive(PartialEq, Eq, Clone, Debug)]
#[must_use]
pub struct ErrorTrace
{
  kind: Error,
  context: Option<String>,
  trace: String
}

pub type Result<S> = StdResult<S, ErrorTrace>;
pub use std::result::Result as StdResult;

pub trait ConvError<OkType, ErrorType>
{
  /**
    * # Errors
    * * Data type contains an error (e.g. Result is of Err(e) variant)
    */
  fn into_trace(self, kind: Error) -> Result<OkType>;
}

macro_rules! innerFatal
{
  (@traceless $message: expr) =>
  {
    use std::process;

    eprintln!("{} {}(ERROR):{} {}{}", binary!(), Colour::RED, Colour::BOLD, $message,
                                      Colour::RESET);
    process::exit(1);
  };

  (@trace $message: expr, $context: expr, $trace: expr) =>
  {
    {
      use std::process;

      // If no context is available, we will just use an empty string
      let addon =
      {
        if let Some(inner) = $context
        {
          format!(": {inner}")
        }
        else {
          String::new()
        }
      };

      // Same as above but for a trace
      let trace =
      {
        if (!$trace.is_empty())
        {
          format!(": {}", $trace)
        }
        else {
          String::new()
        }
      };

      eprintln!("{} {}(ERROR):{} {}{}{trace}{addon}", binary!(), Colour::RED, Colour::BOLD,
                                                      $message, Colour::RESET);

      process::exit(1);
    }
  };
}

impl ReturnError for Error
{
  fn fatal(self) -> !
  {
    innerFatal!(@traceless self.to_string());
  }
  fn warn(self)
  {
    warn!("{}", self.to_string());
  }
}

impl ReturnError for ErrorTrace
{
  fn fatal(self) -> !
  {
    innerFatal!(@trace self.kind.to_string(), self.context, self.trace.trim_end_matches('\n'));
  }

  fn warn(self)
  {
    warn!("{}: {}", self.kind.to_string(), self.trace.trim_end_matches('\n'));
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

impl<OkType> ExtendWithContext<OkType, ErrorTrace> for Result<OkType>
{
  fn context(self, context: impl Display) -> Result<OkType>
  {
    match (self)
    {
      Ok(ok) => Ok(ok),
      Err(error) => Err(
      {
        ErrorTrace {
          kind: error.kind,
          context: Some(context.to_string()),
          trace: error.trace
        }
      })
    }
  }
}

impl<OkType, ErrorType: Display> ConvError<OkType, ErrorType> for StdResult<OkType, ErrorType>
{
  fn into_trace(self, kind: Error) -> Result<OkType>
  {
    match (self)
    {
      Err(error) => Err(kind.trace(&error)),
      Ok(ok) => Ok(ok)
    }
  }
}

impl<OkType: Display> ConvError<OkType, ()> for Option<OkType>
{
  fn into_trace(self, kind: Error) -> Result<OkType>
  {
    if let Some(ok) = self
    {
      Ok(ok)
    }
    else {
      Err(kind.trace("No value is available!"))
    }
  }
}

impl ConvError<(), String> for String
{
  // Will always return an error since there is no way to check if a string is an error or not
  fn into_trace(self, kind: Error) -> Result<()>
  {
    Err(kind.trace(&self))
  }
}

#[macro_export]
macro_rules! ktctl_warn
{
  ($($message: tt)*) =>
  {
    {
      eprintln!("{} {}(WARNING):{} {}{}", binary!(), Colour::ORANGE, Colour::BOLD,
                          format!($($message)*), Colour::RESET);
    }
  };
}
pub use crate::ktctl_warn as warn;
