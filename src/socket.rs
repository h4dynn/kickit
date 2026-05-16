/*!
 * ABI for kickit sockets
 * Keep in mind this isn't the complete implementation, see `init::socket` and `ktctl::socket`
 */

use std::path::PathBuf;
use tokio::net::UnixStream;

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
#[doc = include_str!("../docs/Making_a_Socket.md")]
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
    use crate::file_path;

    // These are the default paths, used except for when a custom method is defined
    if (Self::PRIVATE)
    {
      // This folder is root access only (0o600)
      file_path!(PathBuf::from("/run/kickit/private"), "io", Self::NAME)
    }
    else {
      // Can be accessed by anybody (0o666)
      file_path!(PathBuf::from("/run/kickit"), "io", Self::NAME)
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
  fn handler(&'static self, stream: UnixStream) -> impl Future<Output = ()> + Send;
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
