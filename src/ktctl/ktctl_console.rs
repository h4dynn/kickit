//! Error handling, warning & status implementations for ktctl

use thiserror::Error;
use std::fmt::Display;
use crate::{binary, warn, console::{Colour, ReturnError}};

#[derive(PartialEq, Eq, Clone, Error, Debug)]
#[must_use]
pub enum KTCtlError
{
  #[error("Permission denied: Root is required for this operation")] BadPerms,
  #[error("Too much arguments provided for operation")] TooManyArgs,
  #[error("Missing argument for operation")] MissingArgument,
  #[error("Unrecognised operation provided")] InvalidOperation,
  #[error("Invalid arguments provided")] InvalidArguments,
  #[error("kickit is not running")] InitNotRunning,
  #[error("Failed to access kickit work directory")] AccessRunFsFail,
  #[error("An unrecognised service was provided")] BadService,
  #[error("Invalid file encoding (expected UTF-8)")] FormatFail,
  #[error("Failed to access log file from service")] LogAccessFail,
  #[error("Failed to parse init work data")] RunFsParseFail,
  #[error("Failed to access a socket")] SocketAccessFail
}

#[derive(PartialEq, Eq, Clone, Debug)]
#[must_use]
pub struct KTCtlErrorTrace { kind: KTCtlError, context: Option<String>, trace: String }

type KTCtlResult<S> = Result<S, KTCtlErrorTrace>;

pub trait ConvKTCtlError: Sized
{
  type OK;

  ///
  /// # Errors
  /// * Data type contains an error (e.g. Result is of Err(e) variant)
  ///
  fn trace(self, errorKind: KTCtlError) -> KTCtlResult<Self::OK>
  {
    Err(KTCtlErrorTrace::new(errorKind, ""))
  }
  ///
  /// # Errors
  /// * Data type contains an error (e.g. Result is of Err(e) variant)
  ///
  fn context_trace(self, context: impl Display, errorKind: KTCtlError) -> KTCtlResult<Self::OK>
  {
    Err(KTCtlErrorTrace::with_context(errorKind, context, ""))
  }
}

macro_rules! innerFatal
{
  ($message: expr) =>
  {
    use std::process;

    eprintln!("{} {}(ERROR):{} {}{}", binary!(), Colour::RED, Colour::BOLD, $message,
                                      Colour::RESET);
    process::exit(1);
  };

  ($message: expr, $context: expr, $trace: expr) =>
  {
    {
      use std::process;

      // If no context is available, we will just use an empty string
      let addon = if let Some(_addon) = $context { format!(": {_addon}") } else { String::new() };

      // Same as above but for a trace
      let trace = if (!$trace.is_empty()) { format!(": {}", $trace) } else { String::new() };

      eprintln!("{} {}(ERROR):{} {}{}{trace}{addon}", binary!(), Colour::RED, Colour::BOLD,
                                                      $message, Colour::RESET);
      process::exit(1);
    }
  };
}

impl ReturnError for KTCtlError
{
  fn fatal(self) -> ! { innerFatal!(self.to_string()); }

  fn warn(self) { warn!("{}", self.to_string()); }
}

impl ReturnError for KTCtlErrorTrace
{
  fn fatal(self) -> !
  {
    innerFatal!(self.kind.to_string(), self.context, self.trace.trim_end_matches('\n'));
  }

  fn warn(self) { warn!("{}: {}", self.kind.to_string(), self.trace.trim_end_matches('\n')); }
}

impl KTCtlErrorTrace
{
  pub fn new(kind: KTCtlError, trace: impl ToString) -> Self
  {
    Self { kind, context: None, trace: trace.to_string() }
  }

  pub fn with_context(kind: KTCtlError, context: impl ToString, trace: impl ToString) -> Self
  {
    Self { kind, context: Some(context.to_string()), trace: trace.to_string() }
  }
}

impl<S, F: std::fmt::Display> ConvKTCtlError for Result<S, F>
{
  type OK = S;

  fn trace(self, errorKind: KTCtlError) -> KTCtlResult<Self::OK>
  {
    match (self)
    {
      Err(e) => Err(KTCtlErrorTrace::new(errorKind, e)),
      Ok(c)  => Ok(c)
    }
  }

  fn context_trace(self, context: impl ToString, errorKind: KTCtlError)
    -> KTCtlResult<Self::OK>
  {
    match (self)
    {
      Err(e) => Err(KTCtlErrorTrace::with_context(errorKind, context, e)),
      Ok(c)  => Ok(c)
    }
  }
}

impl<S: std::fmt::Display> ConvKTCtlError for Option<S>
{
  type OK = S;

  fn trace(self, errorKind: KTCtlError) -> KTCtlResult<Self::OK>
  {
    if let Some(contents) = self { return Ok(contents) }
    Err(KTCtlErrorTrace::new(errorKind, ""))
  }

  fn context_trace(self, context: impl ToString, errorKind: KTCtlError)
    -> KTCtlResult<Self::OK>
  {
    if let Some(contents) = self { return Ok(contents) }
    Err(KTCtlErrorTrace::with_context(errorKind, context, ""))
  }
}

impl ConvKTCtlError for String
{
  type OK = ();
  // Will always return an error since there is no way to check if a string is an error or not
  fn trace(self, errorKind: KTCtlError) -> KTCtlResult<()>
  {
    Err(KTCtlErrorTrace::new(errorKind, self))
  }
}
