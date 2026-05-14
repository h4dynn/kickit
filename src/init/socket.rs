//! Socket implementations for init

use tokio::net::UnixStream;
use crate::{socket::{Socket, Core, Log, Power},
            init::init_console::{Error, ErrorResult}, console::HandleError};

/*
 * Provide a failure byte to the socket peer as a signal that something
 * went wrong (this byte is usually 0x0f), shutdown the connection &
 * then return
 */
#[macro_export]
macro_rules! fail
{
  ($stream: expr, $byte: tt) =>
  {
    // Write our "error byte" to signal to peer an error has occurred
    $stream.try_write(&[$byte]).into_trace(Error::Socket).or_warn();
    // Exit our function- do nothing more here
    return
  };
}
pub use crate::fail as fail;

#[macro_export]
macro_rules! stream_sanity
{
  ($stream: expr => Readable) =>
  {
    if ($stream.readable().await.is_err())
    {
      fail!($stream, 0x3f);
    }
  };
  ($stream: expr => Writable) =>
  {
    if ($stream.writable().await.is_err())
    {
      fail!($stream, 0xe6);
    }
  };
  ($stream: expr => Readable + Writable) =>
  {
    stream_sanity!($stream => Readable);
    stream_sanity!($stream => Writable);
  };
}
pub use crate::stream_sanity as stream_sanity;

impl Socket for Core
{
  const NAME: &str = "Core";
  // All users should be able to access this socket as it reports no private data
  const PRIVATE: bool = false;

  async fn handler(&self, stream: UnixStream)
  {
    use crate::{state::state, init::TARGET_NAME};
    use std::process;

    stream_sanity!(stream => Readable + Writable);

    // Only want to read 1 byte
    let mut input = [0u8];

    if (stream.try_read(&mut input).is_err())
    {
      fail!(stream, 0xbb);
    }

    match (input[0])
    {
      Self::STATE =>
      {
        stream.try_write(&[state!() as u8])
      },
      // Add a newline byte, this is our EOL
      Self::VERSION => stream.try_write(&[crate::VERSION.to_string().as_bytes(), b"\n"].concat()),
      Self::TARGET =>
      {
        // Try open the OnceLock here
        if let Some(targetName) = TARGET_NAME.get()
        {
          stream.try_write(&[targetName.as_bytes(), b"\n"].concat())
        }
        else {
          // Error if target name cannot be reached for whatever reason
          fail!(stream, 0x0f);
        }
      },
      Self::PID => stream.try_write(&process::id().to_le_bytes()),
      // Safely ignore newlines
      b'\n' => Ok(0),
      // Send an error for unknown bytes
      _ => { fail!(stream, 0x0f); }
    }
      .into_trace(Error::Socket).or_warn();
  }
}

impl Socket for Log
{
  const NAME: &str = "Log";

  async fn handler(&self, stream: UnixStream)
  {
    use crate::init::init_console::MASTER_LOG;

    stream_sanity!(stream => Readable + Writable);
    let mut input = [0u8];

    if (stream.try_read(&mut input).is_err())
    {
      fail!(stream, 0xbb);
    }

    // If we receive the corresponding byte & the master log is lockable
    if (input[0] == Self::MASTER) && let Ok(log) = MASTER_LOG.lock()
    {
      // Our log is a vector of strings, so we seperate each member by a newline
      stream.try_write(log.join("\n").as_bytes())
    }
    else {
      stream.try_write(&[0x0f])
    }
      .into_trace(Error::Socket).or_warn();
  }
}

impl Socket for Power
{
  const NAME: &str = "Power";

  async fn handler(&self, stream: UnixStream)
  {
    use nix::sys::reboot::{reboot, RebootMode};

    stream_sanity!(stream => Readable + Writable);
    let mut input = [0u8];

    if (stream.try_read(&mut input).is_err())
    {
      fail!(stream, 0xbb);
    }

    match (input[0])
    {
      Self::SHUTDOWN => reboot(RebootMode::RB_POWER_OFF).into_trace(Error::Unknown).or_warn(),
      Self::REBOOT => reboot(RebootMode::RB_AUTOBOOT).into_trace(Error::Unknown).or_warn(),
      // Write error byte to socket- unexpected input
      _ => { fail!(stream, 0x0f); }
    }
  }
}
