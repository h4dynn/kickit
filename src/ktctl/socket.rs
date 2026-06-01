//! I/O with init sockets

use crate::{Data, socket::Socket, console::ExtendWithContext};
use super::console::{Error, Result, ConvError};

pub trait Request: Socket + Sized + Send + Sync
{
  fn request(self, input: u8) -> impl Future<Output = Result<Data>> + Send
  {
    async move
    {
      use tokio::{net::UnixStream, io::AsyncReadExt};

      // Determine where the socket is that we want to interact with
      let path = self.path();

      // We don't know how long the reply is going to be, so a Vec<u8> works best
      let mut out = Data::new();
      // Open a new connection to the socket, may fail if there is an existing connection
      let mut io = UnixStream::connect(&path).await.into_trace(Error::SocketAccessFail)
                                                      .context(path.display())?;

      // Wait until the stream is ready to accept being written to
      io.writable().await.into_trace(Error::SocketAccessFail)?;
      io.try_write(&[input]).into_trace(Error::AccessRunFsFail)?;

      // ..and then wait for the stream to have a reply available
      io.readable().await.into_trace(Error::SocketAccessFail)?;
      io.read_to_end(&mut out).await.into_trace(Error::SocketAccessFail).context("io.Core")?;

      // An 0x0f byte means the operation failed
      if (out.as_slice() == [0x0f])
      {
        Err(Error::SocketAccessFail.trace(format!("Socket error after requesting {}", Self::NAME)))
      }
      else {
        Ok(out)
      }
    }
  }
}

// Blanket implementation
impl<S: Socket + Send + Sync> Request for S {}
