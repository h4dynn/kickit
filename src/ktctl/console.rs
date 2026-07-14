//! Error handling, warning & status implementations for ktctl

use std::fmt::Display;
use crate::{error, binary, console::{Colour, Colourize, ReturnError, ErrorResult, ExtendWithContext}};

error! {
  #[derive(PartialEq, Eq, Clone, Debug, Default, thiserror::Error)]
  #[must_use]
  pub enum Error
  {
    #[error("An unknown error occurred")] #[default] Unknown,
    #[error("Operation not permitted")] OperationNotPermitted,
    #[error("Too much arguments provided for operation")] TooManyArgs,
    #[error("Error occurred whilst resolving current time")] Time,
    #[error("Missing argument for operation")] MissingArgument,
    #[error("Unrecognised operation provided")] InvalidOperation,
    #[error("Invalid arguments provided")] InvalidArguments,
    #[error("kickit is not running")] InitNotRunning,
    #[error("Failed to access kickit work directory")] AccessRunFsFail,
    #[error("Failed to access kickit resources")] AccessResource,
    #[error("Service not found")] BadService,
    #[error("Service configuration is corrupted")] ServiceConfig,
    #[error("Invalid file encoding (expected UTF-8)")] Format,
    #[error("Failed to access log file from service")] LogAccessFail,
    #[error("Failed to parse init work data")] RunFsParseFail,
    #[error("Failed to access a socket")] SocketAccessFail,
    #[error("Socket gave invalid response")] SocketResponse,
    #[error("Error when sending request to socket")] Socket
  }
}

macro_rules! innerFatal
{
  (@traceless $message: expr) =>
  {
    {
      use std::process;

      eprintln!("{}: {} {}", binary!(), "error:".colour(Colour::Red), $message.colour(Colour::Bold));
      process::exit(1);
    }
  };

  (@trace $message: expr, $context: expr, $trace: expr) =>
  {
    {
      use std::process;

      // If no context is available, we will just use an empty string
      let addon = {
        if let Some(inner) = $context
        {
          format!(": {inner}")
        }
        else {
          String::new()
        }
      };

      // Same as above but for a trace
      let trace = (!$trace.is_empty()).then_some(format!(": {}", $trace)).unwrap_or_default();

      eprintln!("{}: {} {}{}{}", binary!(), "error:".colour(Colour::Red), $message.colour(Colour::Bold), trace, addon);
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

#[macro_export]
macro_rules! ktctl_warn
{
  ($($message: tt)*) =>
  {
    {
      eprintln!("{}: {} {}", binary!(), "warning:".colour(Colour::Orange), format!($($message)*).colour(Colour::Bold));
    }
  };
}
pub use crate::ktctl_warn as warn;
