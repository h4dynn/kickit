//! Standardized service logging, with asynchronous piping or basic relaying

use std::{io, fs, sync::Arc};
use crate::{BoxedStr, AnyStr, init::console::prelude::*};

#[derive(Debug)]
pub struct Logger
{
  pub name: Arc<BoxedStr>,
  shout: bool,
  // The current line count
  line: usize,
  /// Matching log file
  pub(super) file: fs::File,
  /// The read pipe from the service process, set when `up` is ran
  pub(super) reader: Option<io::PipeReader>
}

/// Log entry to append to service logfile  with an identified source
/**
  * # Binary
  *
  * Init entries are additionally appended with a singular [`INIT_ENTRY`] byte to symbolize their origin.
  * The reader of the log file simply checks if the next byte past the timestamp (11 bytes) is the exact
  * match of the init entry marker. Padding with null bytes is additionally unnecessary, as the data
  * after the header's end is zstd-compressed and so will have fixed identifiable magic
  *
  *
  * [`INIT_ENTRY`]: #const.INIT_ENTRY
  */
#[derive(PartialEq, Eq, Clone, Debug)]
pub enum LogEntry<'entry>
{
  /// Regular entry from service's stderr & stdout [`std::io::PipeReader`]
  Service(&'entry str),
  /// Special entry from init
  Init(&'entry str)
}

impl Logger
{
  /// Empty zstd file, created from /dev/null (`$ zstd -1c < /dev/null | xxd`)
  // hex: [0x28, 0xb5, 0x2f, 0xfd, 0x24, 0, 0x01, 0, 0, 0x99, 0xe9, 0xd8, 0x51]
  pub const NULL_ZSTD: [u8; 13] = [40, 181, 47, 253, 36, 0, 1, 0, 0, 153, 233, 216, 81];

  #[must_use]
  pub const fn new(name: Arc<BoxedStr>, shout: bool, file: fs::File) -> Self
  {
    Self { name, shout, line: 0, file, reader: None }
  }

  /// Initialize a new logger instance for a service
  /**
    * # Errors
    *
    * This methods expects that the log file has not yet been created, and that the service directory
    * is accessible & writable
    */
  pub fn init(name: Arc<BoxedStr>, shout: bool /* append entry to init master log */) -> Result<Self>
  {
    use std::{fs::{File, Permissions}, io::Write, os::unix::fs::PermissionsExt};

    // Create the empty log file
    let mut log_file = File::options().read(true).write(true).create_new(true)
                        .open(path!("/run/kickit/service", &**name, "log"))
                        .into_trace(Error::ServiceAccess).subject(&name)?;

    // Make log file inaccessible to anybody but the root user
    log_file.set_permissions(Permissions::from_mode(0o600)).into_trace(Error::ServiceAccess)?;

    /*
     * The log file is going to be decompressed later on to view its contents, the decompressor will fail
     * if the file is empty so this provides some valid but empty zstd data
     */
    log_file.write_all(&Self::NULL_ZSTD).into_trace(Error::ServiceAccess)?;

    Ok(Self::new(name, shout, log_file))
  }

  /// Watch the reader for the process for updates & then send them to the service's log
  /**
    * # Implementation
    *
    * This function runs in the following order:
    *
    * 1. The `reader` instance ([`PipeReader`]) is taken. This is the stream responsible for piping
    *    `stderr` and `stdout` output into a singular read-only stream,
    *
    * 2. Block until a new entry is received from the reader instance (see [`read_until`]),
    *
    * 3. The new entry is appended to the service's log file in its binary format (see [`log`]), with
    *    current timestamp (Unix epoch, resolved with `chrono` crate) and compressed with ZSTD.
    *
    * # Errors
    *
    * This method calls for 2 conditions to be met:
    *
    * 1. This logger is asynchronous (for standard services, differentiating factor is the reader),
    *
    * 2. The service is up & running (handled by supervisor, required for the reader endpoint).
    *
    *
    * [`PipeReader`]: std::io::PipeReader
    * [`read_until`]: std::io::BufRead::read_until
    * [`log`]: #method.log
    */
  pub fn watch(&mut self) -> Result<()>
  {
    use std::{io::{BufRead, BufReader}, mem::take};

    // Standard pipe reader for relaying log output
    let log_ro = self.reader.take().ok_or(Error::Unknown.trace("logger missing").subject(&*self.name))?;
    // Using a `BufReader` here is more efficient, as we read line-by-line (with `read_until`)
    let mut log_buf_ro = BufReader::new(log_ro);

    // Buffer of all the (hopefully) UTF-8 bytes this service will provide
    let mut received = Vec::new();

    loop {
      // Read until newlines, capturing contents line-by-line
      log_buf_ro.read_until(b'\n', &mut received).into_trace(Error::ServiceLogContent)?;
      // Convert from raw UTF-8 bytes to a string
      let relay = String::from_utf8(take(&mut received)).into_trace(Error::ServiceLogContent)?;

      for line in (relay.lines())
      {
        // And finally add the logs from the service!
        self.log(&LogEntry::Service(line))?;
      }
    }
  }

  /// Append a new line to the service's log
  /**
    * # Errors
    *
    * * Couldn't open the service's logfile
    *
    * * Couldn't compress or decompress the service's logfile
    *
    * * Couldn't open a lock on the file (or unlock)
    *
    * * Couldn't write to the logfile
    *
    * # Panics
    *
    * * More than one line was provided to the function
    */
  // TO-DO: current method is to decompress & then recompress, which is kinda inefficient
  pub fn log(&mut self, entry: &LogEntry<'_>) -> Result<()>
  {
    use std::{io::{Read, Seek, Cursor}, time::{SystemTime, UNIX_EPOCH}};
    use crate::{state::state, init::console::{LogEntryType::SERVICE_RELAY, log}};
    use ruzstd::{decoding::StreamingDecoder, encoding::{compress, CompressionLevel}};

    let (new, from_init) = (entry.value(), matches!(entry, LogEntry::Init(_)));

    // Don't send empty lines if they are found for whatever reason
    if (new.as_str_ref().is_empty())
    {
      return Ok(())
    }

    /*
     * We cannot have more than 1 line of content, this function is designed to take a-line-at-a-time
     * (use `for line in input.split('\n') { ... }`). This is a panic & not an error because it is solely
     * down to how the program is coded, not user error
     */
    assert!(new.as_str_ref().split('\n').count() <= 1, "log() was given more than 1 line of input!");

    /*
     * Because this function will be called multiple times, changing the seek on the log file, we need to
     * rewind to the beginning of the file
     */
    self.file.rewind().into_trace(Error::ServiceLogContent)?;

    // Get current timestamp in UTC
    let time = SystemTime::now().duration_since(UNIX_EPOCH).into_trace(Error::Time)?.as_millis().to_string();

    // The decoder wraps the file's contents & decompresses on read (lazy type)
    let mut decoder = StreamingDecoder::new(&self.file).into_trace(Error::ServiceLogCompress)?;
    // Log entry contents to be written to file
    let mut out = Vec::new();

    // Decompress previous log contents into output vector
    decoder.read_to_end(&mut out).into_trace(Error::ServiceLogCompress)?;

    // Send timestamp to log
    out.extend_from_slice(time.as_bytes());

    // This byte is how ktctl can differentiate messages from init & service
    if (from_init)
    {
      out.push(LogEntry::INIT_ENTRY);
    }

    // Addon the log contents
    out.extend_from_slice(new.as_str_ref().as_bytes());
    // Add a newline
    out.push(b'\n');

    /*
     * Make sure we are not stalled, because if we are we will interrupt the user shell with our message, and
     * then check this message isn't from the init because if so it will have already been reported once
     */
    if (state!().is_ok() && !from_init && self.shout)
    {
      for line in (new.as_str_ref().split('\n'))
      {
        log!(format!("{} {} {}", SERVICE_RELAY, Colourize::bold(format!("({}): ", self.name)), line));
      }
    }

    /*
     * Reading from the logfile changes the cursor position so we set it back to the beginning to overwrite
     * the file instead of appending it
     */
    self.file.rewind().into_trace(Error::ServiceLogContent)?;

    // Compress the newly formed contents, saving to the log file
    compress(Cursor::new(out), &mut self.file, CompressionLevel::Fastest);
    self.line += 1;

    Ok(())
  }
}

impl<'entry> LogEntry<'entry>
{
  /// Marker added to entries to show the message is from the init system
  pub const INIT_ENTRY: u8 = 0x8f;

  /// Strip down to inner value, discarding init entry status
  #[must_use]
  pub fn value(&self) -> &'entry str
  {
    match (self)
    {
      Self::Service(val) | Self::Init(val) => val
    }
  }
}

