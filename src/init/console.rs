//! Error handling, warning & status implementations for the init process

use std::{sync::Mutex, fmt::{Display, Debug}};
use crate::{console::error_ty, state::InitState};

pub mod prelude
{
  // Universal console implementations shared between all programs
  pub use crate::console::prelude::*;

  pub use super::{QUIET, MASTER_LOG, Error, ErrorTrace, Result, status, warn, debug_cc};
}
use crate::console::prelude::*;

oncelock! {
  /// Hide master log entries from console TTY
  pub static QUIET: OnceLock<bool>;
}

/*
 * This is where our logs from things like `state!()`, `warn!()` and errors will be stored to (mostly for
 * debugging). Do not modify this manually, use the `log!()` macro instead
 */
#[doc(hidden)]
pub static MASTER_LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

error_ty! {
  ///
  /// All recorded possible init level errors. The goal is to be as descriptive & helpful as possible, while
  /// not overwhelming the user with unnecessary information.
  ///
  #[derive(PartialEq, Eq, Clone, Copy, Debug, thiserror::Error, Default)]
  #[must_use]
  pub enum Error
  {
    /// A generic error- thrown as sparingly as possible for exceptionally rare circumstances
    #[default]
    #[error("An unknown error occurred")]
    Unknown,

    /*
     * Errors during init startup
     * ==========================
     */
    #[error("kickit is already running!")]
    AlreadyRunning,
    #[error("Unsupported execution environment: must be ran as the init process")]
    NotInit,
    #[error("Insufficient permissions: must be ran as the root user")]
    NotRoot,

    /*
     * File access or modification errors
     * ==================================
     */
    #[error("File or directory not found")]
    FileNotFound,
    #[error("Failed to setup work directory")]
    RunFsFail,
    #[error("Kernel command-line parameter not found")]
    Cmdline,
    #[error("Invalid command-line parameter")]
    BadCmdline,
    #[error("Failed to get the current system time")]
    Time,

    /// Attempts to parse invalid data as UTF-8
    #[error("Failed to format content in UTF-8")]
    Format,

    /*
     * Target sourcing errors
     * ======================
     */
    #[error("Target not found, has it been created in the library folder?")]
    TargetNotFound,
    #[error("A required value is missing in target configuration")]
    TargetMissingValue,
    #[error("Failed to parse target configuration file")]
    TargetParse,

    /*
     * Socket input/output errors
     * ==========================
     */
    #[error("Failed to start up a socket")]
    SocketStartup,
    #[error("Socket read connection failure")]
    SocketIoRead,
    #[error("Socket write connection failure")]
    SocketIoWrite,
    #[error("Input/output connections failed on socket")]
    SocketIo,

    /*
     * Service misconfigurations or unexpected circumstances
     * =====================================================
     */
    #[error("Error occured while parsing service configuration file")]
    ServiceParse,
    #[error("A required file for this service could not be accessed")]
    ServiceAccess,
    #[error("Operation not supported for service's with this execution pattern")]
    ServicePatternMismatch,
    #[error("Service startup failure")]
    ServiceUp,
    #[error("Service stop failure")]
    ServiceDown,
    #[error("An error occurred in the service's logger")]
    ServiceLog,
    #[error("Service log read failure")]
    ServiceLogContent,
    #[error("Service log compression failure")]
    ServiceLogCompress,
    #[error("Service died during supervision, expected daemon execution")]
    ServiceDied,
    #[error("Service unexpectedly became a zombie during supervision, expected daemon execution")]
    ServiceZombified,
    #[error("Invalid service sandboxing option provided")]
    ServiceSandboxBadOpt,
    #[error("Error occurred while initializing service's sandbox environment")]
    ServiceSandboxInit,
    #[error("Service log file is inaccessible")]
    AccessLog,

    /*
     * Mount failures
     * ==============
     */
    #[error("Invalid formatting of mount entry")]
    MountParse,
    #[error("Error occurred while attempting to mount a critical virtual filesystem")]
    SysFsMount,
    #[error("Error occurred while unmounting a filesystem")]
    Unmount,

    /*
     * Miscellanous
     * ============
     */
    #[error("Failed to shutdown the init system")]
    Shutdown,
    #[error("Failed to read from the proc filesystem")]
    ProcFs,
    #[error("A critical error occurred while trying to power down the system")]
    PowerCritical
  }
}

macro_rules! inner_fatal
{
  // Implementation for a tracless error, just displays the provided message
  (@traceless $kind: tt) =>
  {
    log!(format!("{} {}", LogEntryType::FATAL_ERROR, $kind.bold()));
    $kind.exit();
  };
  (@trace $error: tt) =>
  {
    let Self { kind, trace, subject } = $error;

    log!({
      let trailing_trace_colon = (!trace.is_empty()).then_some(':').unwrap_or_default();
      if let Some(subject) = &subject
      {
        format!("{} {}: {}{}", LogEntryType::FATAL_ERROR, subject, kind.bold(), trailing_trace_colon)
      }
      else {
        format!("{} {}{}", LogEntryType::FATAL_ERROR, kind.bold(), trailing_trace_colon)
      }
    });

    if (!trace.is_empty())
    {
      for line in trace.lines()
      {
        log!(format!("{} (info): {}", LogEntryType::FATAL_ERROR, line));
      }
    }

    kind.exit();
  };
}

impl ReturnError for Error
{
  fn fatal(self) -> !
  {
    inner_fatal!(@traceless self);
  }
  fn warn(self)
  {
    warn!("{self}");
  }
}

impl ReturnError for ErrorTrace
{
  fn fatal(self) -> !
  {
    inner_fatal!(@trace self);
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

impl Error
{
  fn exit(self) -> !
  {
    use std::process::exit;
    use Error::{AlreadyRunning, NotInit, NotRoot};

    if (!matches!(self, AlreadyRunning | NotInit | NotRoot))
    {
      kickToEmergencyShell();
    }

    exit(1);
  }
}

/*
 * This will kick the user into an emergency shell when an error is encountered.
 * The shell, however, will not be privileged, as this creates a security risk,
 * since no password is asked for the root shell.
 */
fn kickToEmergencyShell() -> !
{
  use std::{process::Command, os::unix::process::CommandExt, thread::park};
  use crate::{init::{EMERGENCY_SHELL, EMERGENCY_SHELL_UID}, state::INIT_STATE};

  // "nobody" group
  const NOBODY_GID: u32 = 65534;

  // Try to lock the mutex & see if we are in emergency already
  if let Ok(mut state) = INIT_STATE.lock() && (*state != InitState::Emergency)
  {
    warn!("Critical error when starting init, starting unprivileged emergency shell now");

    *state = InitState::Emergency;
    // Allow other parts of the init system to access the state
    drop(state);

    // Run the emergency shell wrapper script, which should ensure this works in all environments
    Command::new("/usr/lib/kickit/emergency_shell")
      // Run as an unprivileged user, this means if we want to do anything requiring root we need a password
      .uid(EMERGENCY_SHELL_UID)
      .gid(NOBODY_GID)
      .arg(EMERGENCY_SHELL)
      .spawn()
      .expect("Failed to open an emergency shell!")
      .wait()
      .unwrap();

    warn!("Hanging on error, shell was exited!");
  }

  // Don't exit here otherwise we will get a kernel panic
  loop { park(); }
}

pub mod LogEntryType
{
  pub const STATUS: &str = concat!("\x1b[1m", "[*]", "\x1b[0m");
  pub const WARNING: &str = concat!("\x1b[1;33m", "[-]", "\x1b[0m");
  pub const FATAL_ERROR: &str = concat!("\x1b[1;31m", "[!]", "\x1b[0m");
  pub const SERVICE_RELAY: &str = concat!("\x1b[1;32m", "[>]", "\x1b[0m");

  #[cfg(debug_assertions)]
  pub const DEBUG: &str = concat!("\x1b[1;35m", "[?]", "\x1b[0m");
}

/**
  * Append a raw entry to the init's master log
  *
  * This macro shouldn't be called directly, instead use the macro corresponding
  * to the type of entry you want to append (e.g.: [`status!`], [`warn!`])
  */
#[macro_export]
macro_rules! init_log
{
  ($new: expr) =>
  {
    {
      use $crate::init::console::{MASTER_LOG, QUIET};

      // Get the oncelock or fallback to false if not already set
      let quiet = QUIET.get().unwrap_or(&false);

      // Don't print to console if quiet mode is enabled
      if (!quiet)
      {
        eprintln!("{}", $new);
      }

      MASTER_LOG.lock().unwrap().push($new);
    }
  };
}
pub use crate::init_log as log;

/**
  * Print a standard message to the init's master log
  *
  * # Example
  *
  * ```
  * init_status!("kickit {}", kickit::VERSION);
  * ```
  *
  * Output:
  *
  * ```txt
  * [*] kickit 26.07.1
  * ```
  */
#[macro_export]
macro_rules! init_status
{
  ($($message: tt)*) =>
  {
    {
      use $crate::init::console::{log, LogEntryType::STATUS};
      log!(format!("{} {}", STATUS, format!($($message)*)))
    }
  };
}
pub use crate::init_status as status;

/// Append a warning to init's master log
#[macro_export]
macro_rules! init_warn
{
  ($($message: tt)*) =>
  {
    {
      use $crate::init::console::{log, LogEntryType::WARNING};
      log!(format!("{} {}{}{}", WARNING, Colour::Bold, format!($($message)*), Colour::Reset))
    }
  };
}
pub use crate::init_warn as warn;

/**
  * A conditional-compilation macro that prints debugging information to stderr, if
  * compiled with debug profile and the `debug_einfo` feature.
  *
  * This macro acts almost identical to [`std::dbg`] except for its conditionality.
  *
  * NOTE: This doesn't add to the master log as it would use too much memory doing so.
  * You can only read from the init's stderr for these entries
  */
#[macro_export]
macro_rules! debug_cc
{
  ($($item: expr),*) =>
  {
    // These attributes are checked at compile-time
    #[cfg(debug_assertions)]
    #[cfg(feature = "debug_einfo")]
    {
      $(
        $crate::init::console::dbg_eprintln(stringify!($item), $item);
      )*
    }
  };
}
pub use crate::debug_cc as debug_cc;

/// Use a function as implementation so we can track the caller of the function
#[cfg(debug_assertions)]
#[track_caller]
pub fn dbg_eprintln(name: impl Display, item: impl Debug)
{
  use std::panic::Location;

  eprintln!("{} ({}) {name} = {item:?}", LogEntryType::DEBUG, Location::caller());
}
