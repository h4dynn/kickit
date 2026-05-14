//! Error handling, warning & status implementations for the init process

use std::{sync::Mutex, fmt::Display, io};
use thiserror::Error;
use crate::{console::Colour, console::ReturnError, state::InitState, display_enum};

pub enum Marker
{
  Status, Warn, Fatal, Service
}

display_enum!
{
  Marker
  {
    Status => "\x1b[1m[*]\x1b[0m",
    Warn => "\x1b[1;33m[-]\x1b[0m",
    Fatal => "\x1b[1;31m[!]\x1b[0m",
    Service => "\x1b[1;92m[>]\x1b[0m"
  }
}

/*
 * This is where our logs from things like `state!()`, `warn!()`
 * and errors will be stored to (mostly for debugging). Do not
 * modify this manually, use the `log!()` macro instead
 */
#[doc(hidden)]
pub static MASTER_LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub use std::result::Result as StdResult;
// Just a Result where the error type is of ErrorTrace
pub type Result<S> = StdResult<S, ErrorTrace>;

/*
 * Error stores all possible errors that could be thrown
 * Messages and whether the error should send you to an emergency shell
 * or not are defined in the impl
 */
#[derive(PartialEq, Eq, Clone, Copy, Debug, Error, Default)]
#[must_use]
pub enum Error
{
  // A generic error- usually for very specific case errors & impossible situations
  #[default]
  #[error("An unknown error occurred")] Unknown,
  // Errors when starting up the init system
  #[error("kickit is already running!")] AlreadyRunning,
  #[error("kickit must be ran as the init process")] NotInit,
  #[error("Insufficient permissions: kickit can only be ran as root")] NotRoot,
  // File-related errors i.e. can't access or permission denied
  #[error("File or directory not found")] FileNotFound,
  #[error("Failed to setup work directory")] RunFsFail,
  #[error("Kernel command-line parameter not found")] Cmdline,
  // When data can't be represented as a UTF-8 string
  #[error("Failed to format content in UTF-8")] Format,
  // Target-related errors (see how they are used in `src/target.rs`)
  #[error("A required value is missing in target configuration")] TargetMissingValue,
  #[error("Failed to parse target configuration file")] TargetParse,
  // Socket data input/output failure
  #[error("Failed to read from a socket")] Socket,
  // Service-related errors (used in `src/service.rs`)
  #[error("Failed to parse service configuration file")] ServiceParse,
  #[error("Failed to access a service")] ServiceAccess,
  #[error("Failed to start a service")] ServiceUp,
  #[error("Failed to stop a service")] ServiceDown,
  #[error("Failed to start logger for a service")] ServiceLog,
  #[error("Service was killed or stopped")] ServiceNotRunning,
  #[error("Service became a zombie")] ServiceZombified,
  #[error("Failed to access a logfile")] AccessLog,
  // Mount-related failures
  #[error("Failed to mount a critical filesystem")] SysFsMount,
  #[error("Failed to unmount a filesystem")] SysFsUnmount,
  // Pure init errors
  #[error("Failed to shutdown the init system")] Shutdown
}

// This stores an error (in kind) and an error trace/"context" (in trace)
#[derive(PartialEq, Eq, Clone, Debug)]
#[must_use]
pub struct ErrorTrace
{
  kind: Error,
  context: Option<String>,
  trace: String
}

pub trait ErrorResult<OkType, ErrType> where ErrType: Display
{
  /**
    * Convert an error to a trace without context
    *
    * # Errors
    * - Data type contains an error (e.g. Result is not Ok(x) variant)
   **/
  fn trace(self, errorKind: Error) -> Result<OkType>;
  /**
    * Same but with context
    *
    * # Errors
    * - Data type contains an error (e.g. Result is not Ok(x) variant)
   **/
  fn context_trace(self, context: impl Display, errorKind: Error) -> Result<OkType>;
}

// A traceless, unknown error is the default
impl Default for ErrorTrace
{
  fn default() -> Self
  {
    ErrorTrace::new(Error::default(), "")
  }
}

macro_rules! innerFatal
{
  // Implementation for a tracless error, just displays the provided message
  (@traceless $error: tt) =>
  {
    use $crate::init::init_console::Marker::Fatal;

    log!(format!("{}{} {}{}", Fatal, Colour::BOLD, $error, Colour::RESET));
    $error.exit();
  };
  (@trace $error: tt) =>
  {
    use $crate::init::init_console::Marker::Fatal;

    log!(format!("{}{} {}{}{}", Fatal, Colour::BOLD, $error.kind, Colour::RESET,
                  if let Some(ref c) = $error.context { format!(": {c}") } else { String::new() }));

    if (!$error.trace.is_empty())
    {
      log!(format!("{}{} >> {}", Fatal, Colour::RESET, $error.trace.trim_end_matches('\n')));
    }

    $error.kind.exit();
  };
}

/*
 * .fatal() implementations
 * Error will throw just the message with no trace
 * ErrorTrace will provide a trace
 */
impl ReturnError for Error
{
  fn fatal(self) -> !
  {
    innerFatal!(@traceless self);
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
    innerFatal!(@trace self);
  }

  fn warn(self)
  {
    if (self.trace.is_empty())
    {
      warn!("{}", self.kind);
    }
    else {
      warn!("{}: {}", self.kind, self.trace);
    }
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
    Self::new(kind, "")
  }
}

impl Error
{
  fn exit(self) -> !
  {
    use std::process;
    use Error::{AlreadyRunning, NotInit, NotRoot};

    if (!matches!(self, AlreadyRunning | NotInit | NotRoot))
    {
      kickToEmergencyShell();
    }

    process::exit(1);
  }
}

impl ErrorTrace
{
  pub fn new(k: Error, t: &(impl Display + ?Sized)) -> Self
  {
    Self { kind: k, context: None, trace: t.to_string() }
  }

  pub fn with_context(k: Error, c: &(impl Display + ?Sized), t: &(impl Display + ?Sized))
    -> Self
  {
    Self { kind: k, context: Some(c.to_string()), trace: t.to_string() }
  }
}

impl<OkType, ErrType: Display> ErrorResult<OkType, ErrType> for StdResult<OkType, ErrType>
{
  fn trace(self, errorKind: Error) -> Result<OkType>
  {
    self.map_err(|e| ErrorTrace::new(errorKind, &e))
  }

  fn context_trace(self, c: impl Display, k: Error) -> Result<OkType>
  {
    self.map_err(|e| ErrorTrace::with_context(k, &c, &e))
  }
}

impl<ErrType: Display> ErrorResult<(), ErrType> for Option<ErrType>
{
  fn trace(self, k: Error) -> Result<()>
  {
    if let Some(e) = self.map(|why| ErrorTrace::new(k, &why))
    {
      Err(e)
    }
    else {
      Ok(())
    }
  }

  fn context_trace(self, c: impl Display, k: Error) -> Result<()>
  {
    if let Some(e) = self.map(|why| ErrorTrace::with_context(k, &c, &why))
    {
      Err(e)
    }
    else {
      Ok(())
    }
  }
}

impl ErrorResult<(), String> for String
{
  fn trace(self, k: Error) -> Result<()>
  {
    Err(ErrorTrace::new(k, &self))
  }
  fn context_trace(self, c: impl Display, k: Error) -> Result<()>
  {
    Err(ErrorTrace::with_context(k, &c, &self))
  }
}

impl ErrorResult<(), std::io::Error> for io::Error
{
  fn trace(self, k: Error) -> Result<()>
  {
    Err(ErrorTrace::new(k, &self))
  }
  fn context_trace(self, c: impl Display, k: Error) -> Result<()>
  {
    Err(ErrorTrace::with_context(k, &c, &self))
  }
}

fn kickToEmergencyShell()
{
  use std::process::Command;
  use crate::{init::SHELL, state::INIT_STATE};

  // Try to lock the mutex & see if we are in emergency already
  if let Ok(mut state) = INIT_STATE.lock() && (*state != InitState::Emergency)
  {
    warn!("Critical error when starting init, opening emergency shell");

    *state = InitState::Emergency;
    // Allow other parts of the init system to access the state
    drop(state);

    // Open shell in interactive mode
    Command::new("/usr/bin/env")
      .args(["-S", "PS1=\'\\[\\e[1m\\](emergency)\\[\\e[0m\\] \\w # \'", SHELL, "-himBHs"])
      .spawn()
      .expect("Failed to open an emergency shell!")
      .wait()
      .unwrap();

    // Don't exit here otherwise we will get a kernel panic
    warn!("Hanging on error, shell was exited!");
  }
}

#[macro_export]
macro_rules! log
{
  ($new: expr) =>
  {
    {
      use $crate::init::{init_console::MASTER_LOG, QUIET};

      // Get the oncelock or fallback to false if not already set
      let quiet = QUIET.get().unwrap_or(&false);

      dbg!(quiet);

      // Don't print to console if quiet mode is enabled
      if (!quiet)
      {
        eprintln!("{}", $new);
      }

      MASTER_LOG.lock().unwrap().push($new);
    }
  };
}
pub use crate::log as log;

#[macro_export]
macro_rules! stall
{
  () =>
  {
    use std::{thread::sleep, time::Duration};
    use $crate::{state::INIT_STATE, state::InitState::Stalled};

    let mut state = INIT_STATE.lock().unwrap();

    // Signal to all components of init that we are stalled
    *state = Stalled;

    drop(state);

    // Sleep for 584,500,000,000 years aka a long time
    sleep(Duration::new(u64::MAX, 0));
  };
}
pub use crate::stall as stall;

/*
 * Print a message to the init's master log, e.g.:
 *
 * `status!("Hello world!");`
 *
 * Output: `[*] Hello world`
 */
#[macro_export]
macro_rules! status
{
  ($($message: tt)*) =>
  {
    {
      use $crate::init::init_console::{log, Marker::Status};
      log!(format!("{} {}", Status, format!($($message)*)))
    }
  };
}
pub use crate::status as status;

#[macro_export]
macro_rules! warn
{
  ($($message: tt)*) =>
  {
    {
      use $crate::init::init_console::{log, Marker::Warn};
      log!(format!("{} {}{}{}", Warn, Colour::BOLD, format!($($message)*), Colour::RESET))
    }
  };
}
pub use crate::warn as warn;
