//! Sockets for the runfs

use std::{os::unix::net::UnixStream, io::{Read, Write}};
use crate::{Data, init::init_console::{KTErrorTrace, KTError, ConvKTError}, console::HandleKTError};

// The sockets- all of these will have KTSocket implemented
#[derive(PartialEq, Eq, Copy, Clone, Default, Debug)]
pub struct Core;
#[derive(PartialEq, Eq, Copy, Clone, Default, Debug)]
pub struct Log;
#[derive(PartialEq, Eq, Copy, Clone, Default, Debug)]
pub struct Power;

// Accessing data from a UnixStream is tricky so this trait does it for us
pub trait StreamBytes: From<UnixStream> + Read
{
  /*
   * Get <len> amount of bytes from a stream, this will loop forever
   * until the bytes are successfully read or timeout is reached
   */
  fn stream_bytes(&mut self, len: usize) -> Data;
}

// Limit by the static lifetime because Tokio spawning requires this
#[doc = include_str!("../docs/Making_a_Socket.md")]
pub trait KTSocket
{
  /*
   * The name we will use for the socket in runfs, which will end up
   * being /run/kickit/io.<SOCK_NAME>
   */
  const NAME: &'static str;

  // The socket permissions, default is 0o600 (only root can read/write)
  const OCTAL_PERMS: u32 = 0o600;

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
  fn handler(stream: UnixStream) -> impl Future<Output = ()> + Send;

  ///
  /// # Errors
  ///
  /// * Failed to shutdown the stream for whatever reason
  ///
  fn shutdown(stream: UnixStream) -> Result<(), KTErrorTrace>
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
pub trait Start: KTSocket + 'static
{
  fn start(&self) -> impl Future<Output = Result<(), KTErrorTrace>> + Send
  {
    async move
    {
      use std::{os::unix::{net::UnixListener, fs::PermissionsExt}, fs, fs::Permissions};
      use tokio::runtime::Runtime;

      // The socket gets mapped to a matching path with its real name field
      let path = &format!("/run/kickit/io.{}", Self::NAME) as &str;

      // Bind (start) our socket here
      let sock = UnixListener::bind(path).context_trace(path, KTError::RunFsFail)?;

      /*
       * thanks <https://users.rust-lang.org/t/how-to-manage-permissions-of-a-unixlistener/31039/8>
       * for having an answer for this it really hurt my head
       */
      fs::set_permissions(path, Permissions::from_mode(Self::OCTAL_PERMS))
        .context_trace(path, KTError::RunFsFail)?;

      // Our runtime for the sockets- should never be dropped
      let runner = Runtime::new()
                    .context_trace("Failed to start async runtime", KTError::Socket)?;

      for peer in (sock.incoming())
      {
        match (peer)
        {
          // Run the handler once a valid peer is found
          Ok(stream) => { runner.spawn(Self::handler(stream)); },
          // Usually this just means there are no more streams to read
          Err(..) => break
        }
      }
      Ok(())
    }
  }
}

// Blanket implementation
impl<S: KTSocket + 'static> Start for S {}

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
  pub const MASTER: u8 = 0x6C;
}

impl Power
{
  pub const SHUTDOWN: u8 = 0xF2;
  pub const REBOOT: u8 = 0x7E;
}

impl StreamBytes for UnixStream
{
  fn stream_bytes(&mut self, len: usize) -> Data
  {
    // Initialise our buffer that data will be read to
    let mut bytes = Data::new();

    loop {
      // Wait until <len> bytes appear in the stream & can be read
      if let Ok(r) = self.take(len as u64).read_to_end(&mut bytes) && (r == len)
      {
        return bytes
      }
    }
  }
}

impl KTSocket for Core
{
  const NAME: &'static str = "Core";
  // These permissions dictate that any user can read/write, but not execute
  const OCTAL_PERMS: u32 = 0o666;

  async fn handler(mut stream: UnixStream)
  {
    use crate::{state::state, init::target::TARGET_NAME};
    use std::process;

    macro_rules! fail
    {
      () =>
      {
        stream.write_all(&[0x0f]).trace(KTError::Socket).or_warn();
        // Cleanup stream for both input & output
        Self::shutdown(stream).or_warn();
        return
      }
    }

    match (stream.stream_bytes(1)[0])
    {
      Self::STATE => stream.write_all(&[state!() as u8]),
      // Add a newline byte, this is our EOL
      Self::VERSION => stream.write_all(&[crate::VERSION.to_string().as_bytes(), b"\n"].concat()),
      Self::TARGET =>
      {
        // Try open the OnceLock here
        if let Some(targetName) = TARGET_NAME.get()
        {
          stream.write_all(&[targetName.as_bytes(), b"\n"].concat())
        }
        else {
          // Error if target name cannot be reached for whatever reason
          fail!();
        }
      },
      Self::PID => stream.write_all(&process::id().to_be_bytes()),
      // Safely ignore newlines
      b'\n' => Ok(()),
      // Send an error for unknown bytes
      _ => { fail!(); }
    }
      .trace(KTError::Socket).or_warn();

    // Cleanup stream for both input & output- we are done here
    Self::shutdown(stream).or_warn();
  }
}

impl KTSocket for Log
{
  const NAME: &'static str = "Log";

  async fn handler(mut stream: UnixStream)
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
  const NAME: &'static str = "Power";

  async fn handler(mut stream: UnixStream)
  {
    use crate::{console::Colour, init::{POWER_LEVEL, PowerLevel, init_console::warn}};

    /*
     * None of these errors (OnceLock<u8>::set) carry any error
     * information- so we can just check it succeeded &
     * throw a generic error if not (in `.map_err(|_|)`)
     */
    if let Err(reason) = match (stream.stream_bytes(1)[0])
    {
      Self::SHUTDOWN => POWER_LEVEL.set(PowerLevel::Off)
                            .map_err(|_| String::from("Failed to set power level")),

      Self::REBOOT => POWER_LEVEL.set(PowerLevel::Reboot)
                            .map_err(|_| String::from("Failed to set power level")),

      // Write error byte to socket- unexpected input
      _ => stream.write_all(&[0x0f]).map_err(|e| e.to_string())
    }
    { warn!("io.Power: Internal socket error: {reason}") }

    // This might not execute because init will start shutdown at this point
    Self::shutdown(stream).or_warn();
  }
}
