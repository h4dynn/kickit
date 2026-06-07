/*!
 * ABI for kickit sockets
 * For socket behavior implementations, see `init::socket`, for generic requests see `ktctl::socket`
 */

use std::{io, path::PathBuf};
use tokio::net::UnixStream;
use thiserror::Error;

// Error bytes sent to the peer on the other end of the socket
#[derive(Copy, Clone, PartialEq, Eq, Debug, Error, Default)]
pub enum PeerError
{
  #[default]
  #[error("An unknown input/output error occurred on socket")]
  Unknown = 0xaa,
  // Fallback error type for exceptionally rare errors
  #[error("init experienced an internal error while preparing response")]
  Internal = 0x0f,
  // Wanted to read a request from peer but not ready
  #[error("Socket's stream is not readable")]
  NotReadReady = 0x3f,
  // Wanted to write a response to peer but not ready
  #[error("Socket's stream is not writable")]
  NotWriteReady = 0xe6,
  // Failed to read request from peer
  #[error("Failed to read peer's request on socket")]
  IoRead = 0xbb,
  // Failed to write response to peer
  #[error("Failed to write response to peer")]
  IoWrite = 0xcc,
  // Certain configurations will forbid some operations (e.g. no init = no force shutdown)
  #[error("This operation is unsupported in the current environment")]
  Unsupported = 0xa0,
  // Peer wrote bad input to the socket
  #[error("Invalid input request provided")]
  BadInput = 0xdc
}

// Provides init state, version, target & PID
#[derive(PartialEq, Eq, Copy, Clone, Default, Debug)]
pub struct Core;
// Provides the init's master log - not service logs
#[derive(PartialEq, Eq, Copy, Clone, Default, Debug)]
pub struct Log;
// Interface for rebooting & shutting down
#[derive(PartialEq, Eq, Copy, Clone, Default, Debug)]
pub struct Power;

// Limit by the static lifetime because Tokio spawning requires this
pub trait Socket: Send + 'static
{
  /*
   * The name we will use for the socket in runfs, which will end up
   * being /run/kickit/io.<SOCK_NAME>
   */
  const NAME: &str;
  // Make the socket root-access only (mapped to private dir)
  const PRIVATE: bool = true;
  // Max amount of listeners on this socket at a time
  const MAX_LISTENERS: u32 = 1;

  // You may implement a custom method for a different path
  fn path(&self) -> PathBuf
  {
    use crate::{tern, file_path};

    // These are the default paths, used except for when a custom method is defined
    tern! {
      Self::PRIVATE => file_path!(PathBuf::from("/run/kickit/private"), "io", Self::NAME),
      else => file_path!(PathBuf::from("/run/kickit"), "io", Self::NAME)
    }
  }

  /*
   * When the socket receives a new connection, this function is called
   * to deal with it, where you can read the input with the UnixStream
   *
   * (note): The handler should be completely errorless, since we don't
   * want a stream that all users can access that may cause an init
   * error from a simple user request. Instead return an error to the
   * connected peer (see the `init::socket::fail` macro)
   *
   * 'async' isn't specified here, instead a workaround (-> impl Future)
   * because the compiler suggests to use it so others can use
   * auto-traits like Send if needed (lint: `async_fn_in_trait`)
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
  pub const STATE: u8 = 0xf9;
  pub const VERSION: u8 = 0xc4;
  pub const TARGET: u8 = 0xa0;
  pub const PID: u8 = 0xf1;
}

impl Log
{
  // The init master log
  pub const MASTER: u8 = 0xf5;
}

impl Power
{
  pub const SHUTDOWN: u8 = 0xe0;
  pub const REBOOT: u8 = 0xd7;
  pub const FORCE_SHUTDOWN: u8 = 0xfd;
  pub const FORCE_REBOOT: u8 = 0xe3;
}

impl PeerError
{
  pub const IS_OK: u8 = 0xaf;
  // Marker that this reply from socket is infact an error
  pub const IS_ERROR: u8 = 0xee;

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
