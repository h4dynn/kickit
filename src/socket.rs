//! Sockets for the runfs

use std::{os::unix::net::UnixStream as Stream, io::{Read, Write}, path::PathBuf};
use crate::{Data, init::init_console::{KTErrorTrace, KTError, KTErrorResult}, console::HandleKTError};

#[macro_export] macro_rules! socket_struct
{
  { $(pub $name: ident),* } =>
  {
    $(
      #[derive(PartialEq, Eq, Copy, Clone, Default, Debug)]
      pub struct $name;
    )*
  };
}
pub use crate::socket_struct as socket_struct;

/*
 * Provide a failure byte to the socket peer as a signal that something
 * went wrong (this byte is usually 0x0f), shutdown the connection &
 * then return
 */
#[macro_export] macro_rules! fail
{
  ($stream: expr, Default) =>
  {
    // The default failure byte is 0x0f
    $crate::socket::fail!($stream, 0x0f);
  };
  ($stream: expr, $byte: tt) =>
  {
    // Write our "error byte" to signal to peer an error has occurred
    $stream.write_all(&[$byte]).trace(KTError::Socket).or_warn();
    // Cleanup stream for both input & output
    Self::shutdown($stream).or_warn();
    // Exit our function- do nothing more here
    return
  };
}
pub use crate::fail as fail;

// The sockets- all of these will have KTSocket implemented
socket_struct! { pub Core, pub Log, pub Power }

// Accessing data from a Stream is tricky so this trait does it for us
pub trait StreamBytes: Into<Stream> + Read
{
  /*
   * Get <len> amount of bytes from a stream, this will loop forever
   * until the bytes are successfully read or timeout is reached
   */
  fn stream_bytes(&mut self, len: usize) -> Data;
  /*
   * Collect an indefinite amount of bytes until an end-of-line (\n)
   * is provided in the stream, note this will return None if just
   * a newline is provided and no other bytes
   */
  fn stream_bytes_eol(&mut self) -> Option<Data>;
}

// Limit by the static lifetime because Tokio spawning requires this
#[doc = include_str!("../docs/Making_a_Socket.md")]
pub trait KTSocket
{
  /*
   * The name we will use for the socket in runfs, which will end up
   * being /run/kickit/io.<SOCK_NAME>
   */
  fn name(&self) -> String;

  // Make the socket root-access only (mapped to private dir)
  const PRIVATE: bool = true;

  // Add an option for a custom path to place the socket in
  fn path(&self) -> Option<PathBuf> { None }

  /*
   * (note): The handler should be completely errorless, since we don't
   * want a stream that all users can access that may cause an init
   * error from a simple user request. Instead return an error to the
   * connected peer (e.g. 0x0f byte)
   *
   * 'async' isn't specified here, instead a workaround (-> impl Future)
   * because the compiler suggests to use it so others can use
   * auto-traits if needed (lint: `async_fn_in_trait`)
   */
  fn handler(&'static self, stream: Stream) -> impl Future<Output = ()> + Send;

  /**
    * # Errors
    *
    * - Failed to shutdown the stream for whatever reason
    */
  fn shutdown(stream: Stream) -> Result<(), KTErrorTrace>
  {
    use std::net;
    stream.shutdown(net::Shutdown::Both).trace(KTError::Socket)
  }
}

/*
 * Include the start() method as a seperate trait with a blanket
 * implementation as this keeps the implementation universal
 * and stops custom implementations
 */
pub trait Start: KTSocket + Sync + 'static
{
  fn start(&'static self) -> impl Future<Output = Result<(), KTErrorTrace>> + Send
  {
    async move
    {
      use std::{os::unix::{net::UnixListener as Listener, fs::PermissionsExt},
                fs::{Permissions, set_permissions}};

      use tokio::task;
      use crate::file_path;

      // The socket gets mapped to a matching path with its real name field
      let path = if let Some(__path) = self.path()
      {
        __path
      }
      // These are the default paths, used except for when a custom method is defined
      else if (Self::PRIVATE)
      {
        file_path!(PathBuf::from("/run/kickit/private"), "io", self.name())
      }
      else
      {
        file_path!(PathBuf::from("/run/kickit"), "io", self.name())
      };

      // Bind (start) our socket here
      let sock = Listener::bind(&path).context_trace(path.display(), KTError::RunFsFail)?;

      /*
       * thanks <https://users.rust-lang.org/t/how-to-manage-permissions-of-a-unixlistener/31039/8>
       * for having an answer for this it really hurt my head
       */
      set_permissions(&path, Permissions::from_mode(if (Self::PRIVATE) { 0o600 } else { 0o666 }))
          .context_trace(path.display(), KTError::RunFsFail)?;

      for peer in (sock.incoming())
      {
        match (peer)
        {
          // Run the handler once a valid peer is found
          Ok(stream) => { task::spawn(self.handler(stream)); },
          // Usually this just means there are no more streams to read
          Err(..) => break
        }
      }
      Ok(())
    }
  }
}

// Blanket implementation
impl<S: KTSocket + Sync + 'static> Start for S {}

/*
 * The bytes that we use for input requeats for the socket, we do
 * it this way so we have a global implementation that can be
 * changed later on if needed- less messy, more stable
 */
impl Core
{
  pub const STATE: u8 = 0x4D;
  pub const VERSION: u8 = 0x1C;
  pub const TARGET: u8 = 0x7E;
  pub const PID: u8 = 0xF1;
}

impl Log
{
  // The init master log
  pub const MASTER: u8 = 0x6C;
}

impl Power
{
  pub const SHUTDOWN: u8 = 0xF2;
  pub const REBOOT: u8 = 0x7E;
}

impl StreamBytes for Stream
{
  fn stream_bytes(&mut self, len: usize) -> Data
  {
    // Initialise our buffer that data will be read to
    let mut bytes = Data::new();

    loop {
      // Wait until <len> bytes appear in the stream & can be read
      if (self.take(len as u64).read_to_end(&mut bytes).ok() == Some(len))
      {
        return bytes
      }
    }
  }

  fn stream_bytes_eol(&mut self) -> Option<Data>
  {
    let mut currentByte: [u8; 1] = [0; 1];
    let mut bytes = Data::new();

    loop {
      if (self.take(1).read_exact(&mut currentByte).is_ok())
      {
        // If we reach a newline or end-of-line, we can return the data we already read
        if (currentByte[0] == b'\n')
        {
          return Some(bytes)
        }
        // Add the byte we just read to our collection of bytes
        bytes.push(currentByte[0]);
      }
    }
  }
}

impl KTSocket for Core
{
  fn name(&self) -> String { String::from("Core") }
  // All users should be able to access this socket as it reports no private data
  const PRIVATE: bool = false;

  async fn handler(&self, mut stream: Stream)
  {
    use crate::{state::state, init::target::TARGET_NAME};
    use std::process;

    match (stream.stream_bytes(1).as_slice())
    {
      [Self::STATE] => stream.write_all(&[state!() as u8]),
      // Add a newline byte, this is our EOL
      [Self::VERSION] => stream.write_all(&[crate::VERSION.to_string().as_bytes(), b"\n"].concat()),
      [Self::TARGET] =>
      {
        // Try open the OnceLock here
        if let Some(targetName) = TARGET_NAME.get()
        {
          stream.write_all(&[targetName.as_bytes(), b"\n"].concat())
        }
        else {
          // Error if target name cannot be reached for whatever reason
          fail!(stream, Default);
        }
      },
      [Self::PID] => stream.write_all(&process::id().to_be_bytes()),
      // Safely ignore newlines
      [b'\n'] => Ok(()),
      // Send an error for unknown bytes
      _ => { fail!(stream, Default); }
    }
      .trace(KTError::Socket).or_warn();

    // Cleanup stream for both input & output- we are done here
    Self::shutdown(stream).or_warn();
  }
}

impl KTSocket for Log
{
  fn name(&self) -> String { String::from("Log") }

  async fn handler(&self, mut stream: Stream)
  {
    use crate::init::init_console::MASTER_LOG;

    // If we receive the corresponding byte & the master log is lockable
    if (stream.stream_bytes(1) == [Self::MASTER]) && let Ok(log) = MASTER_LOG.lock()
    {
      // Our log is a vector of strings, so we seperate each member by a newline
      stream.write_all(log.join("\n").as_bytes())
    }
    else {
      stream.write_all(&[0x0f])
    }
      .trace(KTError::Socket).or_warn();

    Self::shutdown(stream).or_warn();
  }
}

impl KTSocket for Power
{
  fn name(&self) -> String { String::from("Power") }

  async fn handler(&self, mut stream: Stream)
  {
    use crate::{console::Colour, init::{POWER_LEVEL, PowerLevel, init_console::warn}};

    /*
     * None of these errors (OnceLock<T>::set) carry any remotely useful
     * error information- so we can just check it succeeded & throw a
     * generic error if not (via `.map_err(|_|)`)
     */
    if let Err(reason) = match (stream.stream_bytes(1)[0])
    {
      Self::SHUTDOWN => POWER_LEVEL.set(PowerLevel::Off)
                            .map_err(|_| String::from("Failed to set power level")),

      Self::REBOOT => POWER_LEVEL.set(PowerLevel::Reboot)
                            .map_err(|_| String::from("Failed to set power level")),

      // Write error byte to socket- unexpected input
      _ => stream.write_all(&[0x0f]).map_err(|e| e.to_string())
    } {
      warn!("io.Power: Internal socket error: {reason}");
    }

    // This might not execute because init will start shutdown at this point
    Self::shutdown(stream).or_warn();
  }
}
