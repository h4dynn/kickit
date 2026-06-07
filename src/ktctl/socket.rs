//! I/O with init sockets

use crate::{Data, socket::{Socket, PeerError}, console::{ExtendWithContext, ErrorResult}};
use super::console::{Error, ErrorTrace, StdResult, Result};

impl From<PeerError> for ErrorTrace
{
  fn from(input: PeerError) -> Self
  {
    Error::Socket.trace(input)
  }
}

trait SocketResult<OkType>
{
  fn result(&self) -> Result<StdResult<OkType, PeerError>>;
}

pub trait Request: Socket + Sized + Send + Sync
{
  // Expect the socket to give us a response
  fn request(self, input: u8) -> impl Future<Output = Result<StdResult<Data, PeerError>>> + Send
  {
    use tokio::{net::UnixStream, io::AsyncReadExt};

    async move {
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

      out.result()
    }
  }
}

impl<S: AsRef<[u8]>> SocketResult<Vec<u8>> for S
{
  fn result(&self) -> Result<StdResult<Vec<u8>, PeerError>>
  {
    let inner = self.as_ref();

    if (inner.len() < 2)
    {
      return Err(Error::SocketResponse.trace(format!("Expected at least 2 bytes, got {}", inner.len())))
    }

    match (inner[0])
    {
      PeerError::IS_OK => Ok(Ok(inner[1..].to_vec())),
      PeerError::IS_ERROR => Ok(Err(PeerError::errorize(inner[1]).into_trace(Error::SocketResponse)?)),
      bad => Err(Error::SocketResponse.trace(format!("Expected ok/err, got {bad}")))
    }
  }
}

// Blanket implementation
impl<S: Socket + Send + Sync> Request for S {}
