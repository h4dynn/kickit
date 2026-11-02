//! Error handling, warning & status implementations for ktctl

use std::fmt::Display;
use crate::console::{error_ty, Colour, Colourize, ReturnError, ErrorResult, ExtendWithSubject};

pub mod prelude
{
  pub use crate::console::prelude::*;
  pub use super::{StdResult, Result, Error, ErrorTrace, warn};
}

error_ty! {
  #[derive(PartialEq, Eq, Clone, Debug, Default, thiserror::Error)]
  #[must_use]
  pub enum Error
  {
    #[error("An unknown error occurred")]
    #[default]
    Unknown,

    #[error("Operation not permitted")]
    OperationNotPermitted,
    #[error("Too much arguments provided for operation")]
    TooManyArgs,
    #[error("Error occurred whilst resolving current time")]
    Time,
    #[error("Missing argument for operation")]
    MissingArgument,
    #[error("Unrecognised operation provided")]
    InvalidOperation,
    #[error("Invalid arguments provided")]
    InvalidArguments,
    #[error("kickit is not running")]
    InitNotRunning,
    #[error("Failed to access kickit work directory")]
    AccessRunFsFail,
    #[error("Failed to access kickit resources")]
    AccessResource,
    #[error("Service not found")]
    BadService,
    #[error("Service configuration is corrupted")]
    ServiceConfig,
    #[error("Invalid non-UTF-8 file encoding")]
    Format,
    #[error("Failed to access log file from service")]
    LogAccessFail,
    #[error("Failed to parse init's working data files")]
    RunFsParseFail,
    #[error("Failed to access a socket")]
    SocketAccessFail,
    #[error("Socket gave invalid response")]
    SocketResponse,
    #[error("Error when sending request to socket")]
    Socket
  }
}

macro_rules! inner_fatal
{
  (@traceless $message: expr) =>
  {{
    eprintln!("{} {} {}", binary!(), "(error):".colour(Colour::Red), $message.bold());
    std::process::exit(1);
  }};
  (@trace $message: expr, $subject: expr, $trace: expr) =>
  {{
    // If no subject is available, we will just use an empty string
    let addon = $subject.map(|s| format!(": {s}")).unwrap_or_default();
    // Same as above but for a trace
    let trace = (!$trace.is_empty()).then_some(format!(": {}", $trace)).unwrap_or_default();

    eprintln!("{} {} {}{}{}", binary!(), "(error):".colour(Colour::Red), $message.bold(), trace, addon);
    std::process::exit(1);
  }};
}

impl ReturnError for Error
{
  fn fatal(self) -> !
  {
    inner_fatal!(@traceless self.to_string());
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
    inner_fatal!(@trace self.kind.to_string(), self.subject, self.trace.trim_end_matches('\n'));
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
