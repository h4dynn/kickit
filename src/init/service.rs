//! Service implementation

use serde::Deserialize;
use super::{target::TARGET, console::{Error, Result}};
use crate::{PREFIX, BoxedStr, console::{guard, ExtendWithContext, ErrorResult}, file_path, path, oncelock};
use std::{fs, path::{Path, PathBuf}, collections::VecDeque, fmt, io, sync::{Arc, OnceLock},
            process::{Command, Child, ExitStatus}};

oncelock! {
  // Standard services with name & pids, accessed when shutting down
  pub static STANDARD_SERVICES: Vec<Service>;
}

// The service body which is generated from the init() method
#[derive(Debug)]
pub struct Service
{
  // This will shared across threads, so an Arc is most suitable
  pub name: Arc<BoxedStr>,
  // Optional services won't cause an init error if they exit/fail
  pub optional: bool,
  // See `Pattern` enum for more info
  pub pattern: Pattern,
  // Do we want a logger on this service?
  pub logger: bool,
  // Process's ID, set only after `up()` is called
  pub pid: OnceLock<u32>,
  // Print log entries to the init's console
  shout: bool,
  // Launch with the `warden` namespace sandboxer
  sandbox: Option<Sandbox>,
  // Create a run folder for this service
  runFolder: Option<RunDirectory>,
  // Executable + optional arguments
  exec: VecDeque<BoxedStr>,
  // Automatically set options by service manager
  state: State,
  // The spawned child of this process, set only after `up()` is called
  process: OnceLock<Child>,
  // Logger for the service, also set only by `up()`
  log: OnceLock<Logger>
}

pub struct Supervisor
{
  name: Arc<BoxedStr>,
  process: Child
}

#[derive(Debug)]
pub struct Logger
{
  name: Arc<BoxedStr>,
  shout: bool,
  // The current line count
  line: usize,
  // Matching log file
  file: fs::File,
  // The read pipe from the service process, set when `up` is ran
  reader: Option<io::PipeReader>
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub enum LogEntry<'entry>
{
  // Regular entry from service's stderr & stdout PipeReader
  Service(&'entry str),
  // Special entry from init
  Init(&'entry str)
}

#[derive(PartialEq, Eq, Clone, Deserialize, Debug)]
pub struct Sandbox
{
  // List of all the flags in string form, such as "ShareVm" or "NewUser"
  flags: Vec<BoxedStr>,
  // What binaries this container will be using
  import: Vec<BoxedStr>,
  // Bind mount the system pseudo filesystems, required for majority of services
  bindSystemFs: Option<bool>,
  // Bind mount the dbus socket, some services like lightdm or elogind will need this
  bindDbus: Option<bool>,
  // Share the provided sandboxed files with the rest of the OS via bind mount
  files: Option<BoxedStr>
}

#[derive(PartialEq, Eq, Clone, Copy, Debug, Default)]
pub enum State
{
  Up,
  #[default] Down,
  Dead(ExitStatus)
}
use State::{Up, Down, Dead};

/*
 * Standard -> Service runs in background (on another thread), supervised by kickit,
 * Forking  -> Service spawns children & then exits, treated as standard but not supervised,
 * RunOnce  -> Service runs on the same thread as kickit, will not continue until it exits
 */
#[derive(Deserialize, PartialEq, Eq, Clone, Copy, Debug, Default)]
pub enum Pattern
{
  // The regular-type daemon, that runs forever until shutdown
  #[default]
  Standard = 0xa4,
  // Creates fork(s) of the process, kickit makes sure to not kill the service
  Forking = 0xef,
  // Run command(s) once & then exit
  RunOnce = 0xe9
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
  description: Option<BoxedStr>,
  optional: Option<bool>,
  shout: Option<bool>,
  pattern: Option<Pattern>,
  logger: Option<bool>,
  run_folder: Option<RunDirectory>,
  sandbox: Option<Sandbox>,
  exec: VecDeque<BoxedStr>
}

impl Service
{
  fn run_path(&self, of: impl fmt::Display) -> PathBuf
  {
    PathBuf::from(format!("/run/kickit/service/{}/{of}", &self.name))
  }

  /**
    * # Errors
    *
    * - Service's configuration doesn't exist or can't be read
    * - Service's configuration couldn't be parsed by toml
    * - Service's provided executable doesn't exist
    */
  // Source the service and nothing else
  pub fn init(name: impl AsRef<str>) -> Result<Self>
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

    let path = file_path!(path!(PREFIX, "service"), name.as_ref(), "toml");

    // Make sure the configuration file exists
    guard!(!path.is_file() => Error::FileNotFound.trace(format!("{}: Service not found", name.as_ref())));

    // Read TOML configuration contents
    let toml = fs::read_to_string(&path).into_trace(Error::ServiceParse).context(name.as_ref())?;

    // Source the configuration
    let config: Config = toml::from_str(&toml).into_trace(Error::ServiceParse).context(name.as_ref())?;

    // Check the service's executable actually exists on filesystem
    guard!(fs::metadata(config.exec[0].as_ref()).is_err() =>
              Error::FileNotFound.trace(format!("Service executable missing: {}", name.as_ref())));

    // Optional values: fallback to default if not provided (optional & shout are false)
    set!(config[optional, shout, pattern]);

    let logger = config.logger.unwrap_or(true);

    Ok(Self {
      name: Arc::new(name.as_ref().into()), optional, shout,
      sandbox: config.sandbox, runFolder: config.run_folder,
      pattern, logger, exec: config.exec, state: Down,
      process: OnceLock::new(), log: OnceLock::new(),
      pid: OnceLock::new()
    })
  }

  // Convert provided args to executable & arguments based on service options
  fn args(&mut self) -> Result<(PathBuf, VecDeque<BoxedStr>)>
  {
    use std::mem::take;

    // Take the old contents of the vector, and replace `self.exec` with an empty vector
    let mut serviceArgs = take(&mut self.exec);

    /*
     * Convert the sandbox options into arguments for warden to then handle.
     * The reason why we don't just start the sandbox from here is because using
     * `unshare` will cause the init to also unshare (bad!), so we use the warden
     * executable to start a new, seperate command which can be safely unshared
     */
    if let Some(sandboxOptions) = self.sandbox.take()
    {
      let mut args = VecDeque::<BoxedStr>::new();
      // Setup the container rootfs
      sandboxOptions.setup(self.run_path("container"))?;

      let exec = path!(PREFIX, "warden");

      // Add the generated arguments from our options
      args.extend(sandboxOptions.wardenArgs(self.run_path("container").display().to_string().into(), &mut serviceArgs)?);

      Ok((exec, args))
    }
    else {
      // First field will be the executable path
      let exec = serviceArgs.pop_front().ok_or(Error::ServiceSandboxInit.trace(format!("No executable: {}", self.name)))?;

      Ok((PathBuf::from(exec.as_ref()), serviceArgs))
    }
  }

  /*
   * Generate configuration cache from the source file for ktctl to use. The reason
   * why we cache instead of just telling ktctl where the service's configuration
   * file is, is to make sure ktctl is using the same instance that the init is using,
   * in case the service's config is changed at any point
   */
  fn cache(mut cache: impl io::Write, pattern: Pattern, sandboxed: bool, state: State, pid: u32) -> Result<()>
  {
    cache.write_all(&[pattern as u8, sandboxed.into()]).into_trace(Error::ServiceAccess)?;
    // u32 in LE bytes will be 4 bytes long
    cache.write_all(&pid.to_le_bytes()).into_trace(Error::ServiceAccess)?;

    if let Some(intState) = &<Option<i32> as From<State>>::from(state)
    {
      // 0x01 indicates we have a status available - 0x00 indicates none available
      cache.write_all(&[1]).into_trace(Error::ServiceAccess)?;
      // This will be 4 bytes long
      cache.write_all(&intState.to_le_bytes()).into_trace(Error::ServiceAccess)?;
    }
    else {
      // Write all null bytes as padding to keep the length the same
      cache.write_all(&[0; 5]).into_trace(Error::ServiceAccess)?;
    }

    Ok(())
  }

  /**
    * # Errors
    *
    * - Service is already running
    * - Couldn't spawn or start a process for the service
    *
    * # Panics
    *
    * - Failed to read from `std::io::Lines` iterator (error in `std::io::BufRead::read_line`)
    */
  #[inline]
  pub async fn up(&mut self) -> Result<()>
  {
    use nix::unistd::{chown, Uid, Gid};
    use tokio::time::timeout;
    use std::{fs::{File, Permissions}, io::{Write, BufRead, BufReader, pipe},
                os::unix::fs::PermissionsExt, time::Duration};

    // An empty zstd file, created from /dev/null (`$ zstd -1c < /dev/null | xxd`)
    const EMPTY_ZSTD: [u8; 13] = [0x28, 0xb5, 0x2f, 0xfd, 0x24, 0x00, 0x01, 0x00,
                                    0x00, 0x99, 0xe9, 0xd8, 0x51];

    guard!(self.state != Down => Error::ServiceUp.trace("already up!").context(&self.name));

    if (self.logger)
    {
      // Create the empty log file
      let mut logFile = File::options().read(true).write(true).create_new(true)
                          .open(self.run_path("log"))
                          .into_trace(Error::ServiceAccess).context(&self.name)?;

      // Make log file inaccessible to anybody but the root user
      logFile.set_permissions(Permissions::from_mode(0o600)).into_trace(Error::ServiceAccess)?;

      /*
       * The log file is going to be decompressed later on to view its contents,
       * the decompressor will fail if the file is empty so this provides some
       * valid but empty zstd data
       */
      logFile.write_all(&EMPTY_ZSTD).into_trace(Error::ServiceAccess)?;

      if (self.log.set(Logger::new(Arc::clone(&self.name), self.shout, logFile)).is_err())
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

    // `self.sandbox` gets taken when we call `args()` so test here before it is
    let sandboxed = self.sandbox.is_some();
    // The executable name we are going to call, plus its arguments
    let (exec, args) = self.args()?;

    /*
     * Based on the service's pattern we either spawn it in the background
     * and watch it or run it in foreground and wait for it to finish
     */
    match (self.pattern)
    {
      Standard | Forking =>
      {
        // Spawn the process in the background asynchronous to the program
        let process = Command::new(exec).args(args.iter().map(AsRef::as_ref))
                        .current_dir("/")
                        .stdout(writer.try_clone().into_trace(Error::Unknown)?)
                        .stderr(writer)
                        .spawn()
                        .into_trace(Error::ServiceUp).context(&self.name)?;

        self.pid.set(process.id()).map_err(|_| Error::Unknown.trace(format!("Failed to set PID for {}", self.name)))?;

        // Transfer the process to our service
        self.process.set(process).map_err(|_| Error::Unknown.trace(format!("Failed to set process for {}", self.name)))?;

        if (self.logger)
        {
          let log = self.log.get_mut().ok_or(Error::Unknown.trace("Log is missing!").context(&self.name))?;
          // Link the reader to the log
          log.reader = Some(reader);
        }

        // hooray!
        self.log(&LogEntry::Init("Started service"))?;
        self.state = Up;
      },
      RunOnce =>
      {
        // Start the service's process & wait for it to finish
        let mut process = Command::new(exec).args(args.iter().map(AsRef::as_ref))
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
            self.state = Dead(status);

            if (!status.success())
            {
              dbg!(BufReader::new(reader).lines().map(|x| x.unwrap()).collect::<Vec<String>>());
              return Err(Error::ServiceUp.trace(format!("{} exited on error ({})", &self.name, status)))
            }
          },
          Ok(Err(error)) => error.into_trace(Error::ServiceUp)?,
          // Timeout was reached...
          Err(..) => return Err(Error::ServiceUp.trace(format!("Timeout while waiting for {}", &self.name)))
        }

        // A buffered read is the most efficient way to read line-by-line
        for maybeLine in (BufReader::new(reader).lines())
        {
          let line = maybeLine.into_trace(Error::Format).context(&self.name)?;
          self.log(&LogEntry::Service(&line))?;
        }

        if let Some(log) = self.log.get_mut()
        {
          // Make logfile read-only as the process has finished
          log.file.set_permissions(Permissions::from_mode(0o400)).into_trace(Error::ServiceAccess)?;
        }
        self.log(&LogEntry::Init("Service finished successfully"))?;
        // done!!!!!!!!!!!!!!!!!!!!!!!!!
      }
    }
    let cache = File::create_new(self.run_path("config")).into_trace(Error::ServiceAccess)?;
    // Now that all that crap is done let ktctl know how the service is doing
    Self::cache(cache, self.pattern, sandboxed, self.state, self.pid().unwrap_or(0))?;

    Ok(())
  }

  /**
    * # Errors
    *
    * - Service is already down,
    * - Service's process hasn't been moved out (supervisor is not running, which it should be),
    * - Failed to get the PID of the service (see `Service::pid`),
    * - Failed to kill the service (see `nix::sys::signal::kill`)
    */
  // Kill a service - used when powering off
  pub fn down(&self) -> Result<()>
  {
    use crate::{init::console::warn, console::{Colour, HandleError}};
    use nix::{unistd::Pid, sys::signal::{kill, Signal}, errno::Errno};

    guard!(self.state != Up => Error::ServiceDown.trace("Cannot kill a service that isn't alive!"));
    // Process should have been moved out to the supervisor at this point
    guard!(self.process.get().is_some() => Error::ServiceDown.trace("Service is not ready to be killed"));

    // nix wants a signed integer for some reason?
    let pid: i32 = self.pid()?.cast_signed();

    // First try killing with SIGQUIT
    match (kill(Pid::from_raw(pid), Some(Signal::SIGQUIT)))
    {
      Ok(..) => (),
      // Process is already dead, no need to kill it
      Err(Errno::ESRCH) => warn!("Service is already down"),
      Err(err) =>
      {
        warn!("Failed to kill service, so we will force kill it: {err}");
        // If that doesn't work we use SIGKILL, and if that doesn't work we can't do anything but warn really
        kill(Pid::from_raw(pid), Some(Signal::SIGKILL)).into_trace(Error::Unknown).or_warn();
      }
    }

    Ok(())
  }

  /**
    * # Errors
    *
    * - Logger has already been taken or is missing
    */
  // WARNING: This method will take ownership of the logger from the service, replacing it to be unset
  pub fn logger(&mut self) -> Result<Logger>
  {
    self.log.take().ok_or(Error::Unknown.trace(format!("{}: Logger missing!", &self.name)))
  }

  /**
    * # Errors
    *
    * - Process has already been taken or is missing
    */
  // WARNING: This method will take ownership of the process
  pub fn supervisor(&mut self) -> Result<Supervisor>
  {
    let name = Arc::clone(&self.name);
    let process = self.process.take().ok_or(Error::ServiceAccess.trace(format!("Failed to transfer service's process: {name}")))?;

    Ok(Supervisor { name, process })
  }

  /**
    * # Errors
    *
    * - Service does not have a logger available (this will only happen if the service
    *   hasn't been started)
    */
  pub fn log(&mut self, entry: &LogEntry<'_>) -> Result<()>
  {
    let (new, fromInit) = match (entry)
    {
      LogEntry::Init(new) => (new, true),
      LogEntry::Service(new) => (new, false)
    };

    oncelock!(&mut self.log)?.log(new, fromInit)
  }

  /**
    * # Errors
    *
    * - Service isn't running
    * - Service has a `RunOnce` pattern
    * - Service's process couldn't be found
    */
  pub fn pid(&self) -> Result<u32>
  {
    guard!(self.state == Down => Error::ServiceAccess.trace("Service is down, cannot get PID"));
    oncelock!(&self.pid).map(|pid| *pid)
  }
}

impl Sandbox
{
  /**
    * # Errors
    *
    * - Failed to create a required system directory in the container (e.g. etc, usr, sys, dev, proc),
    * - Failed to change directory to the container for whatever reason,
    * - Failed to create a required symlink (e.g. bin -> usr/bin)
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

  fn wardenArgs(mut self, container: BoxedStr, args: &mut VecDeque<BoxedStr>) -> Result<Vec<BoxedStr>>
  {
    use std::{path::Path, mem::take};

    let mut out = Vec::<BoxedStr>::new();

    // Vast majority of services will require this
    if (self.bindSystemFs.unwrap_or_default())
    {
      out.push(BoxedStr::from("--mount-system-fs"));
    }
    // For services such as elogind or lightdm which use the dbus daemon
    if (self.bindDbus.unwrap_or_default())
    {
      out.push(BoxedStr::from("--dbus"));
    }

    // We can take the s here because we know they won't be used for anything else
    for flag in (take(&mut self.flags))
    {
      out.extend([BoxedStr::from("--flag"), flag]);
    }

    for bind in (&self.import)
    {
      let file: &Path = AsRef::<str>::as_ref(bind).as_ref();

      // Binding directories & files have different implementation due to creating parent dirs/files
      if (file.is_dir())
      {
        out.push(BoxedStr::from("--bind-dir"));
      }
      else if (file.is_file())
      {
        out.push(BoxedStr::from("--bind-file"));
      }
      else {
        return Err(Error::ServiceSandboxInit.trace(format!("No such file or directory: {bind}")))
      }

      out.push(bind.to_owned());
    }

    let exec = args.pop_front().ok_or(Error::ServiceSandboxInit.trace("No executable in args"))?;

    // Tell warden where our root container is
    out.push(container);
    // This is the executable that warden will run
    out.push(exec);
    // `.make_contiguous()` moves all the items at the end of this deque to the front
    out.extend_from_slice(args.make_contiguous());

    Ok(out)
  }
}

impl Supervisor
{
  /**
    * # Errors
    *
    * - Service process exited on a code (even if the code is 0),
    * - Failed to wait for the process
    */
  pub fn supervise(&mut self) -> Result<()>
  {
    use super::power::POWER_OFF_READY;

    /*
     * Make sure if we receive an error, but the init system has been told to
     * power-off, that we don't actually return the error, as a service being
     * killed during power-off is normal behavior
     */
    macro_rules! err
    {
      ($error: expr) =>
      {
        if (POWER_OFF_READY.get().is_some_and(|ready| *ready))
        {
          // Don't trigger an error if powering off - we want the service to be killed
          Ok(())
        }
        else {
          Err($error)
        }
      }
    }

    match (self.process.wait())
    {
      // Service has exited normally on a code
      Ok(status) =>
      {
        err!(Error::ServiceDown.trace(if let Some(code) = status.code()
        {
          format!("Service {} has died: {code}", self.name.as_ref())
        }
        else {
          format!("Service {} has died: code is unavailable!", self.name.as_ref())
        }))
      },
      // Even the watcher has failed for some reason
      Err(error) => err!(Error::ServiceDown.trace(error).context(self.name.as_ref()))
    }
  }
}

impl Logger
{
  pub const INIT_ENTRY: u8 = 0x8f;

  #[must_use]
  pub const fn new(name: Arc<BoxedStr>, shout: bool, file: fs::File) -> Self
  {
    Self { name, shout, line: 0, file, reader: None }
  }

  // Watch the reader for the process for updates & then send them to the service's log
  /**
    * # Errors
    *
    * - Logger doesn't have a reader (this will only happen for services with a `RunOnce` pattern),
    * - Failed to read bytes from the reader,
    * - Failed to add new line to the log (in `self.log(_, _)`)
    */
  pub fn watch(&mut self) -> Result<()>
  {
    use std::{io::{BufRead, BufReader}, mem::take};

    // Using BufReader for this kind of thing is just more efficient (thanks clippy!)
    let mut logReader = BufReader::new(self.reader.take().ok_or(Error::Unknown.trace("Logger missing!"))?);

    // Buffer of all the (hopefully) UTF-8 bytes this service will provide
    let mut content = Vec::new();

    loop {
      // Read until a newline which is our EOF
      logReader.read_until(b'\n', &mut content).into_trace(Error::ServiceLogContent)?;
      // Convert from raw UTF-8 bytes to a string
      let log = String::from_utf8(take(&mut content)).into_trace(Error::ServiceLogContent)?;

      for line in (log.lines())
      {
        // And finally add the logs from the service!
        self.log(line, false)?;
      }
    }
  }

  /**
    * Append a new line to the log
    *
    * # Errors
    *
    * - Couldn't open the service's logfile
    * - Couldn't compress or decompress the service's logfile
    * - Couldn't open a lock on the file (or unlock)
    * - Couldn't write to the logfile
    *
    * # Panics
    *
    * - More than one line was provided to the function
    */
  // TO-DO: current method is to decompress & then recompress, which is kinda inefficient
  pub fn log(&mut self, new: &str, fromInit: bool) -> Result<()>
  {
    use std::{io::{Read, Seek, Cursor}, time::{SystemTime, UNIX_EPOCH}};
    use crate::{state::state, init::console::{Marker::Service as marker, log}, console::Colour};
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
    let timeNow = SystemTime::now().duration_since(UNIX_EPOCH).into_trace(Error::Time)?.as_millis().to_string();

    // Send timestamp to log
    contents.extend_from_slice(timeNow.as_bytes());

    // This byte is how ktctl can differentiate messages from init & service
    if (fromInit)
    {
      contents.push(Self::INIT_ENTRY);
    }

    // Addon the log contents
    contents.extend_from_slice(new.as_bytes());
    // Add a newline
    contents.push(b'\n');

    /*
     * Make sure we are not stalled, because if we are we will interrupt the user shell
     * with our message, and then check this message isn't from the init because if so
     * it will have already been reported once
     */
    if (state!().is_ok() && !fromInit && self.shout)
    {
      for line in (new.split('\n'))
      {
        log!(format!("{marker} {}({}):{} {line}", Colour::Bold, self.name, Colour::Reset));
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

impl From<State> for Option<i32>
{
  fn from(state: State) -> Option<i32>
  {
    match (state)
    {
      Up | Down => None,
      Dead(status) => status.code()
    }
  }
}

impl TryFrom<u8> for Pattern
{
  type Error = io::Error;

  fn try_from(input: u8) -> io::Result<Self>
  {
    // We can't use `x as y` in a match pattern
    const STANDARD: u8 = Pattern::Standard as u8;
    const FORKING: u8 = Pattern::Forking as u8;
    const RUN_ONCE: u8 = Pattern::RunOnce as u8;

    match (input)
    {
      STANDARD => Ok(Self::Standard), FORKING => Ok(Self::Forking), RUN_ONCE => Ok(Self::RunOnce),
      _ => Err(io::Error::new(io::ErrorKind::InvalidInput, format!("Expected valid pattern byte, got: {input}")))
    }
  }
}
