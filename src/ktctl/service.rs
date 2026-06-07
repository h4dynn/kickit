//! Service access from runfs

use std::{fs, fs::File, boxed::Box, io, path::PathBuf};
use super::{console::{Result, Error}};
use crate::{console::{Colour, ExtendWithContext, ErrorResult}, guard, path,
              init::service::{Pattern, Pattern::Standard}};

// This is only partial because we don't know everything about it
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct PartialService
{
  // This is the only field you must provide, in ktctl this is provided by indexing /run/kickit/service
  name: Box<str>,
  // These are imported from the service's cache
  pattern: crate::init::service::Pattern,
  sandboxed: bool,
  status: Option<i32>,
  pid: u32
}

#[derive(PartialEq, Eq, Copy, Clone, Debug)]
struct CacheInstance(u8, u8, /* option switch */ u8, /* (optional) i32 LE bytes */ [u8; 4], /* u32 LE bytes */ [u8; 4]);

impl CacheInstance
{
  // Read 3 seperate bytes from cache, followed by 4 bytes of LE u32 bytes (pid)
  pub(super) fn new(mut source: impl io::Read) -> Result<Self>
  {
    // 3 bytes of different types (pattern -> sandboxed -> option switch)
    let mut bytes = [0u8; 3];
    source.read_exact(&mut bytes).into_trace(Error::ServiceConfig)?;

    // If bytes[2] is set to 1, a i32 status is available but if not, none is available (just 4 null bytes for padding)
    let mut maybeStatus = [0u8; 4];
    source.read_exact(&mut maybeStatus).into_trace(Error::ServiceConfig)?;

    // PID u32 bytes in little-endian order
    let mut pidRaw = [0u8; 4];
    source.read_exact(&mut pidRaw).into_trace(Error::ServiceConfig)?;

    Ok(Self(bytes[0], bytes[1], bytes[2], maybeStatus, pidRaw))
  }

  pub(super) fn export(self) -> Result<(Pattern, bool, Option<i32>, u32)>
  {
    // Must be one of the 3 patterns available, see `init::service::Pattern` for their respective bytes
    let pattern = <Pattern as TryFrom<u8>>::try_from(self.0).into_trace(Error::ServiceConfig)?;
    // This is just a bool so will be 0 for false, 1 for true
    let sandboxed: bool = self.1.try_into().into_trace(Error::ServiceConfig)?;
    // Option switch - if there is a i32 available for us (status) then its 1, if not 0
    let switch: bool = self.2.try_into().into_trace(Error::ServiceConfig)?;

    Ok((pattern, sandboxed, switch.then_some(i32::from_le_bytes(self.3)), u32::from_le_bytes(self.4)))
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
    guard!(!path!("/run/kickit/service", name).is_dir() => Error::BadService.trace("No such file or directory").context(name));

    // Open the cached config file
    let cacheFile = File::open(path!("/run/kickit/service", name, "config")).into_trace(Error::BadService).context(name)?;
    let cache = CacheInstance::new(cacheFile).context(name)?;

    // Convert values from raw byte to their types
    let (pattern, sandboxed, status, pid) = cache.export().context(name)?;

    Ok(Self { name: name.into(), pattern, sandboxed, status, pid })
  }

  /**
    * # Errors
    *
    * * Failed to get the proccess's stat path,
    * * Failed to get the service's PID
    *
    * # Panics
    *
    * * `Service::path` failed to get the stat file (should never happen)
    */
  pub fn print(self) -> Result<()>
  {
    use crate::tern;

    println!("{}{}{}", Colour::BOLD, self.name, Colour::RESET);
    println!("├─ Status:    {}", tern!
    {
      self.pattern == Standard =>
      {
        // Read the /proc/<PID>/stat file, which contains the process's status information
        if let Ok(stat) = fs::read_to_string(self.path("stat"))
        {
          // The 3rd member (split by spaces) contains the status we want (e.g. I = idle)
          match (stat.split(' ').nth(2))
          {
            // Z = zombie (stopped running) and X = killed (by another process)
            Some("Z" | "X") => format!("{}Dead{}", Colour::RED, Colour::RESET),
            // These are all acceptable process statuses
            Some("S" | "I" | "D" | "R") => format!("{}Up{}", Colour::GREEN, Colour::RESET),
            /*
             * This might happen sometimes, for example on older kernel versions which
             * may have additional signals which are now removed / deprecated
             */
            Some(..) | None => format!("{}Unknown{}", Colour::RED, Colour::RESET)
          }
        }
        else {
          format!("{}Dead{}", Colour::RED, Colour::RESET)
        }
      },
      // Service is non-standard & has finished successfully
      self.status == Some(0) => format!("{}Finished{}", Colour::GREEN, Colour::RESET),
      else =>
      {
        if let Some(code) = self.status
        {
          format!("{}Failed{} (exit code {code})", Colour::RED, Colour::RESET)
        }
        else {
          format!("{}Failed{}", Colour::RED, Colour::RESET)
        }
      }
    });

    if (self.sandboxed)
    {
      println!("{} Sandboxed: {}yes{}", tern! { self.pattern == Standard => "├─", else => "└─" }, Colour::GREEN, Colour::RESET);
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
    * * Failed to open the service's log file,
    * * Failed to read from the log file,
    * * Failed to parse i64 timestamp from String,
    * * `chrono::DateTime::from_timestamp_millis` failed to parse the timestamp,
    * * Failed to write the log's contents to stdout,
    * * Unexpected init marker in wrong place on log line
    */
  pub fn readLog(&self, ugly: bool, ignoreInit: bool) -> Result<()>
  {
    use std::{io, io::{BufReader, BufRead, Write, ErrorKind::BrokenPipe}, collections::VecDeque, mem::take, process::exit};
    use crate::{tern, breakif, DumpVec, init::service::INIT_LOG_ENTRY};
    use ruzstd::decoding::StreamingDecoder;
    use chrono::{Local, DateTime};

    // We can't use `Colour::BOLD` in concat!() because it only accepts literals
    const KICKIT_MARKER: &str = concat!("\x1b[1m", "(kickit) ", "\x1b[0m");

    let file = File::open(self.path("log")).into_trace(Error::BadService).context(&self.name)?;

    // The decoder implements Read to idiomatically decompress
    let decoder = StreamingDecoder::new(file).into_trace(Error::LogAccessFail).context(&self.name)?;
    // Use `BufReader` for the handy `read_until` method
    let mut log = BufReader::new(decoder);

    // Our stdout that we write to
    let mut out = io::stdout();
    // Binary contents for the current entry
    let mut vecEntry = Vec::<u8>::new();

    while (log.read_until(b'\n', &mut vecEntry).is_ok())
    {
      // The last entry we get will be empty due to how we are iterating
      breakif! (vecEntry.is_empty());

      // We are removing from the front, which makes a VecDeque more suitable
      let mut entry = VecDeque::<u8>::from(take(&mut vecEntry));

      // First 13 bytes will be the timestamp
      let timestampBytes: [u8; 13] = entry.front_dump();
      // The timestamp will be in a String
      let timestampString = &str::from_utf8(&timestampBytes).into_trace(Error::Format)?;

      let fromInit = tern!
      {
        entry[0] == INIT_LOG_ENTRY =>
        {
          // Removing this byte should give us just UTF-8 string bytes
          let _ = entry.pop_front();
          true
        },
        else => false
      };

      if (fromInit && ignoreInit)
      {
        entry.clear();
        continue
      }

      // If this entry is from the init, 
      let marker = fromInit.then_some(tern! { ugly => "(kickit) ", else => KICKIT_MARKER }).unwrap_or_default();

      // `entry` isn't used after this, so this resets it for the next iteration
      let contents = String::from_utf8(take(&mut entry).into()).into_trace(Error::Format)?;

      let line = if (ugly)
      {
        format!("[{timestampString}] {marker}{contents}")
      }
      else {
        // Convert the millis type from a &str to i64 so it is accepted by chrono
        let timestamp = timestampString.parse::<i64>().into_trace(Error::Format)?;
        /*
         * Get the timestamp from the log and convert it into an actual date & time,
         * then convert from UTC timezone to the system's timzone (Local) using
         * the chrono crate magic
         */
        let logTime: DateTime<Local> = DateTime::from_timestamp_millis(timestamp)
                                        .ok_or(Error::Time.trace("Failed to convert timestamp from service log"))
                                        .context(&self.name)?
                                        .into();
        // Format time as <Day Month Year, Hours:Minutes:Seconds> to not anger the Americans
        format!("[{}] {marker}{contents}", logTime.format("%d %b %Y, %H:%M:%S"))
      };

      /*
       * When we output to stdout and another program is piped (e.g. grep),
       * the program cuts off the pipe after it has consumed everything it
       * needs so we end up with SIGPIPE. We don't error here if found, since
       * this is expected & not an actual error, though is treated as one
       * by println!() so we have to do a janky workaround for compatibility
       *
       * (note): This may change in the future though, see
       * <https://github.com/rust-lang/rust/issues/62569>
       */
      out.write_all(line.as_bytes()).map_err(|e| if (e.kind() == BrokenPipe) { exit(0) } else { e })
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
    if let Some(targets) = maybeTargets
    {
      if (targets.contains(&name.into()))
      {
        upService.print()?;
      }
    }
    else {
      // Print all the services
      upService.print()?;
    }
  }

  Ok(())
}

// TO-DO: not sure how to implement this yet? maybe a socket?
/*fn serviceRestart(_service: String) -> Result<()>
{
  todo!();
}*/
