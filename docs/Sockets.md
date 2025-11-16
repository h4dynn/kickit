# kickit sockets

## (0): The socket itself

Say you want to create a socket that will finish your "Hello world"
when given a partial input

First, create a file called `src/hello_sock.rs`:

```rust
//! hello_sock.rs
/// SPDX-License-Identifier: CC-PDDC
/// This code is released under the public domain, no rights reserved

use std::os::unix::net::UnixStream;
use crate::{socket::KTSocket, init::init_console::KTError, console::HandleKTError};

// Initialise our socket structure here
pub struct Hello;

// The implementation for how our socket works
impl KTSocket for Hello
{
  /*
   * This dictates what name our socket will be created with, so
   * in this case it would be '/run/kickit/io.Hello'
   */
  const REAL_NAME: &'static str = "Hello";

  /*
   * Who is allowed to access/modify our socket, make sure to not
   * include execute permissions. In this case, 666 allows
   * everybody to read & write our socket. If you set this to
   * None and provide no octal permissions, the default of
   * 600 will be used (only root has read/write access)
   */
  const OCTAL_PERMS: Option<u32> = Some(0o666);

  // How we handle input & what output we give
  async fn handler(mut stream: UnixStream)
  {
    use std::io::Write;
    // This method allows us to read our input bytes easily
    use crate::{socket::StreamBytes, init::init_console::ConvKTError};

    // Get just one byte and represent it as a character
    match (stream.stream_bytes(1)[0] as char)
    {
      // Our valid input, we can finish the input off here
      'H' | 'h' => stream.write_all("ello world\n".as_bytes()),
      // Invalid, so write an error
      _ => stream.write_all("unexpected input\n".as_bytes())
    }
      /*
       * Notice the '.or_warn()' method here. Do not use fatal methods,
       * you should always keep your sockets non-fatal
       */
      .trace(KTError::SocketFail).or_warn();

    // Finish off here
    Self::shutdown(stream).or_warn();
  }
}
```

The above code defines a socket that finishes its input when
given a "h", and is compatible with kickit.

To make sure this code is recognised by the project, add it
as a mod in `src/lib.rs`:

```rust
pub mod hello_sock;
```

## (1): Initialising the socket

Open `src/init/kickit.rs`, this is where we will call our
socket to be started.

In the `main()` function, insert code to start your socket,
e.g.:

```rust
use kickit::hello_sock;
tokio::task::spawn(async move { hello_sock::Hello.open().await.handle(); });
```

Now if you compile `kickit` and run it, you should have
your socket ready to be used!

## (2): Interacting with the socket

In our example code, if the socket is provided with a 'H'
character, it will auto-fill it with -"ello world".

So, if we try to do so in a shell using `netcat`, we
should get that as an output:

```sh
$ nc -nU /run/kickit/io.Hello <<< 'h'
ello world
Ncat: Connection reset by peer.
```

You can safely ignore the warning message from `netcat`.
This happens because our socket is only designed to take
1 byte and shutdown the connection immediately after.
