//! Socket implementations for init

use tokio::net::UnixStream;
use crate::{socket::{Socket, PeerError, Core, Log, Power}, console::{ErrorResult, ReturnError}, init::console::{Result, Error},
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

#[macro_export(local_inner_macros)]
macro_rules! socket_relay
{
  // Provide the OK byte as well as our bytes
  ($stream: expr, Ok($ok: expr)) =>
  {
    {
      socket_relay!(@write $stream, &[PeerError::IS_OK]);
      socket_relay!(@write $stream, $ok);
    }
  };
  /*
   * Provide a failure byte to the socket peer as a signal that something
   * went wrong, shutdown the connection & then return, see `socket::PeerError`
   * for all possible errors
   */
  ($stream: expr, Err($error: path)) =>
  {
    {
      // Write our "error byte" to signal to peer an error has occurred
      $stream.try_write(&[PeerError::IS_ERROR, $error as u8]).into_trace(Error::SocketIoWrite).or_warn();
      // Exit our function- do nothing more here
      return
    }
  };
  (@write $stream: expr, $bytes: expr) =>
  {
    if let Err(err) = $stream.try_write($bytes).into_trace(Error::SocketIoWrite)
    {
      err.warn();
      socket_relay!($stream, Err(PeerError::IoWrite))
    }
  };
}
pub use crate::socket_relay as relay;

#[macro_export]
macro_rules! stream_sanity
{
  ($stream: expr => Readable) =>
  {
    if ($stream.readable().await.is_err())
    {
      relay!($stream, Err(PeerError::NotReadReady));
    }
  };
  ($stream: expr => Writable) =>
  {
    if ($stream.writable().await.is_err())
    {
      relay!($stream, Err(PeerError::NotWriteReady));
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
    use crate::{state::state, init::{PID, target::TARGET}};

    stream_sanity!(stream => Readable + Writable);

    // Only want to read 1 byte
    let mut input = [0u8];

    if (stream.try_read(&mut input).is_err())
    {
      relay!(stream, Err(PeerError::IoRead));
    }

    match (input[0])
    {
      Self::STATE => relay!(stream, Ok(&[state!() as u8])),
      // Add a newline byte, this is our EOL
      Self::VERSION => relay!(stream, Ok(&[env!("CARGO_PKG_VERSION").as_bytes(), b"\n"].concat())),
      Self::TARGET =>
      {
        if let Some(target) = TARGET.get()
        {
          relay!(stream, Ok(&[target.name.as_bytes(), b"\n"].concat()));
        }
        else {
          // Error if target name cannot be reached for whatever reason
          relay!(stream, Err(PeerError::Internal));
        }
      },
      Self::PID =>
      {
        if let Some(pid) = PID.get()
        {
          relay!(stream, Ok(&pid.unwrap_or(1).to_le_bytes()));
        }
        else {
          // This will only happen if the PID is uninitialized which it shouldn't ever be
          relay!(stream, Err(PeerError::Internal));
        }
      },
      // Safely ignore newlines
      b'\n' => (),
      // Send an error for unknown bytes
      _ => relay!(stream, Err(PeerError::BadInput))
    }
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
      relay!(stream, Err(PeerError::IoRead));
    }

    // If we receive the corresponding byte & the master log is lockable
    if (input[0] == Self::MASTER) && let Ok(log) = MASTER_LOG.lock()
    {
      // Our log is a vector of strings, so we seperate each member by a newline
      relay!(stream, Ok(log.join("\n").as_bytes()));
    }
    else {
      relay!(stream, Err(PeerError::Internal));
    }
  }
}

impl Socket for Power
{
  const NAME: &str = "Power";

  async fn handler(&self, stream: &mut UnixStream)
  {
    use crate::{console::ReturnError, TrashUnused, init::{PID, power::{poweroff, forcePoweroff, Mode}}};

    stream_sanity!(stream => Readable + Writable);
    // Read 1 byte only
    let mut input = [0u8];

    if (stream.try_read(&mut input).is_err())
    {
      relay!(stream, Err(PeerError::IoRead));
    }

    match (input[0])
    {
      Self::SHUTDOWN => poweroff(Mode::Shutdown).or_warn().trash(),
      Self::REBOOT => poweroff(Mode::Reboot).or_warn().trash(),
      Self::FORCE_SHUTDOWN =>
      {
        if let Some(pid) = PID.get() && (pid.is_none())
        {
          forcePoweroff(Mode::Shutdown).or_warn();
        }
        else {
          Error::Shutdown.trace("Force shutdown is not supported when kickit is not ran as the init process!").warn();
          relay!(stream, Err(PeerError::Unsupported));
        }
      }
      Self::FORCE_REBOOT =>
      {
        if let Some(pid) = PID.get() && (pid.is_none())
        {
          forcePoweroff(Mode::Reboot).or_warn();
        }
        else {
          Error::Shutdown.trace("Force reboot is not supported when kickit is not ran as the init process!").warn();
          relay!(stream, Err(PeerError::Unsupported));
        }
      },
      // Write error byte to socket- unexpected input
      _ => relay!(stream, Err(PeerError::BadInput))
    }
  }
}

// Blanket implementation
impl<S: Socket + Sync + 'static> Open for S {}
