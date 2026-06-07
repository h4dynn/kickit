//! Service implementation

use std::{fs, path::{Path, PathBuf}, boxed::Box, collections::VecDeque, fmt, io,
            process::{Command, Child, ExitStatus}, sync::OnceLock};
use serde::Deserialize;
use super::{target::TARGET, console::{Error, Result}};
use crate::{console::{guard, ExtendWithContext, ErrorResult}, file_path, path, oncelock};

oncelock! {
  // Standard services with name & pids, accessed when shutting down
  pub static SERVICES: Vec<(Box<str>, u32)>;
}

pub const INIT_LOG_ENTRY: u8 = 0x8f;

// The service body which is generated from the init() method
#[derive(Debug)]
pub struct Service
{
  pub name: Box<str>,
  // Optional services won't cause an init error if they exit/fail
  pub optional: bool,
  // See `Pattern` enum for more info
  pub pattern: Pattern,
  // Do we want a logger on this service?
  pub logger: bool,
  // Print log entries to the init's console
  shout: bool,
  // Launch with the `warden` namespace sandboxer
  sandbox: Option<Sandbox>,
  // Create a run folder for this service
  runFolder: Option<RunDirectory>,
  // Executable + optional arguments
  exec: VecDeque<Box<str>>,
  // Automatically set options by service manager
  state: State,
  // The spawned child of this process, set only after `up()` is called
  process: OnceLock<Child>,
  // Logger for the service, also set only by `up()`
  log: OnceLock<Logger>
}

#[derive(Debug)]
pub struct Logger
{
  name: Box<str>,
  shout: bool,
  // The current line count
  line: usize,
  // Matching log file
  file: fs::File,
  // The read pipe from the service process, set when `up` is ran
  reader: Option<io::PipeReader>
}

#[derive(Clone, PartialEq, Eq, Deserialize, Debug)]
pub struct Sandbox
{
  // List of all the flags in string form, such as "share_vm" or "new_user"
  flags: Vec<Box<str>>,
  // What binaries this container will be using
  import: Vec<Box<str>>,
  // Bind mount the system pseudo filesystems, required for majority of services
  bindSystemFs: Option<bool>,
  // Bind mount the dbus socket, some services like lightdm or elogind will need this
  bindDbus: Option<bool>,
  // Share the provided sandboxed files with the rest of the OS via bind mount
  files: Option<Box<str>>
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
  description: Option<Box<str>>,
  optional: Option<bool>,
  shout: Option<bool>,
  pattern: Option<Pattern>,
  logger: Option<bool>,
  run_folder: Option<RunDirectory>,
  sandbox: Option<Sandbox>,
  exec: VecDeque<Box<str>>
}

impl Service
{
  fn run_path(&self, of: impl fmt::Display) -> String
  {
    format!("/run/kickit/service/{}/{of}", &self.name)
  }

  // These methods allow for moving out but not changing/modifying while still in struct
  pub fn process(&mut self) -> Option<Child>
  {
    self.process.take()
  }
  pub fn logger(&mut self) -> Option<Logger>
  {
    self.log.take()
  }

  /**
    * # Errors
    *
    * * Service's configuration doesn't exist or can't be read
    * * Service's configuration couldn't be parsed by toml
    * * Service's provided executable doesn't exist
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

    let path = file_path!(path!(crate::PREFIX, "service"), name.as_ref(), "toml");

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
      name: name.as_ref().into(), optional, shout,
      sandbox: config.sandbox, runFolder: config.run_folder,
      pattern, logger, exec: config.exec, state: Down,
      process: OnceLock::new(), log: OnceLock::new()
    })
  }

  // Convert provided args to executable & arguments based on service options
  fn args(&mut self) -> Result<(PathBuf, VecDeque<Box<str>>)>
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
      let mut args = VecDeque::<Box<str>>::new();
      // Setup the container rootfs
      sandboxOptions.setup(self.run_path("container"))?;

      let exec = PathBuf::from("/usr/lib/kickit/warden");

      // Add the generated arguments from our options
      args.extend(sandboxOptions.wardenArgs(self.run_path("container").into(), &mut serviceArgs)?);

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
    cache.write(&[pattern as u8]).into_trace(Error::ServiceAccess)?;
    cache.write(&[sandboxed.into()]).into_trace(Error::ServiceAccess)?;

    if let Some(intState) = &<State as Into<Option<i32>>>::into(state)
    {
      // 0x01 indicates we have a status available - 0x00 indicates none available
      cache.write(&[1]).into_trace(Error::ServiceAccess)?;
      // This will be 4 bytes long
      cache.write(&intState.to_le_bytes()).into_trace(Error::ServiceAccess)?;
    }
    else {
      // Write all null bytes as padding to keep the length the same
      cache.write(&[0; 5]).into_trace(Error::ServiceAccess)?;
    }
    // u32 in LE bytes will be 4 bytes long
    cache.write(&pid.to_le_bytes()).into_trace(Error::ServiceAccess)?;

    Ok(())
  }

  /**
    * # Errors
    *
    * * Service is already running
    * * Couldn't spawn or start a process for the service
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

        // Transfer the process to our service
        guard!(self.process.set(process).is_err() =>
                  Error::Unknown.trace(format!("Failed to set process for {}", self.name)));

        if (self.logger)
        {
          let log = self.log.get_mut().ok_or(Error::Unknown.trace("Log is missing!").context(&self.name))?;
          // Link the reader to the log
          log.reader = Some(reader);
        }

        // hooray!
        self.log("Started service", true)?;
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
          self.log(&line, false)?;
        }

        if let Some(log) = self.log.get_mut()
        {
          // Make logfile read-only as the process has finished
          log.file.set_permissions(Permissions::from_mode(0o400)).into_trace(Error::ServiceAccess)?;
        }
        self.log("Service finished successfully", true)?;
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
    * * Service process exited on a code (even if the code is 0),
    * * Failed to wait for the process
    */
  pub fn supervise(rawName: impl AsRef<str>, mut process: Child) -> Result<()>
  {
    use super::power::POWER_OFF_READY;

    let name = rawName.as_ref();
    /*
     * Make sure if we receive an error, but the init system has been told to
     * power-off, that we don't actually return the error, as a service being
     * killed during power-off is normal behavior
     */
    macro_rules! err
    {
      ($error: expr) =>
      {
        if (POWER_OFF_READY.get() == Some(&true))
        {
          // Don't trigger an error if powering off - we want the service to be killed
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
        err!(Error::ServiceDown.trace(if let Some(code) = status.code()
        {
          format!("Service {name} has died: {code}")
        }
        else {
          format!("Service {name} has died: code is unavailable!")
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
    guard!(self.state == Down => Error::ServiceAccess.trace("Service is down, cannot get PID"));

    if let Dead(status) = self.state && let Some(code) = status.code()
    {
      return Ok(code.cast_unsigned())
    }

    match (self.process.get())
    {
      Some(i) => Ok(i.id()),
      None => Err(Error::ServiceAccess.into())
    }
  }
}

impl Sandbox
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

  fn wardenArgs(mut self, container: Box<str>, cmdArgs: &mut VecDeque<Box<str>>) -> Result<Vec<Box<str>>>
  {
    use std::{path::Path, mem::take};

    let mut args = Vec::<Box<str>>::new();

    // Vast majority of services will require this
    if (self.bindSystemFs.unwrap_or_default())
    {
      args.push(Box::<str>::from("--mount-system-fs"));
    }
    // For services such as elogind or lightdm which use the dbus daemon
    if (self.bindDbus.unwrap_or_default())
    {
      args.push(Box::<str>::from("--dbus"));
    }

    // We can take the flags here because we know they won't be used for anything else
    for flag in (take(&mut self.flags))
    {
      args.extend([Box::<str>::from("--flag"), flag]);
    }

    for bind in (&self.import)
    {
      let file: &Path = AsRef::<str>::as_ref(bind).as_ref();

      // Binding directories & files have different implementation due to creating parent dirs/files
      if (file.is_dir())
      {
        args.push(Box::<str>::from("--bind-dir"));
      }
      else if (file.is_file())
      {
        args.push(Box::<str>::from("--bind-file"));
      }
      else {
        return Err(Error::ServiceSandboxInit.trace(format!("No such file or directory: {bind}")))
      }

      args.push(bind.to_owned());
    }

    let exec = cmdArgs.pop_front().ok_or(Error::ServiceSandboxInit.trace("No executable in args"))?;

    // Tell warden where our root container is
    args.push(container);
    // This is the executable that warden will run
    args.push(exec);
    // `.make_contiguous()` moves all the items at the end of this deque to the front
    args.extend_from_slice(cmdArgs.make_contiguous());

    Ok(args)
  }
}

impl Logger
{
  #[must_use]
  pub const fn new(name: Box<str>, shout: bool, file: fs::File) -> Self
  {
    Self { name, shout, line: 0, file, reader: None }
  }

  // Watch the reader for the process for updates & then send them to the service's log
  /**
    * # Errors
    *
    * * Logger doesn't have a reader (this will only happen for services with a `RunOnce` pattern),
    * * Failed to read bytes from the reader,
    * * Failed to add new line to the log (in `self.log(_, _)`)
    */
  pub fn watch(&mut self) -> Result<()>
  {
    use std::{thread::sleep, time::Duration, io::{BufRead, BufReader}, mem::take};

    let interval = Duration::from_millis(oncelock!(&TARGET)?.serviceTickInterval);
    // Using BufReader for this kind of thing is just more efficient (thanks clippy!)
    let mut logReader = BufReader::new(self.reader.take().ok_or(Error::Unknown.trace("Logger missing!"))?);

    // Buffer of all the (hopefully) UTF-8 bytes this service will provide
    let mut content = Vec::new();

    loop {
      /*
       * Wait for the provided time for each loop iteration (default is 100ms, so 1/10th a second),
       * without this we would use alot of CPU %
       */
      sleep(interval);

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
    let timeNow = SystemTime::now().duration_since(UNIX_EPOCH)
                                    .into_trace(Error::Time)?
                                    .as_millis()
                                    .to_string();

    // Send timestamp to log
    contents.extend_from_slice(timeNow.as_bytes());

    // This byte is how ktctl can differentiate messages from init & service
    if (fromInit)
    {
      contents.push(INIT_LOG_ENTRY);
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
        log!(format!("{marker} {}({}):{} {line}", Colour::BOLD, self.name, Colour::RESET));
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
      _ => Err(io::Error::new(io::ErrorKind::InvalidInput, format!("Expected standard/forking/oneshot service byte, got: {input}")))
    }
  }
}
