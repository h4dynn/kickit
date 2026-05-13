//! Socket implementations for init

use tokio::net::UnixStream;
use crate::{socket::{Socket, Core, Log, Power, fail, stream_sanity},
            init::init_console::{Error, ErrorResult}, console::HandleError};

impl Socket for Core
{
  fn name(&self) -> String
  {
    String::from("Core")
  }
  // All users should be able to access this socket as it reports no private data
  const PRIVATE: bool = false;

  async fn handler(&self, stream: UnixStream)
  {
    use crate::{state::state, init::target::TARGET_NAME};
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
      Self::PID => stream.try_write(&process::id().to_be_bytes()),
      // Safely ignore newlines
      b'\n' => Ok(0),
      // Send an error for unknown bytes
      _ => { fail!(stream, 0x0f); }
    }
      .trace(Error::Socket).or_warn();
  }
}

impl Socket for Log
{
  fn name(&self) -> String
  {
    String::from("Log")
  }

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
      .trace(Error::Socket).or_warn();
  }
}

impl Socket for Power
{
  fn name(&self) -> String
  {
    String::from("Power")
  }

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
      Self::SHUTDOWN => reboot(RebootMode::RB_POWER_OFF).trace(Error::Unknown).or_warn(),
      Self::REBOOT => reboot(RebootMode::RB_AUTOBOOT).trace(Error::Unknown).or_warn(),
      // Write error byte to socket- unexpected input
      _ => { fail!(stream, 0x0f); }
    }
  }
}
