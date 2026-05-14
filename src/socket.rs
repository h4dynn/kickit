//! Sockets for the runfs

use std::path::PathBuf;
use tokio::net::UnixStream;
use crate::init::init_console::{Error, ExtendWithContext, ErrorResult, Result};

#[macro_export]
macro_rules! socket_struct
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

// The sockets- all of these will have Socket implemented
socket_struct!
{
  pub Core, pub Log, pub Power
}

// Limit by the static lifetime because Tokio spawning requires this
#[doc = include_str!("../docs/Making_a_Socket.md")]
pub trait Socket
{
  /*
   * The name we will use for the socket in runfs, which will end up
   * being /run/kickit/io.<SOCK_NAME>
   */
  const NAME: &str;

  // Make the socket root-access only (mapped to private dir)
  const PRIVATE: bool = true;

  // You may implement a custom method for a different path
  fn path(&self) -> PathBuf
  {
    use crate::file_path;
    // These are the default paths, used except for when a custom method is defined
    if (Self::PRIVATE)
    {
      file_path!(PathBuf::from("/run/kickit/private"), "io", Self::NAME)
    }
    else
    {
      file_path!(PathBuf::from("/run/kickit"), "io", Self::NAME)
    }
  }

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
  fn handler(&'static self, stream: UnixStream) -> impl Future<Output = ()> + Send;
}

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
      let permissions = Permissions::from_mode(
      {
        if (Self::PRIVATE)
        {
          0o600
        }
        else {
          0o666
        }
      });

      let sock = UnixSocket::new_stream().into_trace(Error::Socket)?;
      // Bind (start) our socket here
      sock.bind(&path).into_trace(Error::RunFsFail).context(path.display())?;

      /*
       * thanks <https://users.rust-lang.org/t/how-to-manage-permissions-of-a-unixlistener/31039/8>
       * for having an answer for this it really hurt my head
       */
      set_permissions(&path, permissions).into_trace(Error::RunFsFail).context(path.display())?;

      let listener = sock.listen(1).into_trace(Error::Socket)?;

      while let Ok((stream, _)) = listener.accept().await
      {
        self.handler(stream).await;
      }
      Ok(())
    }
  }
}

// Blanket implementation
impl<S: Socket + Sync + 'static> Open for S {}

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
