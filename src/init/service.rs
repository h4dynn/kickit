//! Service implementation

use std::{fs, process::{Command, Stdio, Child}, path::PathBuf,
          os::unix::net::UnixStream as Stream, sync::{Arc, OnceLock}};

use crate::{init::init_console::{KTError, KTErrorTrace, KTErrorResult},
            console::affirm, file_path, path, socket::KTSocket, Data};

// The service body which is generated from the init() method
#[derive(Debug)]
pub struct Service
{
  // Pre-defined options (found in service configuration)
  pub name: String,
  pub description: String,
  pub optional: bool,
  pub pattern: Pattern,
  shout: bool,
  exec: Vec<String>,

  // Automatically set options by service manager
  //state: State,
  up: bool,
  process: Option<Child>,
  log: Arc<Logger>
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct Logger { line: usize, contents: Data }

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct Socket;

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct SocketHandle
{
  name: String,
  log: Arc<Logger>
}

/*
 * Standard -> Service runs in background (on another thread), monitored by kickit,
 * RunOnce  -> Service runs on the same thread as kickit, will not continue until it exits
 */
#[derive(serde::Deserialize, PartialEq, Eq, Clone, Copy, Debug, Default)]
pub enum Pattern { #[default] Standard, RunOnce }

// This is used when toml::from_str() sources the service's configuration
#[derive(serde::Deserialize, PartialEq, Eq, Clone, Debug)]
struct Config
{
  description: Option<String>,
  optional: Option<bool>,
  shout: Option<bool>,
  pattern: Option<Pattern>,
  exec: Vec<String>
}

// Used to locate which path we want to find (e.g. exited = /run/kickit/service/S/exited)
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum Path { Exited, Pid }

trait FindPath
{
  fn path(&self, which: Path) -> Result<PathBuf, KTErrorTrace>;
}

// An index that may fail - has to be called via `.try_index()`
trait TryIndex<Idx: ?Sized>
{
  type Output: ?Sized;
  fn try_index(&self, index: Idx) -> Option<&Self::Output>;
}

pub static SERVICE_HANDLES: OnceLock<Vec<SocketHandle>> = OnceLock::new();

impl Default for Logger
{
  fn default() -> Self
  {
    // An empty zstd file, in hex bytes, created from /dev/null (`$ zstd -1c < /dev/null | xxd`)
    let EMPTY_ZSTD = crate::hex_data("28b52ffd240001000099e9d851").unwrap();

    Self { line: 0, contents: EMPTY_ZSTD }
  }
}

impl From<&Service> for SocketHandle
{
  fn from(service: &Service) -> Self
  {
    Self { name: service.name.clone(), log: Arc::clone(&service.log) }
  }
}

impl TryIndex<&str> for Vec<SocketHandle>
{
  type Output = SocketHandle;

  fn try_index(&self, candName: &str) -> Option<&SocketHandle>
  {
    for handle in (self)
    {
      if (&handle.name as &str == candName)
      {
        return Some(handle)
      }
    }
    None
  }
}

impl KTSocket for Socket
{
  fn name(&self) -> String { String::from("Service") }

  async fn handler(&self, mut stream: Stream)
  {
    use std::io::Write;
    use crate::{console::HandleKTError, socket::StreamBytes};

    // To avoid repeating the same bytes & stream
    macro_rules! fail
    {
      () =>
      {
        $crate::socket::fail!(stream, Default);
      };
    }

    let Some(input) = stream.stream_bytes_eol() else { fail!(); };
    let Ok(name) = String::from_utf8(input[1..].to_vec()) else { fail!(); };

    match (input[0])
    {
      Self::LOG =>
      {
        let mut handle: SocketHandle =
        {
          /*
           * SERVICE_HANDLES is a OnceLock, so we get the inner value of it
           * or panic if it is unset (which it shouldn't be!), then since this
           * is a vector of ServiceHandles we try to index the service name
           * that we want from the vector, and if it is found return that
           * handle but clone it so we can make the handle mutable
           * ...complicating right? maybe this needs to be improved on
           */
          if let Some(handle) = SERVICE_HANDLES.get().unwrap().try_index(&name as &str)
          {
            handle.clone()
          }
          else {
            fail!();
          }
        };
        stream.write_all(&Arc::make_mut(&mut handle.log).contents)
      },
      _ => stream.write_all(&[0x0f])
    }
      .trace(KTError::Socket).or_warn();

    Self::shutdown(stream).or_warn();
  }
}

impl Socket
{
  pub const LOG: u8 = 0x3C;
}

impl Service
{
  ///
  /// # Errors
  /// * Service's configuration doesn't exist or can't be read
  /// * Service's configuration couldn't be parsed by toml
  /// * Service's provided executable doesn't exist
  ///
  /// Source the service and nothing else
  pub fn init(name: &str) -> Result<Self, KTErrorTrace>
  {
    macro_rules! set
    {
      ($config: tt, $($set: tt),*) => { $(let $set = $config.$set.unwrap_or_default();)* }
    }

    let path = file_path!(path!(crate::PREFIX, "service"), name, "toml");

    // Check the service config exists and is a file
    affirm!(path.is_file(),
            KTErrorTrace::new(KTError::FileNotFound, &format!("{name}: Service not found")));

    // Read TOML configuration contents
    let toml = fs::read_to_string(path).context_trace(name, KTError::ServiceParse)?;

    // Source the configuration
    let config: Config = toml::from_str(&toml).context_trace(name, KTError::ServiceParse)?;

    // Check the service's executable actually exists on filesystem
    affirm!(fs::metadata(&config.exec[0]).is_ok(),
      KTErrorTrace::new(KTError::FileNotFound, &format!("Service executable missing: {name}")));

    // Optional values: fallback to default if not provided (optional & shout are false)
    set!(config, description, optional, shout, pattern);

    Ok(Self
    {
      name: name.into(), description, optional, shout,
      pattern, exec: config.exec, up: false, process: None,
      log: Arc::new(Logger::default())
    })
  }

  ///
  /// # Errors
  /// * Service is already running
  /// * Couldn't spawn or start a process for the service
  ///
  #[inline] pub fn up(&mut self) -> Result<(), KTErrorTrace>
  {
    use std::{fs::File, io::Write};

    affirm!(!self.up,
      KTErrorTrace::with_context(KTError::ServiceUp, &self.name, "already up"));

    // Leave marker that we at least tried to start it
    self.log(&format!("Starting service: {}", &self.name) as &str, true)?;

    // Our arguments for the executable
    let args = if (self.exec.len() == 1) { &Vec::new() } else { &self.exec[1..] };

    /*
     * Based on the service's pattern we either spawn it in the background
     * and watch it or run it in foreground and wait for it to finish
     */
    match (self.pattern)
    {
      Pattern::Standard =>
      {
        // Spawn the process in the background asynchronous to the program
        let process = Command::new(&self.exec[0]).args(args).stderr(Stdio::piped()).spawn()
                                  .context_trace(&self.name, KTError::ServiceUp)?;

        // Where our pid info is stored
        let mut pidFile = File::create(path!("/run/kickit/service", &self.name, "pid"))
                                  .trace(KTError::RunFsFail)?;
        // Write the PID in text
        pidFile.write_all(process.id().to_string().as_bytes()).trace(KTError::RunFsFail)?;
        // Transfer the process to our service
        self.process = Some(process);
      },
      Pattern::RunOnce =>
      {
        // Start the service's process & wait for it to finish
        let process = Command::new(&self.exec[0]).args(args).stderr(Stdio::piped()).output()
                                  .context_trace(&self.name, KTError::ServiceUp)?;

        // Read the process's stderr contents
        let log = String::from_utf8(process.stderr).trace(KTError::Format)?;

        for line in (log.trim_end_matches('\n').split('\n'))
        {
          // Add a line to the logfile
          self.log(line, false)?;
        }

        // Service's process failed on non-zero code
        if (!process.status.success())
        {
          /* return Err(if let Some(last) = &self.log.last
          {
            KTErrorTrace::with_context(KTError::ServiceUp, &self.name, last)
          }
          else {
            KTErrorTrace::new(KTError::ServiceUp, &self.name)
          }) */
          return Err(KTErrorTrace::new(KTError::ServiceUp, &self.name))
        }

        File::create(FindPath::path(self, Path::Exited)?).trace(KTError::RunFsFail)?;

        // done!!!!!!!!!!!!!!!!!!!!!!!!!
        self.log(&format!("Service finished: {}", self.name) as &str, true)?;
      }
    }

    // yippie!
    self.up = true;

    Ok(())
  }

  ///
  /// # Errors
  /// * Service isn't running already
  /// * Service has a `RunOnce pattern` (no process to kill)
  /// * Couldn't kill the service's process
  ///
  /// # Panics
  /// * Service doesn't have a matching process (should have since its up)
  ///
  #[inline] pub fn down(&mut self) -> Result<(), KTErrorTrace>
  {
    use crate::init::{init_console::status, service::Pattern::RunOnce};

    affirm!(self.up,
      KTErrorTrace::with_context(KTError::ServiceDown, &self.name, "Already down"));

    affirm!(self.pattern != RunOnce,
      KTErrorTrace::with_context(KTError::ServiceDown, &self.name, "Has a RunOnce pattern"));

    status!("Killing service: {}", &self.name);
    // Kill the process's main process & by extension all its subprocesses
    self.process.as_mut().unwrap().kill().context_trace(&self.name, KTError::ServiceDown)?;

    self.log(&format!("Fossilised service: {}", self.name) as &str, true)?;
    // Process was successfully killed
    self.up = false;

    Ok(())
  }

  ///
  /// # Errors
  /// * Failed to read bytes from the service log
  /// * Service was killed or turned into a zombie
  ///
  /*
   * Watch the service in the background and do 3 things:
   *
   * 1) Make sure it isn't killed or zombified,
   * 2) Read its logs and report them,
   * 3) Wait for a power signal & stop the service if one is given
   */
  pub fn watch(&mut self) -> Result<(), KTErrorTrace>
  {
    use std::io::{BufReader, Read};
    use crate::state::state;

    loop {
      let mut tester = [0; 1];
      let mut log = String::new();

      /*
       * Read stderr and take one byte; if this fails then we know there
       * is nothing to read but if it succeeds then we can read the rest
       */
      if let Some(ref mut process) = self.process && let Some(out) = process.stderr.as_mut() &&
        (out.read_exact(&mut tester).is_ok())
      {
        // Push the single byte we read
        log.push(tester[0] as char);

        // Create new BufReader for the stderr and loop through the bytes
        for wrapped in (BufReader::new(out).bytes())
        {
          // Panic if byte is None (should never be)
          let byte = wrapped.context_trace(&self.name, KTError::ServiceLog)?;

          // This is our EOF- we know to stop reading bytes here
          if (byte == b'\n') { break }

          log.push(byte as char);
        }

        // Sometimes our log has multiple lines so account for this here
        for logLine in (log.split('\n'))
        {
          self.log(logLine, false)?;
        }
      }

      // TO-DO: implement this properly as sometimes it will hang forever
      /* if (crate::init::POWER_LEVEL.get().is_some())
      {
        self.down()?;
        return Ok(())
      } */

      if (state!().is_ok())
      {
        // Continue onto next loop if warned instead of aborted
        let Ok(spec) = fs::read_to_string(FindPath::path(self, Path::Pid)?)
                          else { self.died()?; continue; };

        // The third value in the file shows the current state (e.g. Z for zombie)
        if (matches!(spec.split(' ').nth(2), Some("Z" | "X"))) { self.died()? }
      }
    }
  }

  /// Append a new line to the log
  ///
  /// # Errors
  /// * Couldn't open the service's logfile
  /// * Couldn't compress or decompress the service's logfile
  /// * Couldn't open a lock on the file (or unlock)
  /// * Couldn't write to the logfile
  ///
  /// # Panics
  /// * More than one line was provided to the function
  ///
  fn log(&mut self, new: &str, fromInit: bool) -> Result<(), KTErrorTrace>
  {
    use std::time::{SystemTime, UNIX_EPOCH};
    use crate::{state::state, init::init_console::{Marker::Service as Mark, log}, console::Colour};
    use zstd::bulk::decompress as zstdDecompress;
    use zstd::bulk::compress as zstdCompress;

    // Make a mutable reference we can modify the service log with
    let logger = Arc::make_mut(&mut self.log);

    // Don't send empty lines if they are found for whatever reason
    if (new.is_empty()) { return Ok(()) }

    /*
     * We cannot have more than 1 line of content, this function is designed
     * to take a-line-at-a-time (use `for line in input.split('\n') { ... }`)
     */
    assert!(new.split('\n').count() <= 1, "log() was given more than 1 line of input!");

    // Decompress the previous log contents
    let mut newLogContents = zstdDecompress(logger.contents.as_slice(), 10_000_000_000)
                                .context_trace(&self.name, KTError::AccessLog)?;

    // Fancy way of saying get the current time
    let timeNow = SystemTime::now().duration_since(UNIX_EPOCH)
                                    .trace(KTError::Unknown)?
                                    .as_millis()
                                    .to_string();

    // Send timestamp to log
    newLogContents.extend_from_slice(timeNow.as_bytes());

    // Our marker to make sure we know this message was from init, not the service
    if (fromInit) { newLogContents.push(0x8f) }

    // Addon the log contents
    newLogContents.extend_from_slice(new.as_bytes());
    // Add a newline
    newLogContents.push(0x0a);

    /*
     * Make sure we are not stalled, because if we are we will
     * interrupt the user shell with our message, and then
     * check this message isn't from the init because if so
     * it will have already been reported once
     */
    if (state!().is_ok() && !fromInit)
    {
      // Only report the message to master log if service says we should
      if (self.shout)
      {
        for line in (new.split('\n'))
        {
          log!(format!("{} {}({}):{} {line}", Mark, Colour::BOLD, self.name, Colour::RESET));
        }
      }
    }

    // Recompress old log contents & new contents
    logger.contents = zstdCompress(&newLogContents, 3).trace(KTError::Unknown)?;
    logger.line += 1;

    Ok(())
  }

  /* pub fn is_up(name: &str) -> bool
  {
    use std::{fs, fs::File};

    // Get the service's pid (if it exists)
    let Ok(pid) = fs::read_to_string(path!("/run/kickit", name, "pid")) else { return false };

    match (fs::read_to_string(path!("/proc", pid, "stat")))
    {
      Ok(stat) if (matches!(stat.split(' ').nth(2), Some("S" | "I" | "D" | "R"))) => true,
      _ => false
    }
  } */

  ///
  /// # Errors
  /// * Service was killed/zombified and isn't optional
  /// * Service couldn't be killed gracefully (`.down()` method)
  ///
  #[doc(hidden)]
  #[inline] fn died(&mut self) -> Result<(), KTErrorTrace>
  {
    use crate::console::ReturnError;

    // Try to stop it or return error if that fails
    self.down()?;

    /*
     * If the service has shout set to true, then there is no point in providing
     * the service's log as a trace since it has already been reported
     */
    let error = if (self.shout)
    {
      KTErrorTrace::with_context(KTError::ServiceNotRunning, &self.name, "")
    }
    /* else if let Some(log) = &self.log.last
    {
      KTErrorTrace::with_context(KTError::ServiceNotRunning, &self.name, log)
    } */
    else {
      KTErrorTrace::new(KTError::ServiceNotRunning, &self.name)
    };

    if (self.optional) { error.warn(); Ok(()) } else { Err(error) }
  }

  ///
  /// # Errors
  /// * Service isn't running
  /// * Service has a `RunOnce` pattern
  /// * Service's process couldn't be found
  ///
  fn pid(&self) -> Result<u32, KTErrorTrace>
  {
    use crate::init::service::Pattern::RunOnce;

    affirm!(self.up, KTErrorTrace::new(KTError::ServiceAccess,
            "Cannot get PID: Service is not running"));

    affirm!(self.pattern != RunOnce, KTErrorTrace::new(KTError::ServiceAccess,
                                    "Cannot get PID: Service has an incompatible pattern"));

    match (&self.process)
    {
      Some(i) => Ok(i.id()), None => Err(KTError::ServiceAccess.into())
    }
  }
}

impl FindPath for Service
{
  fn path(&self, which: Path) -> Result<PathBuf, KTErrorTrace>
  {
    use Path::{Exited, Pid};

    Ok(PathBuf::from(match (which)
    {
      Exited => format!("/run/kickit/service/{}/exited", self.name),
      Pid => format!("/proc/{}/stat", self.pid()?)
    }))
  }
}
