//! Socket implementations for init

use tokio::net::UnixStream;
use crate::{socket::{Socket, PeerError, Core, Log, Power}, init::console::{Result, Error, ErrorResult},
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
      use tokio::{task, net::UnixSocket};

      let mode = if (Self::PRIVATE) { 0o600 } else { 0o666 };
      let permissions = Permissions::from_mode(mode);

      let sock = UnixSocket::new_stream().into_trace(Error::SocketStartup)?;
      // Bind (start) our socket here
      sock.bind(self.path()).into_trace(Error::RunFsFail).context(self.path().display())?;

      /*
       * thanks <https://users.rust-lang.org/t/how-to-manage-permissions-of-a-unixlistener/31039/8>
       * for having an answer for this it really hurt my head
       */
      set_permissions(self.path(), permissions).into_trace(Error::ServiceAccess).context(self.path().display())?;

      // Set max listeners at a time (backlog)
      let listener = sock.listen(Self::MAX_LISTENERS).into_trace(Error::SocketIo)?;

      // Loop forever, handling each incoming connection
      loop {
        // Start accepting incoming connections
        let (mut socket, _) = listener.accept().await.map_err(|_| Error::SocketStartup.trace("Failed to start listener"))?;

        task::spawn(async move { self.handler(&mut socket).await; });
      }
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
  ($stream: expr, $error: path) =>
  {
    {
      // Write our "error byte" to signal to peer an error has occurred
      $stream.try_write(&[PeerError::IS_ERROR, $error as u8]).into_trace(Error::SocketIoWrite).or_warn();
      // Exit our function- do nothing more here
      return
    }
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
      fail!($stream, PeerError::NotReadReady);
    }
  };
  ($stream: expr => Writable) =>
  {
    if ($stream.writable().await.is_err())
    {
      fail!($stream, PeerError::NotWriteReady);
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

  async fn handler(&self, stream: &mut UnixStream)
  {
    use crate::{oncelock, state::state, init::{NO_INIT, target::TARGET_NAME}};
    use std::process;

    stream_sanity!(stream => Readable + Writable);

    // Only want to read 1 byte
    let mut input = [0u8];

    if (stream.try_read(&mut input).is_err())
    {
      fail!(stream, PeerError::IoRead);
    }

    match (input[0])
    {
      Self::STATE => stream.try_write(&[state!() as u8]),
      // Add a newline byte, this is our EOL
      Self::VERSION => stream.try_write(&[env!("CARGO_PKG_VERSION").as_bytes(), b"\n"].concat()),
      Self::TARGET =>
      {
        if let Ok(targetName) = oncelock!(&TARGET_NAME)
        {
          stream.try_write(&[targetName.as_bytes(), b"\n"].concat())
        }
        else {
          // Error if target name cannot be reached for whatever reason
          fail!(stream, PeerError::Internal);
        }
      },
      Self::PID => stream.try_write(&process::id().to_le_bytes()),
      Self::NO_INIT =>
      {
        if let Ok(noInit) = oncelock!(&NO_INIT)
        {
          stream.try_write(&[(*noInit).into()])
        }
        else {
          fail!(stream, PeerError::Internal);
        }
      },
      // Safely ignore newlines
      b'\n' => Ok(0),
      // Send an error for unknown bytes
      _ => fail!(stream, PeerError::BadInput)
    }
      .into_trace(Error::SocketIoWrite).or_warn();
  }
}

impl Socket for Log
{
  const NAME: &str = "Log";

  async fn handler(&self, stream: &mut UnixStream)
  {
    use crate::init::console::MASTER_LOG;

    stream_sanity!(stream => Readable + Writable);
    let mut input = [0u8];

    if (stream.try_read(&mut input).is_err())
    {
      fail!(stream, PeerError::IoRead);
    }

    // If we receive the corresponding byte & the master log is lockable
    if (input[0] == Self::MASTER) && let Ok(log) = MASTER_LOG.lock()
    {
      // Our log is a vector of strings, so we seperate each member by a newline
      stream.try_write(log.join("\n").as_bytes()).into_trace(Error::SocketIoWrite).or_warn();
    }
    else {
      fail!(stream, PeerError::Internal);
    }
  }
}

impl Socket for Power
{
  const NAME: &str = "Power";

  async fn handler(&self, stream: &mut UnixStream)
  {
    use crate::{oncelock, console::ReturnError, TrashUnused, init::{NO_INIT, power::{poweroff, forcePoweroff, Mode}}};

    stream_sanity!(stream => Readable + Writable);
    // Read 1 byte only
    let mut input = [0u8];

    if (stream.try_read(&mut input).is_err())
    {
      fail!(stream, PeerError::IoRead);
    }

    match (input[0])
    {
      Self::SHUTDOWN => poweroff(Mode::Shutdown).or_warn().trash(),
      Self::REBOOT => poweroff(Mode::Reboot).or_warn().trash(),
      Self::FORCE_SHUTDOWN =>
      {
        if (oncelock!(&NO_INIT) == Ok(&false))
        {
          forcePoweroff(Mode::Shutdown).or_warn();
        }
        else {
          Error::Shutdown.trace("Force shutdown is not supported when kickit is not ran as the init process!").warn();
          fail!(stream, PeerError::BadInput);
        }
      }
      Self::FORCE_REBOOT =>
      {
        if (oncelock!(&NO_INIT) == Ok(&false))
        {
          forcePoweroff(Mode::Reboot).or_warn();
        }
        else {
          Error::Shutdown.trace("Force reboot is not supported when kickit is not ran as the init process!").warn();
          fail!(stream, PeerError::BadInput);
        }
      },
      // Write error byte to socket- unexpected input
      _ => fail!(stream, PeerError::BadInput)
    }
  }
}

// Blanket implementation
impl<S: Socket + Sync + 'static> Open for S {}
