//! Error handling, warning & status implementations for the init process

use std::{sync::Mutex, fmt::Display};
use crate::{oncelock, console::{Colour, Colourize, ReturnError, ErrorResult, ExtendWithContext, error}, state::InitState};

#[derive(derive_more::Display)]
pub enum Marker
{
  #[display("\x1b[1m[*]\x1b[0m")]
  Status,
  #[display("\x1b[1;33m[-]\x1b[0m")]
  Warn,
  #[display("\x1b[1;31m[!]\x1b[0m")]
  Fatal,
  #[display("\x1b[1;92m[>]\x1b[0m")]
  Service
}

oncelock! {
  // If set to true, master log entries won't be shown on console
  pub static QUIET: bool;
}

/*
 * This is where our logs from things like `state!()`, `warn!()`
 * and errors will be stored to (mostly for debugging). Do not
 * modify this manually, use the `log!()` macro instead
 */
#[doc(hidden)]
pub static MASTER_LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

error! {
  /*
   * Error stores all possible errors that could be thrown
   * Messages and whether the error should send you to an emergency shell
   * or not are defined in the impl
   */
  #[derive(PartialEq, Eq, Clone, Copy, Debug, thiserror::Error, Default)]
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
    #[error("Invalid command-line parameter")] BadCmdline,
    #[error("Failed to get the current system time")] Time,
    // When data can't be represented as a UTF-8 string
    #[error("Failed to format content in UTF-8")] Format,
    // Target-related errors (see how they are used in `src/target.rs`)
    #[error("Target not found, has it been created in the library folder?")] TargetNotFound,
    #[error("A required value is missing in target configuration")] TargetMissingValue,
    #[error("Failed to parse target configuration file")] TargetParse,
    // Socket data input/output failure
    #[error("Failed to start up a socket")] SocketStartup,
    #[error("Socket read connection failure")] SocketIoRead,
    #[error("Socket write connection failure")] SocketIoWrite,
    #[error("Input/output connections failed on socket")] SocketIo,
    // Service-related errors (used in `src/service.rs`)
    #[error("Failed to parse service configuration file")] ServiceParse,
    #[error("Failed to access a service's files")] ServiceAccess,
    #[error("Failed to start a service")] ServiceUp,
    #[error("Failed to stop a service")] ServiceDown,
    #[error("Failed to start logger for a service")] ServiceLog,
    #[error("Failed to read from the service's logger")] ServiceLogContent,
    #[error("Failed to compress/decompress the service's logger")] ServiceLogCompress,
    #[error("Service was killed or stopped")] ServiceNotRunning,
    #[error("Service became a zombie")] ServiceZombified,
    #[error("Invalid sandboxing option specified")] ServiceSandboxBadOpt,
    #[error("Failed to initialise sandbox directory")] ServiceSandboxInit,
    #[error("Failed to access a logfile")] AccessLog,
    // Mount-related failures
    #[error("Invalid formatted line in fstab")] FstabParse,
    #[error("Failed to mount a critical filesystem")] SysFsMount,
    #[error("Failed to unmount a filesystem")] SysFsUnmount,
    // Pure init errors
    #[error("Failed to shutdown the init system")] Shutdown,
    #[error("Failed to read from the proc filesystem")] ProcFs,
    #[error("A critical error occurred while trying to power down the system")] PowerCritical
  }
}

macro_rules! innerFatal
{
  // Implementation for a tracless error, just displays the provided message
  (@traceless $error: tt) =>
  {
    use $crate::init::console::Marker::Fatal;

    log!(format!("{Fatal} {}", $error.colour(Colour::Red)));
    $error.exit();
  };
  (@trace $error: tt) =>
  {
    use $crate::init::console::Marker::Fatal;

    // Move out from struct
    let ErrorTrace { kind, ref trace, context } = $error;

    log!(format!("{Fatal} {}{}", kind.colour(Colour::Red), context.as_ref().map(|c| format!(": {c}")).unwrap_or_default()));

    if (!$error.trace.is_empty())
    {
      log!(format!("{Fatal} >> {}", trace.trim_end_matches('\n')));
    }

    kind.exit();
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
    warn!("{self}");
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
  loop {
    park();
  }
}

#[macro_export]
macro_rules! log
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
pub use crate::log as log;

#[macro_export]
macro_rules! stall
{
  () => {
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
      use $crate::init::console::{log, Marker::Status};
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
      use $crate::init::console::{log, Marker::Warn};
      log!(format!("{} {}{}{}", Warn, Colour::Bold, format!($($message)*), Colour::Reset))
    }
  };
}
pub use crate::warn as warn;
