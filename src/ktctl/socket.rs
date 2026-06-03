//! I/O with init sockets

use crate::{Data, socket::{Socket, PeerError}, console::ExtendWithContext};
use super::console::{Error, ErrorTrace, Result, ConvError};

impl From<PeerError> for ErrorTrace
{
  fn from(input: PeerError) -> Self
  {
    Error::SocketAccessFail.trace(input)
  }
}

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

      // If we have an error then only one byte will be provided
      if (out.len() == 1)
      {
        // Make sure this singular byte isn't an error
        PeerError::errorize(out[0]).map(|ok| vec![ok]).map_err(ErrorTrace::from)
      }
      else {
        // We're all good!
        Ok(out)
      }
    }
  }
}

// Blanket implementation
impl<S: Socket + Send + Sync> Request for S {}
