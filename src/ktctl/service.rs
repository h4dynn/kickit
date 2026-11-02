//! Service access from runfs

use std::{fs, fs::File, boxed::Box, io, path::PathBuf};
use crate::{ktctl::console::prelude::*, init::service::{Pattern, Pattern::Standard}, Data};

// This is only partial because we don't know everything about it
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct PartialService
{
  // This is the only field you must provide, in ktctl this is provided by indexing /run/kickit/service
  name: Box<str>,
  // These are imported from the service's cache
  pattern: Pattern,
  sandboxed: bool,
  status: Option<i32>,
  pid: u32
}

#[derive(PartialEq, Eq, Copy, Clone, Debug)]
struct CacheInstance
{
  pattern: u8,
  sandboxed: u8,
  pid: u32,
  status: Option<i32>
}

pub struct Delimited<B>
{
  // What our items are seperated with
  delimiter: u8,
  buf: B
}

pub trait Delimit: Sized + io::BufRead
{
  // Create a delimited iterator from this instance
  fn delimit(self, delimiter: u8) -> Delimited<Self>;
}

impl<B: io::BufRead> Iterator for Delimited<B>
{
  type Item = Data;

  // This code is adapted from the standard library's `Lines<B>`, see `std/io/mod.rs`:3140
  fn next(&mut self) -> Option<Data>
  {
    let mut buf = Data::new();

    if (self.buf.read_until(self.delimiter, &mut buf).unwrap() == 0)
    {
      return None
    }
    else if (buf.last() == Some(&self.delimiter))
    {
      // Remove the delimiter at the end of the buffer
      buf.pop();
    }
    Some(buf)
  }
}

impl<B: io::BufRead> Delimit for B
{
  fn delimit(self, delimiter: u8) -> Delimited<Self>
  {
    Delimited { delimiter, buf: self }
  }
}

impl CacheInstance
{
  // Read 3 seperate bytes from cache, followed by 4 bytes of LE u32 bytes (pid)
  pub(super) fn new(mut source: impl io::Read) -> Result<Self>
  {
    /*
     * bytes => 2 bytes all of different types (pattern -> sandboxed),
     * pidRaw => PID in u32 LE byte order
     */
    let (mut bytes, mut pid_raw, mut status_switch) = ([0u8; 2], [0u8; 4], [0u8; 1]);

    source.read_exact(&mut bytes).into_trace(Error::ServiceConfig)?;
    // PID u32 bytes in little-endian order
    source.read_exact(&mut pid_raw).into_trace(Error::ServiceConfig)?;
    source.read_exact(&mut status_switch).into_trace(Error::ServiceConfig)?;

    // Check the option switch to see if there is a status available for us
    let status = (status_switch[0] == 1).then_some(
    {
      // Exit status of this service in i32 LE bytes
      let mut status = [0u8; 4];
      source.read_exact(&mut status).into_trace(Error::ServiceConfig)?;
      i32::from_le_bytes(status)
    });

    Ok(Self { pattern: bytes[0], sandboxed: bytes[1], pid: u32::from_le_bytes(pid_raw), status })
  }

  pub(super) fn export(self) -> Result<(Pattern, bool, u32, Option<i32>)>
  {
    // Must be one of the 3 patterns available, see `init::service::Pattern` for their respective bytes
    let pattern = <Pattern as TryFrom<u8>>::try_from(self.pattern).into_trace(Error::ServiceConfig)?;
    // This is just a bool so will be 0 for false, 1 for true
    let sandboxed: bool = self.sandboxed.try_into().into_trace(Error::ServiceConfig)?;

    Ok((pattern, sandboxed, self.pid, self.status ))
  }
}

impl PartialService
{
  #[inline]
  #[must_use]
  pub fn path(&self, path: &str) -> PathBuf
  {
    if (path == "stat")
    {
      PathBuf::from(format!("/proc/{}/stat", self.pid))
    }
    else {
      path!("/run/kickit/service", self.name.as_ref(), path)
    }
  }

  /**
    * # Errors
    *
    * * Service does not exist for this current init session,
    * * Failed to open the service's cached configuration file,
    * * Failed to export cache configuration
    */
  pub fn import(name: &str) -> Result<Self>
  {
    // Make sure the requested service actually exists for this init session
    guard!(!path!("/run/kickit/service", name).is_dir() =>
            Error::BadService.trace("No such file or directory").subject(name));

    // Open the cached config file
    let cache_file = File::open(path!("/run/kickit/service", name, "config"))
                        .into_trace(Error::BadService).subject(name)?;
    let cache = CacheInstance::new(cache_file).subject(name)?;

    // Convert values from raw byte to their types
    let (pattern, sandboxed, pid, status) = cache.export().subject(name)?;

    Ok(Self { name: name.into(), pattern, sandboxed, status, pid })
  }

  /**
    * # Errors
    *
    * * Failed to get the proccess's stat path
    *
    * * Failed to get the service's PID
    *
    * # Panics
    *
    * * `Service::path` failed to get the stat file (should never happen)
    */
  pub fn print(self) -> Result<()>
  {
    println!("{}", (&self.name).bold());
    println!("├─ Status:    {}",
    {
      if (self.pattern == Standard)
      {
        // Read the /proc/<PID>/stat file, which contains the process's status information
        if let Ok(stat) = fs::read_to_string(self.path("stat"))
        {
          // The 3rd member (split by spaces) contains the status we want (e.g. I = idle)
          match (stat.split(' ').nth(2))
          {
            // Z = zombie (stopped running) and X = killed (by another process)
            Some("Z" | "X") => "Dead".colour(Colour::Red),
            // These are all acceptable process statuses
            Some("S" | "I" | "D" | "R") => "Up".colour(Colour::Green),
            /*
             * This might happen sometimes, for example on older kernel versions which
             * may have additional signals which are now removed / deprecated
             */
            Some(..) | None => "Unknown".colour(Colour::Red)
          }
        }
        else {
          "Dead".colour(Colour::Red)
        }
      }
      else if let Some(status) = self.status && (status > 0)
      {
        format!("Failed (exit code {status})").colour(Colour::Red)
      }
      // Service is non-standard & has finished successfully
      else if (self.status == Some(0))
      {
        "Finished".colour(Colour::Green)
      }
      else {
        "Failed".colour(Colour::Red)
      }
    });

    if (self.sandboxed)
    {
      let branch = tern! { self.pattern == Standard => "├─", _ => "└─" };
      println!("{} Sandboxed: {}", branch, "yes".colour(Colour::Green));
    }

    // Non-standard services will not have an active PID
    if (self.pattern == Standard)
    {
      println!("└─ PID:       {}", self.pid);
    }

    // Print newline seperator for next service
    println!();

    Ok(())
  }

  /**
    * # Errors
    *
    * * Service's log file is not accessible (this should never be the case)
    *
    * * Encountered log entry with an invalid timestamp (should be in Unix millis)
    *
    * * Timestamp parsing failure in [`from_timestamp_millis`]
    *
    * * `stdout` console is not writable (TTY error, not kickit related)
    *
    * [`from_timestamp_millis`]: chrono::DateTime::from_timestamp_millis
    */
  pub fn logs(&self, ugly: bool, ignore_init: bool) -> Result<()>
  {
    use std::{io, io::{BufReader, Write, ErrorKind::BrokenPipe}, process::exit};
    use crate::{DumpVec, DequeData, init::service::logger::LogEntry};
    use ruzstd::decoding::StreamingDecoder;
    use chrono::{Local, DateTime};

    // Open the zstd compressed log file
    let file = File::open(self.path("log")).into_trace(Error::BadService).subject(&self.name)?;
    // The decoder implements Read to idiomatically decompress
    let decoder = StreamingDecoder::new(file).into_trace(Error::LogAccessFail).subject(&self.name)?;
    // Use `BufReader` for the handy `read_until` method
    let log = BufReader::new(decoder);

    // Our stdout that we write to
    let mut out = io::stdout().lock();

    /*
     * We don't uses the `lines()` method here because that creates a String but our entries contain binary data
     * so we need a vector of bytes instead
     */
    for mut entry in (log.delimit(b'\n').map(|mut v| <DequeData as From<Data>>::from(std::mem::take(&mut v))))
    {
      // First 13 bytes will always be the timestamp
      let timestamp_raw: [u8; 13] = entry.dump_front();
      // The timestamp will be in a String
      let ts = &str::from_utf8(timestamp_raw.as_slice()).into_trace(Error::Format)?;

      let from_init = entry[0] == LogEntry::INIT_ENTRY;

      if (from_init)
      {
        // Remove this byte now that we have parsed it
        entry.pop_front();
      }

      continueif! (from_init && ignore_init);

      let marker = from_init.then_some(tern! { ugly => "(kickit) ", _ => "\x1b[1m(kickit) \x1b[0m" })
                      .unwrap_or_default();

      // `entry` isn't used after this, so this resets it for the next iteration
      let contents = String::from_utf8(std::mem::take(&mut entry).into()).into_trace(Error::Format)?;

      let line = tern!
      {
        ugly => format!("[{ts}] {marker}{contents}\n"),
        _ => {
          // Convert the millis type from a &str to i64 so it is accepted by chrono
          let timestamp: i64 = ts.parse().into_trace(Error::Format)?;
          /*
           * Get the timestamp from the log and convert it into an actual date & time, then convert from
           * UTC timezone to the system's timzone (Local) using the chrono crate magic
           */
          let ts_local: DateTime<Local> = DateTime::from_timestamp_millis(timestamp)
                                          .ok_or(Error::Time.trace("invalid timestamp while appending log entry"))
                                          .subject(&self.name)?
                                          .into();
          // Format time as <Day Month Year, Hours:Minutes:Seconds> to not anger the Americans
          format!("[{}] {marker}{contents}\n", ts_local.format("%d %b %Y, %H:%M:%S"))
        }
      };

      /*
       * When we output to stdout and another program is piped (e.g. grep), the program cuts off the pipe
       * after it has consumed everything it needs so we end up with SIGPIPE. We don't error here if found,
       * since this is expected & not an actual error, though is treated as one by `println!` so we have to
       * do a janky workaround for compatibility
       *
       * (note): This may change in the future though, see <https://github.com/rust-lang/rust/issues/62569>
       */
      out.write_all(line.as_bytes())
            .inspect_err(|e| if e.kind() == BrokenPipe { exit(0) } /* exit OK on pipe cut off */)
            .into_trace(Error::Format)?;
    }
    Ok(())
  }
}

/**
  * # Errors
  *
  * * Failed to open the service from its name (see `Service::From::<&str>::from`),
  * * Failed to print the service's details (pid & status)
  */
pub fn serviceList(maybeTargets: Option<&[String]>) -> Result<()>
{
  for maybeService in (fs::read_dir(PathBuf::from("/run/kickit/service")).into_trace(Error::RunFsParseFail)?)
  {
    let service = maybeService.into_trace(Error::BadService)?.file_name();
    // Import each entry as a service from its OsString file name
    let upService = PartialService::import(service.display().to_string().as_str())?;
    let name = &*upService.name;

    // Only print if targets provided by user allow
    if (maybeTargets.is_none_or(|targets| targets.contains(&name.into())))
    {
      upService.print()?;
    }
  }

  Ok(())
}
