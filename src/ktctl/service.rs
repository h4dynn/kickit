//! Service access from runfs

use std::{fs, fs::File, ffi::OsString, path::PathBuf};
use super::{console::{Result, Error, ConvError}};
use crate::{console::{Colour, ExtendWithContext}, affirm, path};

// Lookup service from the runfs directory
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct Service
{
  name: String
}

impl From<OsString> for Service
{
  // Display our OsString first and then use that Display to be converted to a String
  fn from(name: OsString) -> Self
  {
    Self { name: name.display().to_string() }
  }
}

/*
 * (slight annoyance): we can't use a generic AsRef<str> because of way too strict
 * Rust compilation rules:
 *   upstream crates may add a new impl of trait `std::convert::AsRef<str>` for type
 *   `std::ffi::OsString` in future versions
 */
impl From<&str> for Service
{
  fn from(name: &str) -> Self
  {
    Self { name: name.to_owned() }
  }
}

impl Service
{
  #[must_use]
  pub fn is_standard(&self) -> bool
  {
    fs::metadata(path!("/run/kickit/service/", &self.name, "pid")).is_ok()
  }

  #[must_use]
  pub fn is_sandboxed(&self) -> bool
  {
    path!("/run/kickit/service/", &self.name, "container").is_dir()
  }

  /**
    * # Errors
    *
    * * Failed to read from the service's PID file,
    * * Received incorrect length of bytes from service's PID file (should be 4 bytes)
    */
  pub fn pid(&self) -> Result<Option<u32>>
  {
    if (self.is_standard())
    {
      let rawPid = fs::read(path!("/run/kickit/service", &self.name, "pid"))
                      .into_trace(Error::AccessRunFsFail)?;

      // Read the PID in little-endian ordered bytes
      let pid = u32::from_le_bytes(rawPid.try_into()
                  .map_err(|_| Error::Format.trace(format!("Invalid PID for {}", &self.name)))?);

      Ok(Some(pid))
    }
    else {
      Ok(None)
    }
  }

  /**
    * # Errors
    *
    * * Failed to get the service's PID,
    *
    * # Panics
    * * stat file doesn't exist for the PID (should never happen)
    */
  #[inline]
  pub fn path(&self, pathName: &str) -> Result<String>
  {
    if (pathName == "stat")
    {
      // This file should exist or else its a bug
      Ok(format!("/proc/{}/stat", self.pid()?.expect("path(stat): pid is missing")))
    }
    else {
      panic!()
    }
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
    let status: String =
    [
      String::from(
      {
        if (self.is_standard())
        {
          // Standard services will contain more information (e.g. pid)
          "├─ Status:   "
        }
        else {
          // Non-standard services will just have a status & nothing else
          "└─ Status:   "
        }
      }),
      if (self.is_standard())
      {
        // Read the /proc/<PID>/stat file, which contains the process's status information
        if let Ok(stat) = fs::read_to_string(self.path("stat")?)
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
      }
      /*
       * If the `/run/kickit/service/<S>/killed` file exists, we know the service
       * process exited on an OK signal (0)
       */
      else if (fs::metadata(path!("/run/kickit/service/", &self.name, "exited")).is_ok())
      {
        format!("{}Finished{}", Colour::GREEN, Colour::RESET)
      }
      // If not, it exited on a failure (non-zero)
      else {
        format!("{}Failed{}", Colour::RED, Colour::RESET)
      }
    ]
      .join("");

    println!("{}{}{}", Colour::BOLD, self.name, Colour::RESET);
    println!("{status}");

    if (self.is_sandboxed())
    {
      println!("├─ Sandbox:  {}active{}", Colour::GREEN, Colour::RESET);
    }

    // Non-standard services will not have an active PID
    if (self.is_standard())
    {
      println!("└─ PID:      {}", self.pid()?.unwrap());
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
    use chrono::{Local, DateTime};
    use std::{io, io::{BufWriter, Read, Write, ErrorKind::BrokenPipe}, process::exit};
    use ruzstd::decoding::StreamingDecoder;

    // The decoder implements Read to idiomatically decompress
    let mut serviceLog = StreamingDecoder::new(File::open(path!("/run/kickit/service", &self.name, "log"))
                                            .into_trace(Error::BadService).context(&self.name)?)
                          .into_trace(Error::LogAccessFail).context(&self.name)?;

    let mut logBin = Vec::new();
    // Read (potentially binary) log contents
    serviceLog.read_to_end(&mut logBin).into_trace(Error::AccessRunFsFail)?;

    // Our stdout that we write to
    let mut out = BufWriter::new(io::stdout());

    // Log properties for every line
    let mut timestamp = String::new();
    let mut logContents = String::new();
    let mut fromInit = false;

    // Loop through each byte in the log
    for (logByteCount, logByte) in (logBin.iter().enumerate())
    {
      match (logByte)
      {
        b'\n' =>
        {
          // We don't want to show an empty line if it's found (which it shouldn't be)
          if !(logContents.is_empty() || fromInit && ignoreInit)
          {
            let marker = if (fromInit)
            {
              if (ugly)
              {
                String::from("(kickit) ")
              }
              else {
                format!("{}(kickit){} ", Colour::BOLD, Colour::RESET)
              }
            }
            else {
              String::new()
            };

            let lineFormatted =
            {
              if (ugly)
              {
                // Don't make the timestamp human-readable, just millis
                format!("[{timestamp}] {marker}{logContents}\n")
              }
              else {
                // Convert the millis type from a String to i64 so it is accepted by chrono
                let timestampUgly: i64 = timestamp.parse().into_trace(Error::Format)?;

                /*
                 * Get the timestamp from the log and convert it into an actual date & time,
                 * then convert from UTC timezone to the system's timzone (Local) using
                 * the chrono crate magic
                 */
                let logTime: DateTime<Local> = DateTime::from_timestamp_millis(timestampUgly)
                                                .into_trace(Error::LogAccessFail)
                                                .context(&self.name)?
                                                .into();

                /*
                 * Format time as <Day Month Year, Hours:Minutes:Seconds> to not
                 * anger the Americans
                 */
                format!("[{}] {marker}{logContents}\n", logTime.format("%d %b %Y, %H:%M:%S"))
              }
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
            out.write_all(lineFormatted.as_bytes())
                .map_err(|e| if (e.kind() == BrokenPipe) { exit(0) } else { e })
                .into_trace(Error::Format)?;
          }

          // Reset our values for next line
          timestamp.clear();
          logContents.clear();
          fromInit = false;
        },
        // This is the marker that the message is from the init
        0x8F =>
        {
          // Must be the 14th byte or else something is wrong
          affirm!(timestamp.len() == 13 && logContents.is_empty(),
                  Error::Format.trace(format!("Unexpected byte 0x8F on byte {logByteCount}"))
                                .context(&self.name));

          fromInit = true;
        },
        _ =>
        {
          // The timestamp is just 13 bytes long
          if (timestamp.len() < 13)
          {
            // We are not done reading the full timestamp
            timestamp.push(*logByte as char);
          }
          else {
            // This is real log content, not a timestamp
            logContents.push(*logByte as char);
          }
        }
      }
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
  for service in (fs::read_dir(PathBuf::from("/run/kickit/service"))
                    .into_trace(Error::RunFsParseFail)?)
  {
    // Import each entry as a service from its OsString file name
    let upService: Service = service.into_trace(Error::AccessRunFsFail)?.file_name().into();

    // Only print if targets provided by user allow
    if let Some(targets) = maybeTargets
    {
      if (targets.contains(&upService.name))
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
