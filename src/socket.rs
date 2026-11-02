/*!
 * ABI for socket input/output
 *
 * For socket behavior implementations, see `init::socket`, for requests see `ktctl::socket`
 */

use std::{io, path::PathBuf};
use tokio::net::UnixStream;

/// Error bytes sent to the peer on the other end of the socket
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error, Default)]
#[repr(u8)]
pub enum Error
{
  #[default]
  #[error("An unknown input/output error occurred on socket")]
  Unknown = 0xaa,
  /// Fallback error type for exceptionally rare errors
  #[error("init experienced an internal error while preparing response")]
  Internal = 0x0f,
  /// Wanted to read a request from peer but not ready
  #[error("Socket's stream is not readable")]
  NotReadReady = 0x3f,
  /// Wanted to write a response to peer but not ready
  #[error("Socket's stream is not writable")]
  NotWriteReady = 0xe6,
  /// Failed to read request from peer
  #[error("Failed to read peer's request on socket")]
  IoRead = 0xbb,
  /// Failed to write response to peer
  #[error("Failed to write response to peer")]
  IoWrite = 0xcc,
  /// Certain configurations will forbid some operations (e.g. no init = no force shutdown)
  #[error("This operation is unsupported in the current environment")]
  Unsupported = 0xa0,
  /// Peer wrote bad input to the socket
  #[error("Invalid input request provided")]
  BadInput = 0xdc
}

/**
  * Init core value provider
  * ========================
  *
  * Provides contents of the various values used at the core of the init system,
  * used to determine `ktctl`s behavior. This socket is public, accessible to any
  * user as it doesn't provide any sensitive values.
  *
  * Available values:
  *
  * ## state
  *
  * State of the init currently (see [`crate::state::InitState`]). In sane environments,
  * this should always be `Okay`. `ktctl` utilizes this value to safeguard some
  * operations (such as shutdown).
  *
  * ## version
  *
  * Constant, defined in source, compared with `ktctl`'s version for future proofing. If
  * a non-matching version is detected, `ktctl` will refuse to operate - in case of
  * potential incompatibilities introduced by new/old versions.
  *
  * ## target
  *
  * `OnceLock` value of the init target. This is `system` by default.
  *
  * ## pid
  *
  * `OnceLock` value of kickit's pid (if ran as init, it is `1`). This is used by
  * `ktctl` to prevent init-only operations from being performed in a non-init
  * environment.
  */
#[derive(PartialEq, Eq, Copy, Clone, Default, Debug)]
pub struct Core;
/**
  * Init master log provider
  * ========================
  *
  * A single-operation socket that returns the init system's current master log (see
  * [`crate::init::console::MASTER_LOG`]). Useful for debugging issues.
  *
  * Kept private in case the log contains any sensitive information that could be
  * abused by, for example, malicious software.
  */
#[derive(PartialEq, Eq, Copy, Clone, Default, Debug)]
pub struct Log;
/**
  * Power signal handler
  * ====================
  *
  * Receives poweroff/restart signals (which may also be forceful). After receiving a
  * signal, it begins the poweroff routine (see [`crate::init::power`]).
  *
  * This socket is private so that these signals can only be sent by the root user.
  */
#[derive(PartialEq, Eq, Copy, Clone, Default, Debug)]
pub struct Power;

// Limit by the static lifetime because Tokio spawning requires this
pub trait Socket: Send + 'static
{
  /// The name we will use for the socket in runfs, which will end up being `/run/kickit/io.{NAME}`
  const NAME: &str;
  /// Make the socket root-access only (mapped to private dir)
  const PRIVATE: bool = true;
  /// Max amount of listeners on this socket at a time
  const MAX_LISTENERS: u32 = 1;

  /// You may implement a custom method for a different path
  fn path(&self) -> PathBuf
  {
    // These are the default paths, used except for when a custom method is defined
    tern! {
      Self::PRIVATE => path!("/run/kickit/private", format!("io.{}", Self::NAME)),
      _ => path!("/run/kickit", format!("io.{}", Self::NAME))
    }
  }

  /**
    * When the socket receives a new connection, this function is called
    * to deal with it, where you can read the input with the `UnixStream`
    *
    * (note): The handler should be completely errorless, since we don't
    * want a stream that all users can access that may cause an init
    * error from a simple user request. Instead return an error to the
    * connected peer (see the [`crate::init::socket::relay`] macro)
    *
    * 'async' isn't specified here, instead a workaround (-> impl [`Future`])
    * because the compiler suggests to use it so others can use
    * auto-traits like Send if needed (lint: `clippy::async_fn_in_trait`)
    */
  fn handler(&'static self, stream: &mut UnixStream) -> impl Future<Output = ()> + Send;
}

/*
 * The bytes that we use for input requeats for the socket, we do
 * it this way so we have a global implementation that can be
 * changed later on if needed- less messy, more stable
 */
impl Core
{
  /// Init system's state (see [`crate::state::InitState`])
  pub const STATE: u8 = 0xf9;
  /// Version of this init system, used to avoid potential incompatibilities between ktctl & init versions
  pub const VERSION: u8 = 0xc4;
  /// Init's currently selected target (default is `system`)
  pub const TARGET: u8 = 0xa0;
  /// PID of this init system, if actually ran as the init, this will be `1`
  pub const PID: u8 = 0xf1;
}

impl Log
{
  /// Access the init's master log (see [`crate::init::console::MASTER_LOG`])
  pub const MASTER: u8 = 0xf5;
}

impl Power
{
  /// Standard shutdown request
  pub const SHUTDOWN: u8 = 0xe0;
  /// Standard reboot request
  pub const REBOOT: u8 = 0xd7;
  /// Bypass checks & graceful processes, skipping straight to shutdown (not recommended)
  pub const FORCE_SHUTDOWN: u8 = 0xfd;
  /// Bypass checks & graceful processes, skipping straight to reboot (not recommended)
  pub const FORCE_REBOOT: u8 = 0xe3;
}

impl Error
{
  /// 1st byte of socket output signifying an OK result
  pub const OK: u8 = 0xaf;
  /// 1st byte of socket output followed by error variant byte
  pub const ERR: u8 = 0xee;

  // Resolve error variant from its matching byte
  /**
    * # Errors
    *
    * * Input byte was matched to a `PeerError` variant
    */
  pub fn errorize(input: u8) -> io::Result<Self>
  {
    macro_rules! errorize
    {
      ($test: expr => $($variant: ident)|*) =>
      {
        $(
          if ($test == Self::$variant as u8)
          {
            return Ok(Self::$variant)
          }
        )*
      }
    }

    // If we match this byte to an error, return the matching error
    errorize!(input => Unknown | Internal | NotReadReady | NotWriteReady | IoRead | IoWrite | Unsupported | BadInput);
    // No error byte was found..
    Err(io::Error::new(io::ErrorKind::InvalidInput, format!("Couldn't match {input} to any valid error variant")))
  }
}
