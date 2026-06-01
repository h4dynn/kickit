//! Service implementation

use std::{fs, path::{Path, PathBuf}, fmt, io, process::{Command, Child}, sync::OnceLock};
use serde::Deserialize;
use crate::{init::console::{Error, ErrorResult, Result},
      console::{affirm, ExtendWithContext}, file_path, path, oncelock};

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
  sandbox: Option<SandboxOptions>,
  runFolder: Option<RunDirectory>,
  exec: Vec<String>,

  // Automatically set options by service manager
  state: State,
  pub process: Option<Child>,
  pub log: OnceLock<Logger>
}

#[derive(Debug)]
pub struct Logger
{
  name: String,
  shout: bool,
  // The current line count
  line: usize,
  // Matching log file
  file: fs::File,
  reader: Option<io::PipeReader>
}

#[derive(Clone, PartialEq, Eq, Deserialize, Debug)]
pub struct SandboxOptions
{
  // List of all the flags in string form, such as "share_vm" or "new_user"
  flags: Vec<String>,
  // What binaries this container will be using
  import: Vec<String>,
  // Bind mount the system pseudo filesystems, required for majority of services
  bindSystemFs: Option<bool>,
  // Bind mount the dbus socket, some services like lightdm or elogind will need this
  bindDbus: Option<bool>,
  // Share the provided sandboxed files with the rest of the OS via bind mount
  files: Option<Vec<String>>
}

#[derive(PartialEq, Eq, Clone, Copy, Debug, Default)]
pub enum State
{
  Up,
  #[default] Down
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
  sandbox: Option<SandboxOptions>,
  exec: Vec<String>
}

impl Service
{
  fn run_dir(&self, of: impl fmt::Display) -> PathBuf
  {
    path!("/run/kickit/service", &self.name, of.to_string())
  }

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
      ($config: tt [$($set: tt),*]) =>
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
            Error::FileNotFound.trace(format!("{name}: Service not found")));

    // Read TOML configuration contents
    let toml = fs::read_to_string(path).into_trace(Error::ServiceParse).context(name)?;

    // Source the configuration
    let config: Config = toml::from_str(&toml).into_trace(Error::ServiceParse).context(name)?;

    // Check the service's executable actually exists on filesystem
    affirm!(fs::metadata(&config.exec[0]).is_ok(),
      Error::FileNotFound.trace(format!("Service executable missing: {name}")));

    // Optional values: fallback to default if not provided (optional & shout are false)
    set!(config[description, optional, shout, pattern]);

    let logger = config.logger.unwrap_or(true);

    Ok(Self
    {
      name: name.into(), description, optional, shout, sandbox: config.sandbox,
      runFolder: config.run_folder,
      pattern, logger, exec: config.exec, state: Down, process: None,
      log: OnceLock::new()
    })
  }

  /**
    * # Errors
    * * Service is already running
    * * Couldn't spawn or start a process for the service
    */
  #[inline]
  pub async fn up(&mut self) -> Result<()>
  {
    use nix::unistd::{chown, Uid, Gid};
    use super::TARGET;
    use tokio::time::timeout;
    use std::{fs::{File, Permissions}, path::PathBuf, io::{Write, BufRead, BufReader, pipe},
                  os::unix::fs::PermissionsExt, time::Duration};

    // An empty zstd file, created from /dev/null (`$ zstd -1c < /dev/null | xxd`)
    const EMPTY_ZSTD: [u8; 13] = [0x28, 0xb5, 0x2f, 0xfd, 0x24, 0x00, 0x01, 0x00,
                                    0x00, 0x99, 0xe9, 0xd8, 0x51];

    affirm!(self.state == Down, Error::ServiceUp.trace("already up!").context(&self.name));

    if (self.logger)
    {
      // Create the empty log file
      let mut logFile = File::options().read(true).write(true).create_new(true)
                          .open(self.run_dir("log"))
                          .into_trace(Error::ServiceAccess).context(&self.name)?;

      // Make log file inaccessible to anybody but the root user
      logFile.set_permissions(Permissions::from_mode(0o600)).into_trace(Error::ServiceAccess)?;

      /*
       * The log file is going to be decompressed later on to view its contents,
       * the decompressor will fail if the file is empty so this provides some
       * valid but empty zstd data
       */
      logFile.write_all(&EMPTY_ZSTD).into_trace(Error::ServiceAccess)?;

      if (self.log.set(Logger::new(self.name.clone(), self.shout, logFile)).is_err())
      {
        return Err(Error::Unknown.trace(format!("Failed to set logger for {}", self.name)))
      }
    }

    // Create the run folder, if one is given
    if let Some(run) = &self.runFolder
    {
      let path = path!("/run", &run.name);
      fs::create_dir(&path).into_trace(Error::ServiceAccess).context(&run.name)?;

      // Change the owner (UID) & group (GID)
      chown(&path, Some(Uid::from_raw(run.owner)), Some(Gid::from_raw(run.group))).into_trace(Error::ServiceAccess)?;
      // Set the permissions mode
      fs::set_permissions(path, Permissions::from_mode(run.mode)).into_trace(Error::ServiceAccess)?;
    }

    /*
     * Opening a pipe here allows us to read both stderr/stdout as one (like in bash),
     * since some services will output logs to stdout & some to stderr
     */
    let (reader, writer) = pipe().into_trace(Error::Unknown)?;

    // The executable name we are going to call, plus its arguments
    let (exec, args): (PathBuf, Vec<String>) =
    {
      /*
       * Convert the sandbox options into arguments for warden to then handle.
       * The reason why we don't just start the sandbox from here is because using
       * `unshare` will cause the init to also unshare (bad!), so we use the warden
       * executable to start a new, seperate command which can be safely unshared
       */
      if let Some(sandboxOptions) = &self.sandbox
      {
        // Setup the container rootfs
        sandboxOptions.setup(self.run_dir("container"))?;
        (PathBuf::from("/usr/lib/kickit/warden"), sandboxOptions.wardenArgs(self.run_dir("container"), &mut self.exec)?)
      }
      else {
        (PathBuf::from(&self.exec[0]), self.exec[1..].to_vec())
      }
    };

    /*
     * Based on the service's pattern we either spawn it in the background
     * and watch it or run it in foreground and wait for it to finish
     */
    match (self.pattern)
    {
      Standard | Forking =>
      {
        // Spawn the process in the background asynchronous to the program
        let process = Command::new(exec).args(args)
                        .current_dir("/")
                        .stdout(writer.try_clone().into_trace(Error::Unknown)?)
                        .stderr(writer)
                        .spawn()
                        .into_trace(Error::ServiceUp).context(&self.name)?;

        // Where our pid info is stored
        let mut pidFile = File::create_new(self.run_dir("pid")).into_trace(Error::ServiceAccess)?;

        // Write the PID in little-endian ordered bytes
        pidFile.write_all(&process.id().to_le_bytes()).into_trace(Error::ServiceAccess)?;
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
        let mut process = Command::new(exec).args(args)
                            .current_dir("/")
                            .stderr(writer.try_clone().into_trace(Error::Unknown)?)
                            .stdout(writer)
                            .spawn()
                            .into_trace(Error::ServiceUp).context(&self.name)?;

        // How long are we willing to wait for this service? (default: 5 seconds)
        let maxWaitTime = Duration::from_secs(oncelock!(&TARGET)?.serviceTimeout);

        match (timeout(maxWaitTime, async { process.wait() }).await)
        {
          // Timeout was not reached, and service exited normally!
          Ok(Ok(status)) =>
          {
            if (status.success())
            {
              self.log("Service has finished", true)?;
            }
            else {
              return Err(Error::ServiceUp.trace(format!("{} exited on error ({})", &self.name, status)))
            }
          },
          // Service's process failed
          Ok(Err(error)) => error.into_trace(Error::ServiceUp)?,
          // Timeout was reached...
          Err(..) => return Err(Error::ServiceUp.trace(format!("Timeout while waiting for {}", &self.name)))
        }

        // A buffered read is the most efficient way to read line-by-line
        for maybeLine in (BufReader::new(reader).lines())
        {
          let line = maybeLine.into_trace(Error::Format).context(format!("Invalid input for {}", self.name))?;
          self.log(&line, false)?;
        }

        // Signal to ktctl that we have finished
        File::create_new(self.run_dir("exited")).into_trace(Error::ServiceAccess)?;

        if let Some(log) = self.log.get_mut()
        {
          // Make logfile read-only as the process has finished
          log.file.set_permissions(Permissions::from_mode(0o400)).into_trace(Error::ServiceAccess)?;
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
    *
    * * Service process exited on a code (even if the code is 0),
    * * Failed to wait for the process
    */
  pub fn supervise(name: &str, mut process: Child) -> Result<()>
  {
    use super::POWER_OFF;

    /*
     * Make sure if we receive an error, but the init system has been told to
     * power-off, that we don't actually return the error, as a service being
     * killed during power-off is normal behavoir
     */
    macro_rules! err
    {
      ($error: expr) =>
      {
        if let Some(powerOff) = POWER_OFF.get() && (*powerOff)
        {
          // Don't want to trigger an error if we are powering off - this is expected behavoir
          Ok(())
        }
        else {
          Err($error)
        }
      }
    }

    match (process.wait())
    {
      // Service has exited normally on a code
      Ok(status) =>
      {
        err!(Error::ServiceDown.trace(
        {
          if let Some(code) = status.code()
          {
            format!("Service {name} has died: {code}")
          }
          else {
            format!("Service {name} has died: code is unavailable!")
          }
        }))
      },
      // Even the watcher has failed for some reason
      Err(error) => err!(Error::ServiceDown.trace(error).context(name))
    }
  }

  /**
    * # Errors
    *
    * * Service does not have a logger available (this will only happen if the service
    *   hasn't been started)
    */
  pub fn log(&mut self, new: &str, fromInit: bool) -> Result<()>
  {
    oncelock!(&mut self.log)?.log(new, fromInit)
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

impl SandboxOptions
{
  /**
    * # Errors
    *
    * * Failed to create a required system directory in the container (e.g. etc, usr, sys, dev, proc),
    * * Failed to change directory to the container for whatever reason,
    * * Failed to create a required symlink (e.g. bin -> usr/bin)
    */
  // Create the container rootfs, with bind mounts
  pub fn setup(&self, containerPath: impl AsRef<Path>) -> Result<()>
  {
    use std::{fs::create_dir, env::set_current_dir, os::unix::fs::symlink};

    macro_rules! mkdir
    {
      ($dir: expr) =>
      {
        create_dir($dir).into_trace(Error::ServiceSandboxInit)?;
      }
    }

    let container: &Path = containerPath.as_ref();

    // This is where our temporary container rootfs will be stored
    mkdir!(container);

    mkdir!(path!(container, "etc"));
    // Create the main usr directories
    mkdir!(path!(container, "usr"));
    mkdir!(path!(container, "usr", "bin"));
    mkdir!(path!(container, "usr", "lib"));
    mkdir!(path!(container, "usr", "share"));

    // Create system pseudo filesystem directories
    for systemFsDir in ["sys", "dev", "proc", "run", "tmp"]
    {
      mkdir!(path!(container, systemFsDir));
    }

    // Change to this directory to make accurate symlinks that work both in & outside of chroot
    set_current_dir(path!(container, "usr")).into_trace(Error::ServiceSandboxInit)?;
    symlink("lib", path!(container, "usr", "lib64")).into_trace(Error::ServiceSandboxInit)?;

    set_current_dir(container).into_trace(Error::ServiceSandboxInit)?;

    symlink("usr/lib", path!(container, "lib")).into_trace(Error::ServiceSandboxInit)?;
    // Assume that we are running on a 64-bit system here
    symlink("usr/lib", path!(container, "lib64")).into_trace(Error::ServiceSandboxInit)?;
    symlink("usr/bin", path!(container, "bin")).into_trace(Error::ServiceSandboxInit)?;

    Ok(())
  }

  fn wardenArgs(&self, container: impl AsRef<Path>, cmdArgs: &mut Vec<String>) -> Result<Vec<String>>
  {
    use std::path::Path;

    let mut args = Vec::new();

    // Vast majority of services will require this
    if (self.bindSystemFs.unwrap_or_default())
    {
      args.push("--mount-system-fs".to_owned());
    }
    // For services such as elogind or lightdm which use the dbus daemon
    if (self.bindDbus.unwrap_or_default())
    {
      args.push("--dbus".to_owned());
    }

    for flag in (&self.flags)
    {
      // Flag arg can be either 'new' or 'share', so for NsFlag::NewMount it would be 'new_mount'
      let (flagArg, flagName) = flag.split_once('_').ok_or(Error::ServiceSandboxBadOpt.trace(flag))?;

      match (flagArg)
      {
        "share" => args.push(String::from("--share")),
        "new" => args.push(String::from("--new")),
        _ => return Err(Error::ServiceSandboxBadOpt.trace(format!("Expected either share/new type flag, got '{flagArg}'")))
      }

      // Cut off the flag arg part, as we have turned it into its relative argument
      args.push(flagName.to_owned());
    }

    for bind in (&self.import)
    {
      let file: &Path = bind.as_ref();

      // Binding directories & files have different implementation due to creating parent dirs/files
      if (file.is_dir())
      {
        args.push(String::from("--bind-dir"));
      }
      else if (file.is_file())
      {
        args.push(String::from("--bind-file"));
      }
      else {
        return Err(Error::ServiceSandboxInit.trace(format!("No such file or directory: {bind}")))
      }

      args.push(bind.to_owned());
    }

    // The first argument will be the executable, so we take it away from the vector
    let exec = cmdArgs.remove(0);
    // Tell warden where our root container is
    args.push(container.as_ref().display().to_string());
    // This is the executable that warden will run
    args.push(exec);

    if (cmdArgs.len() > 1)
    {
      // Tell warden to pass over the rest of the arguments to the command we are executing
      args.push("--".to_owned());
      // Make sure we can index past this to avoid a panic
      args.extend_from_slice(cmdArgs.as_slice());
    }

    Ok(args)
  }
}

impl Logger
{
  #[must_use]
  pub const fn new(name: String, shout: bool, file: fs::File) -> Self
  {
    Self { name, shout, line: 0, file, reader: None }
  }

  /*
   * Watch the reader for the process for updates & then send them to the
   * service's log
   */
  /**
    * # Errors
    *
    * * Logger doesn't have a reader (this will only happen for services with a `RunOnce` pattern),
    * * Failed to read bytes from the reader,
    * * Failed to add new line to the log (in `self.log(_, _)`)
    */
  pub fn watch(&mut self) -> Result<()>
  {
    use std::{thread::sleep, time::Duration, io::{Read, BufReader}};

    loop {
      /*
       * Wait every 1/4th a second to check service, without this
       * kickit would use lots of CPU %
       */
      sleep(Duration::from_millis(250));

      let mut tester = [0; 1];
      let mut log = String::new();

      let mut logReader = self.reader.as_ref().ok_or(Error::Unknown.trace("Logger missing!"))?;

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
          let byte = maybeByte.into_trace(Error::ServiceLogContent).context(&self.name)?;

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
    use crate::{state::state, init::console::{Marker::Service as Mark, log}, console::Colour};
    use ruzstd::{decoding::StreamingDecoder, encoding::{compress, CompressionLevel}};

    // Don't send empty lines if they are found for whatever reason
    if (new.is_empty())
    {
      return Ok(())
    }

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
    self.file.rewind().into_trace(Error::ServiceLogContent)?;

    // This is the zstd decoder which wraps over the file & implements Read
    let mut decoder = StreamingDecoder::new(&self.file).into_trace(Error::ServiceLogCompress)?;
    let mut contents = Vec::new();
    // Read the decompressed log contents
    decoder.read_to_end(&mut contents).into_trace(Error::ServiceLogCompress)?;

    // This is to timestamp the log at the time it was received
    let timeNow = SystemTime::now().duration_since(UNIX_EPOCH)
                                    .into_trace(Error::Time)?
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
    self.file.rewind().into_trace(Error::ServiceLogContent)?;
    // Overwrite log file with new contents
    compress(Cursor::new(contents), &mut self.file, CompressionLevel::Fastest);
    self.line += 1;

    Ok(())
  }
}
