//! I/O with init sockets

use crate::{Data, socket::Socket, socket, console::{ExtendWithSubject, ErrorResult}};
use super::console::{Error, ErrorTrace, StdResult, Result};

impl From<socket::Error> for ErrorTrace
{
  fn from(input: socket::Error) -> Self
  {
    Error::Socket.trace(input)
  }
}

// Interpret given response from the socket as Result enum
trait SocketResult<OkType>
{
  fn result(&self) -> Result<StdResult<OkType, socket::Error>>;
}

pub trait Request: Socket + Sized + Send + Sync
{
  // Expect the socket to give us a response
  fn request(self, input: u8) -> impl Future<Output = Result<StdResult<Data, socket::Error>>> + Send
  {
    use tokio::{net::UnixStream, io::AsyncReadExt};

    async move {
      // Determine where the socket is that we want to interact with
      let path = self.path();

      // We don't know how long the reply is going to be, so a Vec<u8> works best
      let mut out = Data::new();
      // Open a new connection to the socket, may fail if there is an existing connection
      let mut io = UnixStream::connect(&path).await.into_trace(Error::SocketAccessFail)
                                                      .subject(path.display())?;

      // Wait until the stream is ready to accept being written to
      io.writable().await.into_trace(Error::SocketAccessFail)?;
      io.try_write(&[input]).into_trace(Error::AccessRunFsFail)?;

      // ..and then wait for the stream to have a reply available
      io.readable().await.into_trace(Error::SocketAccessFail)?;
      io.read_to_end(&mut out).await.into_trace(Error::SocketAccessFail).subject("io.Core")?;

      out.result()
    }
  }
}

impl<S: AsRef<[u8]>> SocketResult<Vec<u8>> for S
{
  fn result(&self) -> Result<StdResult<Vec<u8>, socket::Error>>
  {
    let inner = self.as_ref();

    if (inner.len() < 2)
    {
      return Err(Error::SocketResponse.trace(format!("Expected at least 2 bytes, got {}", inner.len())))
    }

    match (inner[0])
    {
      socket::Error::OK => Ok(Ok(inner[1..].to_vec())),
      // Match the returned byte to its corresponding error variant
      socket::Error::ERR => Ok(Err(socket::Error::errorize(inner[1]).into_trace(Error::SocketResponse)?)),
      // This shouldn't ever happen unless there is a bug in the init itself
      bad => Err(Error::SocketResponse.trace(format!("Expected ok/err, got {bad}")))
    }
  }
}

// Blanket implementation
impl<S: Socket + Send + Sync> Request for S {}
