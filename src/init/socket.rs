//! Socket implementations for init

use tokio::net::UnixStream;
use crate::{socket::{Socket, Core, Log, Power},
              init::console::{Result, Error, ErrorResult},
              console::{HandleError, ExtendWithContext}};

/*
 * Include the open_sock() method as a seperate trait with a blanket
 * implementation as this keeps the implementation universal
 * and stops custom implementations
 */
pub trait Open: Socket + Sync + 'static
{
  fn open_sock(&'static self) -> impl Future<Output = Result<()>> + Send
  {
    async move
    {
      use std::{os::unix::fs::PermissionsExt, fs::{Permissions, set_permissions}};
      use tokio::net::UnixSocket;

      let path = self.path();
      let permissions = Permissions::from_mode(if (Self::PRIVATE) { 0o600 } else { 0o666 });

      let sock = UnixSocket::new_stream().into_trace(Error::Socket)?;
      // Bind (start) our socket here
      sock.bind(&path).into_trace(Error::RunFsFail).context(path.display())?;

      /*
       * thanks <https://users.rust-lang.org/t/how-to-manage-permissions-of-a-unixlistener/31039/8>
       * for having an answer for this it really hurt my head
       */
      set_permissions(&path, permissions).into_trace(Error::RunFsFail).context(path.display())?;

      // Set max listeners at a time to 1
      let listener = sock.listen(Self::MAX_LISTENERS).into_trace(Error::Socket)?;

      while let Ok((stream, _)) = listener.accept().await
      {
        self.handler(stream).await;
      }
      Ok(())
    }
  }
}

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
  const MAX_LISTENERS: u32 = 3;

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
      Self::VERSION => stream.try_write(&[env!("CARGO_PKG_VERSION").as_bytes(), b"\n"].concat()),
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
    use crate::init::console::MASTER_LOG;

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
      stream.try_write(log.join("\n").as_bytes()).into_trace(Error::Socket).or_warn();
    }
    else {
      fail!(stream, 0x0f);
    }
  }
}

impl Socket for Power
{
  const NAME: &str = "Power";

  async fn handler(&self, stream: UnixStream)
  {
    use crate::init::power::{poweroff, forcePoweroff, Mode};

    stream_sanity!(stream => Readable + Writable);
    // Read 1 byte only
    let mut input = [0u8];

    if (stream.try_read(&mut input).is_err())
    {
      fail!(stream, 0xbb);
    }

    let _ = {
      match (input[0])
      {
        Self::SHUTDOWN => poweroff(Mode::Shutdown).or_warn(),
        Self::REBOOT => poweroff(Mode::Reboot).or_warn(),
        Self::FORCE_SHUTDOWN => forcePoweroff(Mode::Shutdown).or_warn(),
        Self::FORCE_REBOOT => forcePoweroff(Mode::Reboot).or_warn(),
        // Write error byte to socket- unexpected input
        _ => { fail!(stream, 0x0f); }
      }
    };
  }
}

// Blanket implementation
impl<S: Socket + Sync + 'static> Open for S {}
