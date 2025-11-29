//! Error handling, warning & status implementations for the init process

use std::{sync::Mutex, fmt::Display};
use thiserror::Error;
use crate::{console::Colour, console::ReturnError, state::InitState};

// The marker for each of these display types
pub const STATUS: &str = "\x1b[1m[*]\x1b[0m";
pub const WARN: &str = "\x1b[1;33m[-]\x1b[0m";
pub const FATAL: &str = "\x1b[1;31m[!]\x1b[0m";
pub const SERVICE: &str = "\x1b[1;92m[>]\x1b[0m";

/*
 * This is where our logs from things like `state!()`, `warn!()`
 * and errors will be stored to (mostly for debugging). Do not
 * modify this manually, use the `log!()` macro instead
 */
#[doc(hidden)]
pub static MASTER_LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

// Just a Result where the error type is of KTErrorTrace
type KTResult<S> = Result<S, KTErrorTrace>;

/*
 * KTError stores all possible errors that could be thrown
 * Messages and whether the error should send you to an emergency shell
 * or not are defined in the impl
 */
#[derive(PartialEq, Eq, Clone, Copy, Debug, Error, Default)]
#[must_use]
pub enum KTError
{
  // A generic error- usually for internal errors / bugs
  #[error("An unknown error occurred")] #[default] Unknown,
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
  #[error("Failed to access logs from service")] ServiceAccess,
  #[error("Failed to start a service")] ServiceUp,
  #[error("Failed to stop a service")] ServiceDown,
  #[error("Failed to start a logger for a service")] ServiceLog,
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
pub struct KTErrorTrace { kind: KTError, context: Option<String>, trace: String }

pub trait ConvKTError
{
  type ErrType: Display;
  type OkType;
  /// Convert an error to a trace without context
  ///
  /// # Errors
  /// * Data type contains an error (e.g. Result is not Ok(x) variant)
  ///
  fn trace(self, errorKind: KTError) -> KTResult<Self::OkType>;
  /// Same but with context
  ///
  /// # Errors
  /// * Data type contains an error (e.g. Result is not Ok(x) variant)
  ///
  fn context_trace(self, context: impl ToString, errorKind: KTError)
    -> KTResult<Self::OkType>;
}

// A traceless, unknown error is the default
impl Default for KTErrorTrace { fn default() -> Self { KTErrorTrace::new(KTError::default(), "") } }

macro_rules! innerFatal
{
  // Implementation for a tracless error, just displays the provided message
  (@traceless $error: tt) =>
  {
    log!(format!("{}{} {}{}", $crate::init::init_console::FATAL, Colour::BOLD, $error,
                Colour::RESET));

    $error.exit();
  };

  (@trace $error: tt) =>
  {
    log!(format!("{}{} {}{}{}", $crate::init::init_console::FATAL, Colour::BOLD, $error.kind,
                  Colour::RESET,
                  if let Some(ref a) = $error.context { format!(": {a}") } else { String::new() }));

    if (!$error.trace.is_empty())
    {
      log!(format!("{}{} >> {}", $crate::init::init_console::FATAL, Colour::RESET,
                    $error.trace.trim_end_matches('\n')));
    }

    $error.kind.exit();
  };
}

/*
 * .fatal() implementations
 * KTError will throw just the message with no trace
 * KTErrorTrace will provide a trace
 */
impl ReturnError for KTError
{
  fn fatal(self) -> ! { innerFatal!(@traceless self); }
  fn warn(self) { warn!("{}", self.to_string()); }
}

impl ReturnError for KTErrorTrace
{
  fn fatal(self) -> ! { innerFatal!(@trace self); }

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
impl From<KTErrorTrace> for KTError { fn from(trace: KTErrorTrace) -> Self { trace.kind } }

// ...and vice versa
impl From<KTError> for KTErrorTrace { fn from(kind: KTError) -> Self { Self::new(kind, "") } }

impl KTError
{
  fn exit(self) -> !
  {
    use std::process;
    use KTError::{AlreadyRunning, NotInit, NotRoot};

    if (!matches!(self, AlreadyRunning | NotInit | NotRoot))
    {
      kickToEmergencyShell();
    }

    process::exit(1);
  }
}

impl KTErrorTrace
{
  pub fn new(kind: KTError, t: &(impl ToString + ?Sized)) -> Self
  {
    Self { kind, context: None, trace: t.to_string() }
  }

  pub fn with_context(kind: KTError, c: &(impl ToString + ?Sized), t: &(impl ToString + ?Sized))
    -> Self
  {
    Self { kind, context: Some(c.to_string()), trace: t.to_string() }
  }
}

impl<S, F: Display> ConvKTError for Result<S, F>
{
  type OkType = S;
  type ErrType = F;

  fn trace(self, errorKind: KTError) -> KTResult<Self::OkType>
  {
    self.map_err(|e| KTErrorTrace::new(errorKind, &e))
  }

  fn context_trace(self, context: impl ToString, errorKind: KTError)
    -> KTResult<Self::OkType>
  {
    self.map_err(|e| KTErrorTrace::with_context(errorKind, &context, &e))
  }
}

impl<F: Display> ConvKTError for Option<F>
{
  type OkType = ();
  type ErrType = F;

  fn trace(self, errorKind: KTError) -> KTResult<()>
  {
    if let Some(e) = self.map(|why| KTErrorTrace::new(errorKind, &why))
    {
      Err(e)
    }
    else {
      Ok(())
    }
  }

  fn context_trace(self, context: impl ToString, errorKind: KTError) -> KTResult<()>
  {
    if let Some(e) = self.map(|why| KTErrorTrace::with_context(errorKind, &context, &why))
    {
      Err(e)
    }
    else {
      Ok(())
    }
  }
}

impl ConvKTError for String
{
  type OkType = ();
  type ErrType = Self;

  fn trace(self, errorKind: KTError) -> KTResult<()>
  {
    Err(KTErrorTrace::new(errorKind, &self))
  }
  fn context_trace(self, context: impl ToString, errorKind: KTError)
    -> KTResult<Self::OkType>
  {
    Err(KTErrorTrace::with_context(errorKind, &context, &self))
  }
}

impl ConvKTError for std::io::Error
{
  type OkType = ();
  type ErrType = Self;

  fn trace(self, errorKind: KTError) -> KTResult<()>
  {
    Err(KTErrorTrace::new(errorKind, &self))
  }
  fn context_trace(self, context: impl ToString, errorKind: KTError)
    -> KTResult<Self::OkType>
  {
    Err(KTErrorTrace::with_context(errorKind, &context, &self))
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

#[macro_export] macro_rules! log
{
  ($new: expr) =>
  {
    {
      eprintln!("{}", $new);
      $crate::init::init_console::MASTER_LOG.lock().unwrap().push($new);
    }
  };
}
pub use crate::log as log;

#[macro_export] macro_rules! stall
{
  () =>
  {
    use std::{thread, time::Duration};
    use $crate::{state::INIT_STATE, state::InitState::Stalled};

    let mut state = INIT_STATE.lock().unwrap();

    // Signal to all components of init that we are stalled
    *state = Stalled;

    drop(state);

    // Sleep for 584500000000 years aka a long time
    thread::sleep(Duration::new(u64::MAX, 0));
  };
}
pub use crate::stall as stall;

#[macro_export] macro_rules! status
{
  ($($message: tt)*) =>
  {
    {
      $crate::init::init_console::log!(format!("{} {}", $crate::init::init_console::STATUS,
                                        format!($($message)*)))
    }
  };
}
pub use crate::status as status;

#[macro_export] macro_rules! warn
{
  ($($message: tt)*) =>
  {
    {
      $crate::init::init_console::log!(format!("{} {}{}{}", $crate::init::init_console::WARN,
                                              Colour::BOLD, format!($($message)*), Colour::RESET))
    }
  };
}
pub use crate::warn as warn;
