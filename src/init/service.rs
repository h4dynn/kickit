//! Service implementation

use std::{fs, process::{Command, Stdio, Child}, path::PathBuf, fmt};
use crate::{init::init_console::{KTError, KTErrorTrace, ConvKTError},
            affirm, file_path, path};

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

/*
 * Standard -> Service runs in background (on another thread), monitored by kickit,
 * RunOnce  -> Service runs on the same thread as kickit, will not continue until it exits
 */
#[derive(serde::Deserialize, PartialEq, Eq, Clone, Copy, Debug, Default)]
pub enum Pattern { #[default] Standard, RunOnce }

// Used to locate which path we want to find (e.g. exited = /run/kickit/service/S/exited)
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum Path { Exited, Pid }

#[derive(Clone, Debug)]
pub struct Logger { file: PathBuf, line: usize, last: Option<String> }

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
  log: Logger
}

trait FindPath { fn path(&self, which: Path) -> Result<PathBuf, KTErrorTrace>; }

// TO-DO!
//pub static UP_SERVICES: Mutex<Vec<Service>> = Mutex::new(Vec::default());

impl Logger
{
  ///
  /// # Errors
  /// * Service's logfile from runfs doesn't exist
  ///
  pub fn new(name: impl fmt::Display) -> Result<Self, KTErrorTrace>
  {
    use std::fs;

    let file = path!("/run/kickit/service", name.to_string(), "log");

    // Ensure log file exists
    fs::exists(&file).context_trace(name, KTError::FileNotFound)?;

    Ok(Self { line: 0, file, last: None })
  }
}

impl Service
{
  /// Source the service and nothing else
  ///
  /// # Errors
  /// * Service's configuration doesn't exist or can't be read
  /// * Service's configuration couldn't be parsed by toml
  /// * Service's provided executable doesn't exist
  ///
  pub fn init(name: &str) -> Result<Self, KTErrorTrace>
  {
    macro_rules! set
    {
      ($config: tt, $($set: tt),*) => { $(let $set = $config.$set.unwrap_or_default();)* }
    }

    let path = file_path!(path!(crate::PREFIX, "service"), name, "toml");

    // Check the service config exists and is a file
    if let Ok(meta) = path.metadata() && meta.is_file() { }
    else {
      return Err(KTErrorTrace::new(KTError::FileNotFound, &format!("{name}: Service not found")))
    }

    // Read TOML configuration contents
    let toml = fs::read_to_string(path).context_trace(name, KTError::ServiceParseFail)?;

    // Source the configuration
    let config: Config = toml::from_str(&toml)
                            .context_trace(name, KTError::ServiceParseFail)?;

    // Check the service's executable actually exists on filesystem
    affirm!(fs::metadata(&config.exec[0]).is_ok(),
      KTErrorTrace::new(KTError::FileNotFound, &format!("Executable is missing for {name}")));

    // Optional values: fallback to default if not provided (optional & shout are false)
    set!(config, description, optional, shout, pattern);

    Ok(Self
    {
      name: name.into(), description, optional, shout,
      pattern, exec: config.exec, up: false, process: None,
      log: Logger::new(name)?
    })
  }

  ///
  /// # Errors
  /// * Service is already running
  /// * Couldn't spawn or start a process for the service
  ///
  #[inline] pub fn up(&mut self) -> Result<(), KTErrorTrace>
  {
    affirm!(!self.up,
      KTErrorTrace::with_context(KTError::ServiceUpFail, &self.name, "Service is already up!"));

    // Leave marker that we at least tried to start it
    self.appendLog(format!("Starting service: {}", &self.name), true)?;

    /*
     * Based on the service's pattern we either spawn it in the background
     * and watch it or run it in foreground and wait for it to finish
     */
    match (self.pattern)
    {
      Pattern::Standard => self.process = Some(self.spawnService()?),
      Pattern::RunOnce => self.runService()?
    }

    // yippie!
    self.up = true;

    Ok(())
  }

  ///
  /// # Errors
  /// * Service isn't running already
  /// * Couldn't kill the service's process
  ///
  /// # Panics
  /// * Service doesn't have a matching process (should have since its up)
  ///
  #[inline] pub fn down(&mut self) -> Result<(), KTErrorTrace>
  {
    affirm!(self.up,
      KTErrorTrace::with_context(KTError::ServiceDownFail, &self.name, "Service is already down!"));

    // Kill the process's main process & by extension all its subprocesses
    self.process.as_mut().unwrap().kill().context_trace(&self.name, KTError::ServiceDownFail)?;

    self.appendLog(format!("Fossilised service: {}", self.name), true)?;
    // Process was successfully killed
    self.up = false;

    Ok(())
  }

  /// Append a new line to the log
  ///
  /// # Errors
  /// * Couldn't open the service's logfile
  /// * Couldn't compress or decompress the service's logfile
  /// * Couldn't open a lock on the file (or unlock)
  /// * Couldn't write to the logfile
  ///
  fn appendLog(&mut self, new: String, fromInit: bool) -> Result<(), KTErrorTrace>
  {
    use std::{io::{Write, BufReader}, fs::File, time::{SystemTime, UNIX_EPOCH}};
    use crate::{state, log, init::init_console::SERVICE, state::InitState};
    use zstd::stream::decode_all as zstdDecompressFile;
    use zstd::bulk::compress as zstdCompress;

    let logStr = &self.log.file.display();

    // Don't send empty lines if they are found for whatever reason
    if (new.is_empty()) { return Ok(()) }

    // Open the logfile with read-only permissions
    let logReader = File::open(&self.log.file)
                              .context_trace(logStr, KTError::FileNotFound)?;

    // Decompress the previous log contents
    let mut newLogContents = zstdDecompressFile(BufReader::new(logReader))
                              .context_trace(logStr, KTError::AccessLogFail)?;

    // Empty out the logfile as a write-only file here
    let mut log = File::create(&self.log.file)
                              .context_trace(logStr, KTError::FileNotFound)?;

    // Make sure nothing else touches the log while we write to it
    log.lock().trace(KTError::AccessLogFail)?;

    // Fancy way of saying get the current time
    let timeNow = SystemTime::now().duration_since(UNIX_EPOCH)
                                    .trace(KTError::Unknown)?
                                    .as_millis()
                                    .to_string();

    // Send timestamp to log
    newLogContents.extend_from_slice(timeNow.as_bytes());

    // Our marker to make sure we know this message was from init, not the service
    if (fromInit) { newLogContents.push(0x8F) }

    // Addon the log contents
    newLogContents.extend_from_slice(new.as_bytes());
    // Add a newline
    newLogContents.push(0x0A);

    if (state!() == InitState::Ok && !fromInit)
    {
      if (self.shout)
      {
        new.split('\n').for_each(|line| log!(format!("{} {}: {line}", SERVICE, self.name)));
      }
      else {
        self.log.last = Some(new);
      }
    }

    // Recompress old log contents & new contents
    let comptents = zstdCompress(&newLogContents, 3).trace(KTError::Unknown)?;

    // Send the regenerated log
    log.write_all(&comptents).context_trace(logStr, KTError::ServiceLogFail)?;

    self.log.line += 1;

    log.unlock().trace(KTError::AccessLogFail)?;

    Ok(())
  }

  /*
   * Watch the service in the background and do 2 things:
   * a) Make sure it isn't killed or zombified,
   * b) Read its logs and report them
   */
  ///
  /// # Errors
  /// * Failed to read bytes from the service log
  /// * Service was killed or turned into a zombie
  ///
  pub fn watchService(&mut self) -> Result<(), KTErrorTrace>
  {
    use std::io::{BufReader, Read};
    use crate::{state, state::InitState};

    loop {
      let mut startingBuf = [0; 1];
      let mut stderrContents = String::new();

      /*
       * Read stderr and take one byte; if this fails then we know there
       * is nothing to read but if it succeeds then we can read the rest
       */
      if let Some(process) = self.process.as_mut() &&
          let Some(stderr) = process.stderr.as_mut() &&
          stderr.read_exact(&mut startingBuf).is_ok()
      {
        // Push the single byte we read
        stderrContents.push(startingBuf[0] as char);

        // Create new BufReader for the stderr and loop through the bytes
        for wrapped in (BufReader::new(stderr).bytes())
        {
          // Panic if byte is None (should never be)
          let byte = wrapped.context_trace(&self.name, KTError::ServiceLogFail)?;

          if (byte == b'\n') { break }

          stderrContents.push(byte as char);
        }

        // Sometimes our log has multiple lines so account for this here
        for logLine in (stderrContents.split('\n'))
        {
          self.appendLog(logLine.to_owned(), false)?;
        }
      }

      if (state!() == InitState::Ok)
      {
        // Continue onto next loop if warned instead of aborted
        let Ok(spec) = fs::read_to_string(self.path(Path::Pid)?) else { self.died()?; continue; };

        // The third value in the file shows the current state (e.g. Z for zombie)
        if (matches!(spec.split(' ').nth(2), Some("Z" | "X"))) { self.died()? }
      }
    }
  }

  ///
  /// # Errors
  /// * Service was killed/zombified and isn't optional
  /// * Service couldn't be killed gracefully (`.down()` method)
  ///
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
    else if let Some(log) = &self.log.last
    {
      KTErrorTrace::with_context(KTError::ServiceNotRunning, &self.name, log)
    }
    else {
      KTErrorTrace::new(KTError::ServiceNotRunning, &self.name)
    };

    if (self.optional)
    {
      error.warn();
      Ok(())
    }
    else {
      Err(error)
    }
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

    affirm!(self.up,
      KTErrorTrace::new(KTError::ServiceAccessFail, "Cannot get PID: Service is not running"));

    affirm!(self.pattern != RunOnce, KTErrorTrace::new(KTError::ServiceAccessFail,
                                            "Cannot get PID: Service has a RunOnce pattern"));

    match (self.process.as_ref())
    {
      Some(i) => Ok(i.id()),
      None    => Err(KTError::ServiceAccessFail.into())
    }
  }

  ///
  /// # Errors
  /// * Couldn't spawn the service's executable on a new thread
  /// * Couldn't open or write to the service's PID file
  ///
  fn spawnService(&mut self) -> Result<Child, KTErrorTrace>
  {
    use std::{fs::File, io::Write};

    let args = match (self.exec.len()) { 1 => &Vec::new(), _ => &self.exec[1..] };

    let process = Command::new(&self.exec[0])
                          .args(args)
                          .stderr(Stdio::piped())
                          .spawn()
                          .context_trace(&self.name, KTError::ServiceUpFail)?;

    let mut pidFile = File::create(path!("/run/kickit/service", &self.name, "pid"))
                              .trace(KTError::RunFsFail)?;
    // Write the PID in text
    pidFile.write_all(process.id().to_string().as_bytes()).trace(KTError::RunFsFail)?;

    Ok(process)
  }

  ///
  /// # Errors
  /// * Couldn't run the service's executable
  /// * A non-UTF-8 byte was in the service's error output
  /// * Service's executable exited on a non-zero code (error)
  /// * Couldn't create the service's exit file on runfs
  ///
  fn runService(&mut self) -> Result<(), KTErrorTrace>
  {
    use std::fs::File;

    let args = match (self.exec.len()) { 1 => &Vec::new(), _ => &self.exec[1..] };

    let process = Command::new(&self.exec[0])
                          .args(args)
                          .stderr(Stdio::piped())
                          .output()
                          .context_trace(&self.name, KTError::ServiceUpFail)?;

    let logContents = String::from_utf8(process.stderr).trace(KTError::FormatFail)?;

    for line in (logContents.trim_end_matches('\n').split('\n'))
    {
      self.appendLog(line.to_owned(), false)?;
    }

    if (!process.status.success())
    {
      return Err(if let Some(log) = &self.log.last
      {
        KTErrorTrace::with_context(KTError::ServiceUpFail, &self.name, log)
      }
      else {
        KTErrorTrace::new(KTError::ServiceUpFail, &self.name)
      })
    }

    File::create(self.path(Path::Exited)?)
      .trace(KTError::RunFsFail)?;

    self.appendLog(format!("Service finished: {}", self.name), true)?;

    Ok(())
  }
}

impl fmt::Display for Service
{
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error>
  {
    write!(f, "{}", self.name)
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
