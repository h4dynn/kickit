//! Service implementation

use std::{fs, io, process::{Command, Child}, sync::OnceLock, path::PathBuf};
use serde::Deserialize;
use crate::{init::init_console::{Error, ExtendWithContext, ErrorResult, Result},
      console::affirm, file_path, path};

pub static UP_SERVICES: OnceLock<Vec<&str>> = OnceLock::new();

// The service body which is generated from the init() method
#[derive(Debug)]
pub struct Service
{
  // Pre-defined options (found in service configuration)
  pub name: String,
  pub description: String,
  pub optional: bool,
  pub pattern: Pattern,
  pub logger: bool,
  shout: bool,
  runFolder: Option<RunDirectory>,
  exec: Vec<String>,

  // Automatically set options by service manager
  state: State,
  process: Option<Child>,
  log: OnceLock<Logger>
}

#[derive(Debug)]
pub struct Logger
{
  // The current line count
  line: usize,
  // Matching log file
  file: fs::File,
  reader: Option<io::PipeReader>
}

#[derive(PartialEq, Eq, Clone, Copy, Debug, Default)]
pub enum State
{
  Up,
  #[default]
  Down
}
use State::{Up, Down};

/*
 * Standard -> Service runs in background (on another thread), monitored by kickit,
 * RunOnce  -> Service runs on the same thread as kickit, will not continue until it exits
 */
#[derive(Deserialize, PartialEq, Eq, Clone, Copy, Debug, Default)]
pub enum Pattern
{
  // The regular-type daemon, that runs forever until shutdown
  #[default]
  Standard,
  // Creates fork(s) of the process, kickit makes sure to not kill the service
  Forking,
  // Run command(s) once & then exit
  RunOnce
}
use Pattern::{Standard, Forking, RunOnce};

#[derive(Deserialize, PartialEq, Eq, Clone, Debug)]
struct RunDirectory
{
  name: String,
  group: u32,
  owner: u32,
  mode: u32
}

// This is used when toml::from_str() sources the service's configuration
#[derive(Deserialize, PartialEq, Eq, Clone, Debug)]
struct Config
{
  description: Option<String>,
  optional: Option<bool>,
  shout: Option<bool>,
  pattern: Option<Pattern>,
  logger: Option<bool>,
  run_folder: Option<RunDirectory>,
  exec: Vec<String>
}

// Used to locate which path we want to find (e.g. exited = /run/kickit/service/S/exited)
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum Path
{
  Exited,
  Pid
}

trait FindPath
{
  fn path(&self, which: Path) -> Result<PathBuf>;
}

impl Logger
{
  #[must_use]
  pub const fn new(file: fs::File) -> Self
  {
    Self { line: 0, file, reader: None }
  }
}

impl Service
{
  /**
    * # Errors
    * * Service's configuration doesn't exist or can't be read
    * * Service's configuration couldn't be parsed by toml
    * * Service's provided executable doesn't exist
    */
  // Source the service and nothing else
  pub fn init(name: &str) -> Result<Self>
  {
    macro_rules! set
    {
      ($config: tt, $($set: tt),*) =>
      {
        $(
          // Look for the requested value in the configuration or use the default
          let $set = $config.$set.unwrap_or_default();
        )*
      };
    }

    let path = file_path!(path!(crate::PREFIX, "service"), name, "toml");

    // Check the service config exists and is a file
    affirm!(path.is_file(),
            Error::FileNotFound.trace(&format!("{name}: Service not found")));

    // Read TOML configuration contents
    let toml = fs::read_to_string(path).into_trace(Error::ServiceParse).context(name)?;

    // Source the configuration
    let config: Config = toml::from_str(&toml).into_trace(Error::ServiceParse).context(name)?;

    // Check the service's executable actually exists on filesystem
    affirm!(fs::metadata(&config.exec[0]).is_ok(),
      Error::FileNotFound.trace(&format!("Service executable missing: {name}")));

    // Optional values: fallback to default if not provided (optional & shout are false)
    set!(config, description, optional, shout, pattern);

    let logger = config.logger.unwrap_or(true);

    Ok(Self
    {
      name: name.into(), description, optional, shout,
      runFolder: config.run_folder, pattern, logger,
      exec: config.exec, state: Down, process: None,
      log: OnceLock::new()
    })
  }

  /**
    * # Errors
    * * Service is already running
    * * Couldn't spawn or start a process for the service
    */
  #[inline]
  pub fn up(&mut self) -> Result<()>
  {
    use nix::unistd::{chown, Uid, Gid};
    use std::{fs::{File, Permissions}, os::unix::fs::PermissionsExt,
              io::{Write, BufRead, BufReader, pipe}};

    // An empty zstd file, created from /dev/null (`$ zstd -1c < /dev/null | xxd`)
    const EMPTY_ZSTD: [u8; 13] = [0x28, 0xb5, 0x2f, 0xfd, 0x24, 0x00, 0x01, 0x00,
                                    0x00, 0x99, 0xe9, 0xd8, 0x51];

    affirm!(self.state == Down, Error::ServiceUp.trace("already up").context(&self.name));

    // Our arguments for the executable
    let args = {
      // Check to make sure we don't index past the vector's length (causes a panic)
      if (self.exec.len() == 1)
      {
        &Vec::new()
      }
      else {
        &self.exec[1..]
      }
    };

    if (self.logger)
    {
      // Create the empty log file
      let mut logFile = File::options().read(true).write(true).create_new(true)
                          .open(path!("/run/kickit/service", &self.name, "log"))
                          .into_trace(Error::RunFsFail).context(&self.name)?;

      // Make log file inaccessible to anybody but the root user
      logFile.set_permissions(Permissions::from_mode(0o600)).into_trace(Error::RunFsFail)?;
      /*
       * The log file is going to be decompressed later on to view its contents,
       * the decompressor will fail if the file is empty so this provides some
       * valid but empty zstd data
       */
      logFile.write_all(&EMPTY_ZSTD).into_trace(Error::RunFsFail)?;

      if (self.log.set(Logger::new(logFile)).is_err())
      {
        return Err(Error::Unknown.trace(&format!("Failed to set logger for {}", self.name)))
      }
    }

    // Create the run folder, if one is given
    if let Some(run) = &self.runFolder
    {
      let path = path!("/run", &run.name);
      fs::create_dir(&path).into_trace(Error::RunFsFail).context(&run.name)?;

      // Change the owner (UID) & group (GID)
      chown(&path, Some(Uid::from_raw(run.owner)), Some(Gid::from_raw(run.group))).into_trace(Error::RunFsFail)?;
      // Set the permissions mode
      fs::set_permissions(path, Permissions::from_mode(run.mode)).into_trace(Error::RunFsFail)?;
    }

    /*
     * Opening a pipe here allows us to read both stderr/stdout as one (like in bash),
     * since some services will output logs to stdout & some to stderr
     */
    let (reader, writer) = pipe().into_trace(Error::Unknown)?;
    /*
     * Based on the service's pattern we either spawn it in the background
     * and watch it or run it in foreground and wait for it to finish
     */
    match (self.pattern)
    {
      Standard | Forking =>
      {
        // Spawn the process in the background asynchronous to the program
        let process = Command::new(&self.exec[0]).args(args)
                        .stdout(writer.try_clone().into_trace(Error::Unknown)?)
                        .stderr(writer)
                        .spawn()
                        .into_trace(Error::ServiceUp).context(&self.name)?;

        // Where our pid info is stored
        let mut pidFile = File::create_new(path!("/run/kickit/service", &self.name, "pid"))
                                  .into_trace(Error::RunFsFail)?;

        // Write the PID in text
        pidFile.write_all(process.id().to_string().as_bytes()).into_trace(Error::RunFsFail)?;
        // Transfer the process to our service
        self.process = Some(process);

        let log = self.log.get_mut().ok_or(Error::Unknown.trace("Log is missing!").context(&self.name))?;
        // Link the reader to the log
        log.reader = Some(reader);

        self.log("Started service", true)?;
      },
      RunOnce =>
      {
        // Start the service's process & wait for it to finish
        let process = Command::new(&self.exec[0]).args(args)
                        .stderr(writer.try_clone().into_trace(Error::Unknown)?)
                        .stdout(writer)
                        .output()
                        .into_trace(Error::ServiceUp).context(&self.name)?;

        for maybeLine in (BufReader::new(reader).split(b'\n'))
        {
          let line = maybeLine.into_trace(Error::Format).context(format!("Invalid input for {}", self.name))?;
          let stringLine = String::from_utf8(line).into_trace(Error::Format).context(&self.name)?;

          self.log(&stringLine, false)?;
        }

        // Service's process exit on 0 (success)
        if (process.status.success())
        {
          self.log("Service has finished", true)?;
        }
        else {
          return Err(Error::ServiceUp.trace(&self.name))
        }

        File::create_new(FindPath::path(self, Path::Exited)?).into_trace(Error::RunFsFail)?;

        if let Some(log) = self.log.get_mut()
        {
          // Make logfile read-only as the process has finished
          log.file.set_permissions(Permissions::from_mode(0o400)).into_trace(Error::RunFsFail)?;
        }
        // done!!!!!!!!!!!!!!!!!!!!!!!!!
      }
    }

    // yippie!
    self.state = Up;

    Ok(())
  }

  /**
    * # Errors
    * * Service isn't running already
    * * Service has a `RunOnce pattern` (no process to kill)
    * * Couldn't kill the service's process
    *
    * # Panics
    * * Service doesn't have a matching process (should have since its up)
    */
  #[inline]
  pub(crate) fn down(&mut self) -> Result<()>
  {
    use crate::init::{init_console::status, service::Pattern::RunOnce};

    affirm!(self.state == Up, Error::ServiceDown.trace("Already down").context(&self.name));
    // RunOnce services are already dead so we can't kill them
    affirm!(self.pattern != RunOnce, Error::ServiceDown.trace("Invalid pattern").context(&self.name));

    status!("Killing service: {}", &self.name);
    // Kill the process's main process & by extension all its subprocesses
    self.process.as_mut().unwrap().kill().into_trace(Error::ServiceDown).context(&self.name)?;

    self.log(&format!("Fossilised service: {}", self.name), true)?;
    // Process was successfully killed
    self.state = Down;

    Ok(())
  }

  /**
    * # Errors
    * * Failed to read bytes from the service log
    * * Service was killed or turned into a zombie
    */
  /*
   * Watch the service in the background and do 3 things:
   *
   * 1) Make sure it isn't killed or zombified,
   * 2) Read its logs and report them,
   * 3) Wait for a power signal & stop the service if one is given
   */
  pub fn watch(&mut self) -> Result<()>
  {
    use std::{io::{Read, BufReader}, thread::sleep, time::Duration};
    use crate::{state::state, console::ReturnError};
    use super::POWER_OFF;

    loop {
      /*
       * Wait every 1/4th a second to check service, without this
       * kickit would use lots of CPU %
       */
      sleep(Duration::from_millis(250));

      if (self.logger) && let Some(logger) = self.log.get()
      {
        let mut tester = [0; 1];
        let mut log = String::new();

        let mut logReader = logger.reader.as_ref().ok_or(Error::Unknown.trace("Logger missing!"))?;

        /*
         * Read stderr and take one byte; if this fails then we know there
         * is nothing to read but if it succeeds then we can read the rest
         */
        if (logReader.read_exact(&mut tester).is_ok())
        {
          // Push the single byte we read
          log.push(tester[0] as char);

          /*
           * We use a BufReader here since its more efficient to do so
           * (thanks clippy!)
           */
          for maybeByte in (BufReader::new(logReader).bytes())
          {
            // Panic if byte is Err (should never be)
            let byte = maybeByte.into_trace(Error::ServiceLog).context(&self.name)?;

            // This is our EOF- we know to stop reading bytes here
            if (byte == b'\n')
            {
              break
            }

            log.push(byte as char);
          }

          // Sometimes our log has multiple lines so account for this here
          for logLine in (log.split('\n'))
          {
            self.log(logLine, false)?;
          }
        }
      }

      if (state!().is_ok())
      {
        // Make sure our service is running okay (Z = zombie / X = killed)
        if let Ok(spec) = fs::read_to_string(FindPath::path(self, Path::Pid)?) &&
            (!matches!(spec.split(' ').nth(2), Some("Z" | "X")))
        {
          // Don't need to do anything
          continue;
        }
        // Power off signal sent by `poweroff(_, _)` function
        if let Some(powerOff) = POWER_OFF.get() && (*powerOff)
        {
          // Don't want to trigger an error if we are powering off - this is expected behavoir
          return Ok(())
        }

        // Try to stop it or return error if that fails
        self.down()?;

        let error = Error::ServiceNotRunning.trace("Service died!").context(&self.name);

        if (self.optional)
        {
          // We don't want an optional service throwing a global error
          error.warn();
        }
        else {
          // uh-oh very bad
          return Err(error)
        }
      }
    }
  }

  /**
    * Append a new line to the log
    *
    * # Errors
    * * Couldn't open the service's logfile
    * * Couldn't compress or decompress the service's logfile
    * * Couldn't open a lock on the file (or unlock)
    * * Couldn't write to the logfile
    *
    * # Panics
    * * More than one line was provided to the function
    */
  pub fn log(&mut self, new: &str, fromInit: bool) -> Result<()>
  {
    use std::{io::{Read, Seek, Cursor}, time::{SystemTime, UNIX_EPOCH}};
    use crate::{state::state, init::init_console::{Marker::Service as Mark, log}, console::Colour};
    use ruzstd::{decoding::StreamingDecoder, encoding::{compress, CompressionLevel}};

    // Don't send empty lines if they are found for whatever reason
    if (new.is_empty() || !self.logger)
    {
      return Ok(())
    }

    let log = self.log.get_mut().ok_or(Error::Unknown.trace("Log uninitialised!"))?;

    /*
     * We cannot have more than 1 line of content, this function is designed
     * to take a-line-at-a-time (use `for line in input.split('\n') { ... }`).
     * This is a panic & not an error because it is solely down to how the
     * program is coded, not user error
     */
    assert!(new.split('\n').count() <= 1, "log() was given more than 1 line of input!");

    /*
     * Because this function will be called multiple times, changing the seek
     * on the log file, we do this to ensure we are reading from the beginning
     */
    log.file.rewind().into_trace(Error::Unknown)?;

    // This is the zstd decoder which wraps over the file & implements Read
    let mut decoder = StreamingDecoder::new(&log.file).into_trace(Error::Unknown)?;
    let mut contents = Vec::new();
    // Read the decompressed log contents
    decoder.read_to_end(&mut contents).into_trace(Error::AccessLog)?;

    // This is to timestamp the log at the time it was received
    let timeNow = SystemTime::now().duration_since(UNIX_EPOCH)
                                    .into_trace(Error::Unknown)?
                                    .as_millis()
                                    .to_string();

    // Send timestamp to log
    contents.extend_from_slice(timeNow.as_bytes());

    // This byte is how ktctl can differentiate messages from init & service
    if (fromInit)
    {
      contents.push(0x8f);
    }

    // Addon the log contents
    contents.extend_from_slice(new.as_bytes());
    // Add a newline
    contents.push(b'\n');

    /*
     * Make sure we are not stalled, because if we are we will
     * interrupt the user shell with our message, and then
     * check this message isn't from the init because if so
     * it will have already been reported once
     */
    if (state!().is_ok() && !fromInit && self.shout)
    {
      for line in (new.split('\n'))
      {
        log!(format!("{} {}({}):{} {line}", Mark, Colour::BOLD, self.name, Colour::RESET));
      }
    }

    /*
     * Reading from the logfile changes the cursor position so we set it back to
     * the beginning to overwrite the file instead of appending it
     */
    log.file.rewind().into_trace(Error::Unknown)?;
    // Overwrite log file with new contents
    compress(Cursor::new(contents), &mut log.file, CompressionLevel::Fastest);
    log.line += 1;

    Ok(())
  }

  /**
    * # Errors
    * * Service isn't running
    * * Service has a `RunOnce` pattern
    * * Service's process couldn't be found
    */
  pub fn pid(&self) -> Result<u32>
  {
    use crate::init::service::Pattern::RunOnce;

    affirm!(self.state == Up, Error::ServiceAccess.trace("Down"));
    // These aren't saved because they'll be dead
    affirm!(self.pattern != RunOnce, Error::ServiceAccess.trace("Cannot get PID for RunOnce service"));

    match (&self.process)
    {
      Some(i) => Ok(i.id()),
      None => Err(Error::ServiceAccess.into())
    }
  }
}

impl FindPath for Service
{
  fn path(&self, which: Path) -> Result<PathBuf>
  {
    use Path::{Exited, Pid};

    Ok(PathBuf::from(match (which)
    {
      Exited => format!("/run/kickit/service/{}/exited", self.name),
      Pid => format!("/proc/{}/stat", self.pid()?)
    }))
  }
}
