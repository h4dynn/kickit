//! Init input/output implementations for sockets

use tokio::net::UnixStream;
use crate::{socket::{Socket, Core, Log, Power}, socket, init::console::prelude::*};

/**
  * The [`Open::open_sock`] method is a function with a blanket which keeps the implementation universal
  * between all sockets and stops custom implementations
  */
pub trait Open: Socket + Sync + 'static
{
  /// Initializes the socket in its provided path, creating a listener for input/output
  fn open_sock(&'static self) -> impl Future<Output = Result<()>> + Send
  {
    async move
    {
      use std::{os::unix::fs::PermissionsExt, fs::{Permissions, set_permissions}};
      use tokio::{task, net::UnixSocket};

      let path = self.path();
      let permissions = Permissions::from_mode(if Self::PRIVATE { 0o600 } else { 0o666 });

      let sock = UnixSocket::new_stream().into_trace(Error::SocketStartup)?;
      // Bind (start) our socket here
      sock.bind(&path).into_trace(Error::RunFsFail).subject(self.path().display())?;

      /*
       * thanks <https://users.rust-lang.org/t/how-to-manage-permissions-of-a-unixlistener/31039/8>
       * for having an answer for this it really hurt my head
       */
      set_permissions(path, permissions).into_trace(Error::ServiceAccess).subject(self.path().display())?;

      // Set max listeners at a time (backlog)
      let listener = sock.listen(Self::MAX_LISTENERS).into_trace(Error::SocketIo)?;

      // Loop forever, handling each incoming connection
      loop {
        // Start accepting incoming connections
        let (mut socket, _) = listener.accept().await
                                .map_err(|_| Error::SocketStartup.trace("failed to start listener"))?;

        task::spawn(async move { self.handler(&mut socket).await; });
      }
    }
  }
}

/// Relay to the receiving peer of this socket a `Result` in byte form (see [`crate::socket::Error`])
#[macro_export(local_inner_macros)]
macro_rules! socket_relay
{
  // Provide the OK byte as well as our bytes
  ($stream: expr, Ok($ok: expr)) =>
  {{
    socket_relay!(@write $stream, &[socket::Error::OK as u8]);
    socket_relay!(@write $stream, $ok);
  }};
  /*
   * Provide a failure byte to the socket peer as a signal that something went wrong, shutdown the
   * connection & then return, see `socket::Error`for all possible errors
   */
  ($stream: expr, Err($error: path)) =>
  {{
    // Write our "error byte" to signal to peer an error has occurred
    $stream.try_write(&[socket::Error::ERR, $error as u8]).into_trace(Error::SocketIoWrite).or_warn();
    // Exit our function- do nothing more here
    return
  }};
  (@write $stream: expr, $bytes: expr) =>
  {
    if let Err(err) = $stream.try_write($bytes).into_trace(Error::SocketIoWrite)
    {
      // TO-DO: Generally for debugging purposes only, so may fill up init logs if we're not careful
      err.warn();

      socket_relay!($stream, Err(socket::Error::IoWrite))
    }
  };
}
pub use crate::socket_relay as relay;

/// Make sure init stream ([`UnixStream`]) is ready to be read/written to
#[macro_export]
macro_rules! stream_sanity
{
  ($stream: expr => Readable) =>
  {
    if ($stream.readable().await.is_err())
    {
      relay!($stream, Err(socket::Error::NotReadReady))
    }
  };
  ($stream: expr => Writable) =>
  {
    if ($stream.writable().await.is_err())
    {
      relay!($stream, Err(socket::Error::NotWriteReady))
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
    use crate::{state::state, init::target::TARGET};
    use std::process::id as pid;

    stream_sanity!(stream => Readable + Writable);

    // Only want to read 1 byte
    let mut input = [0u8];

    if (stream.try_read(&mut input).is_err())
    {
      relay!(stream, Err(socket::Error::IoRead));
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
          relay!(stream, Err(socket::Error::Internal));
        }
      },
      Self::PID => relay!(stream, Ok(&pid().to_le_bytes())),
      // Safely ignore newlines
      b'\n' => (),
      // Send an error for unknown bytes
      _ => relay!(stream, Err(socket::Error::BadInput))
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
      relay!(stream, Err(socket::Error::IoRead));
    }

    // If we receive the corresponding byte & the master log is lockable
    if (input[0] == Self::MASTER) && let Ok(log) = MASTER_LOG.lock()
    {
      // Our log is a vector of strings, so we seperate each member by a newline
      relay!(stream, Ok(log.join("\n").as_bytes()));
    }
    else {
      relay!(stream, Err(socket::Error::Internal));
    }
  }
}

impl Socket for Power
{
  const NAME: &str = "Power";

  async fn handler(&self, stream: &mut UnixStream)
  {
    use crate::init::{init_guard, power::{poweroff, force_poweroff, Mode}};

    stream_sanity!(stream => Readable + Writable);

    let mut input = [0u8];

    if (stream.try_read(&mut input).is_err())
    {
      relay!(stream, Err(socket::Error::IoRead));
    }
    debug_cc!(&input[0]);

    match (input[0])
    {
      Self::SHUTDOWN => poweroff(Mode::Shutdown).or_warn(),
      Self::REBOOT => poweroff(Mode::Reboot).or_warn(),
      Self::FORCE_SHUTDOWN | Self::FORCE_REBOOT if init_guard() =>
      {
        ReturnError::warn(Error::Shutdown.trace("force shutdown is unsupported when kickit is not init!"));
        relay!(stream, Err(socket::Error::Unsupported));
      },
      Self::FORCE_SHUTDOWN => force_poweroff(Mode::Shutdown).or_warn(),
      Self::FORCE_REBOOT => force_poweroff(Mode::Reboot).or_warn(),
      // Write error byte to socket- unexpected input
      _ => relay!(stream, Err(socket::Error::BadInput))
    };
  }
}

// Blanket implementation
impl<S: Socket + Sync + 'static> Open for S {}
