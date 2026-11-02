/*!
 * Service implementation for the init system backend
 */

// Internal modules for more complex implementations
modules! {
  #[internal]
  pub mod logger;
  #[internal]
  pub mod sandbox;
  #[internal(pub)]
  mod lifetime;

  mod cache;
}

use std::{fs, fmt, io, path::PathBuf, os::unix::fs::PermissionsExt, collections::VecDeque,
            marker::PhantomData, sync::{Arc, OnceLock}, process::{Command, Child, ExitStatus}};

// Objects
use crate::{init::console::prelude::*, BoxedStr, DynamicOwnership, AnyStr, AsyncBlock};
// Constants
use crate::{init::target::TARGET, PREFIX};

use serde::Deserialize;

oncelock! {
  ///
  /// Vector of standard service handles, initialized by init
  ///
  /// Every standard service ran by the init system is included
  ///
  /// # Usage
  ///
  /// This is later used by [`poweroff`], which calls [`down`] for each service to make sure they are
  /// killed gracefully before continuing shutdown
  ///
  /// [`poweroff`]: crate::init::power::poweroff
  /// [`down`]: Service::down
  ///
  pub static STANDARD_SERVICES: OnceLock<Vec<RootService>>;
}

/// [Pattern] dependent mutable fields, as a major part of the [Service] structure
#[derive(Debug, Default)]
pub enum ServiceBody
{
  /// State of the service before `up` function is called
  #[default]
  PreExecution,
  /// Standard pattern services have unique fields
  Standard {
    /// Process's ID, set once only by the `up` function
    pid: OnceLock<u32>,
    /// This is made true by the `up` function
    up: bool,
    /**
      * The spawned child process of the init system. This value is only set once the process is
      * initialized by `up`. If this service is standard, it gets moved to the supervisor task, which
      * constantly monitors the process to make sure it never exits (daemon-style execution)
      */
    process: DynamicOwnership<Child, owner_id::SupervisorTask>,
    /**
      * The service's logger instance (see definition in [`logger`]). Similarly to the process field,
      * this is set by `up`, and also moved to the logger task if this is a standard service. This task
      * interprets the process's stderr and relays it back to the logfile (if undesired,
      * [`ServiceConfig::logger`] can be set to false)
      */
    log: DynamicOwnership<Logger, owner_id::LogWatchTask>
  },
  /*Forking {
    parent_pid: u32
  },*/
  /// [`RunOnce`] services are relatively simple compared to standard ones
  RunOnce {
    log: OnceLock<Logger>,
    code: OnceLock<ExitStatus>
  }
}

/// Fixed, public values defined in the service's configuration file
#[derive(Debug)]
pub struct ServiceConfig
{
  /// This will shared across threads, so an Arc is most suitable
  pub name: Arc<BoxedStr>,
  /// Prevents service errors from causing init errors (& subsequent execution interruption)
  pub optional: bool,
  /// The pattern of execution the service follows (see [`Pattern`])
  pub pattern: Pattern,
  /// Controls whether a logger task is setup for this service (see [`logger::Logger`])
  pub logger: bool,
  // Concatenate log entries to the init's master log (will show up on the init's tty)
  shout: bool,
  // Sandbox configuration (see `sandbox.rs` for definition)
  sandbox: Option<Sandbox>,
  // Create a run folder for this service
  run_dir: Option<RunDirectory>,
  // Executable + optional arguments, utilizing `VecDeque` for optimized front-end `pop` usage
  exec: VecDeque<BoxedStr>
}

/// Service monolith
/**
  *
  * Structure which contains configuration & the execution handle of the service. This should follow a
  * fixed lifetime depending on the service's pattern (see [`up`] for standard services & [`ServiceBody`]
  * for the backend implementation).
  *
  * # Initialization
  *
  * Services are sourced ([`try_source`]) in the kickit binary first. This is done so we can weed out any
  * kind of bad configurations, seperating from actual service initialization errors.
  *
  * After all the services have been sourced, the kickit binary starts each one-by-one ([`up`]).
  *
  * Finally, the service handles returned by the start method are moved to the [`STANDARD_SERVICES`]
  * oncelock. Here they remain untouched until shutdown, where the handles are accessed to kill the
  * services with [`down`].
  *
  * # Example
  *
  * ```
  * use std::thread::park;
  * # use kickit::init::{service::Service, console::prelude::*};
  *
  * #[tokio::main]
  * async fn main()
  * {
  *   // `try_source` method attempts to access /usr/lib/kickit/service/hellod.toml
  *   let mut hello_world_daemon = Service::try_source("hellod").handle();
  *
  *   // The `start` method causes some values to be moved, leaving us with only a handle
  *   let _ = hello_world_daemon.start().await.handle();
  *
  *   // Now, the process runs as a seperate tokio task. The init blocks until shutdown.
  *   loop {
  *     park();
  *   }
  * }
  * ```
  *
  * **Note:** services can only be initialized in the kickit init binary
  *
  * [`up`]: #method.up
  * [`try_source`]: #method.try_source
  * [`down`]: #method.down
  */
#[derive(Debug)]
pub struct Service<Lifetime: ServiceLifetime>
{
  /**
    * Fixed values defined in the service's configuration (parsed from TOML)
    *
    * These are never modified, and may be cloned & also copied if needed.
    */
  pub cfg: ServiceConfig,
  // Unlike the config, these fields will be modified and as such are mutable
  body: ServiceBody,

  _impl: PhantomData<Lifetime>
}

/// Regular kind of service used by kickit
pub type RootService = Service<lifetime::Root>;

/// Standard service process supervisor
/**
  *
  * The final destination of a standard [`Service`], the process is constantly monitored - to make sure it
  * never exits.
  *
  * The instance is initialized from [`up`], running asynchronously as a tokio task.
  *
  * # Behavior
  *
  * * The underlying main method used by the supervisor is [`wait`]. This blocks until the child process
  *   exits, at which point the supervisor assesses if the service is optional. If it is, a warning is
  *   given, if not, a global init error is thrown. [^1]
  *
  * * When [`poweroff`] kills the services provided in [`STANDARD_SERVICES`], this will cause the
  *   supervisor to throw an error under normal circumstances. To mitigate this, the value
  *   [`POWER_OFF_READY`] is set to true before services are killed. The supervisor exits normally
  *   without throwing any errors.
  *
  * [^1]: See the [`supervise`] function for reference.
  *
  * [`up`]: Service::up
  * [`wait`]: std::process::Child::wait
  * [`poweroff`]: crate::init::power::poweroff
  * [`POWER_OFF_READY`]: crate::init::power::POWER_OFF_READY
  * [`supervise`]: ServiceLifetime::supervise
  */
pub struct Supervisor
{
  name: Arc<BoxedStr>,
  process: Child
}

/**
  * Service execution pattern
  *
  * # Implementation
  *
  * Some operations are pattern-specific. For example, a supervisor & logger may only be created for a
  * [`Standard`] service.
  *
  * This additionally changes how the service is started, standard services run like a daemon, so are
  * ran as a tokio task. On the other hand, run-once services run in tandem to kickit's execution,
  * which waits for it to finish (with a timeout, which is 5 seconds by default).
  *
  * # Representation
  *
  * Each pattern includes a byte `u8` representation for usage in the cache file. See [`ktctl::service`]
  * for how this is parsed, and [`Service`] for its creation.
  *
  * [`ktctl::service`]: crate::ktctl::service
  */
#[derive(Deserialize, PartialEq, Eq, Clone, Copy, Debug, Default)]
#[repr(u8)]
pub enum Pattern
{
  /**
    * Service runs in the background (daemon-style), and should never exit. kickit supervises these services
    * with a seperate supervisor instance to make sure they don't exit. If they do, an error will be thrown
    * (if this service is optional, however, this won't be a fatally handled error)
    */
  #[default]
  Standard = 0xa4,
  /**
    * Service spawns children processes & then exits, this is initialized similarly to a standard service
    * but without a supervisor
    */
  Forking = 0xef,
  /// Service runs in tandem with kickit's execution, which waits for it to finish before continuing
  RunOnce = 0xe9
}
use Pattern::{Standard, Forking, RunOnce};

// Optional configuration provided to create a directory in /run for this service (useful, for example, with dbus)
#[derive(Deserialize, PartialEq, Eq, Clone, Debug)]
struct RunDirectory
{
  // The name of the directory in /run
  name: Box<str>,
  // Group ID (GID)
  group: u32,
  // User ID (UID)
  owner: u32,
  // Access permissions & special mode flags in bits (must convert from octal to decimal)
  mode: u32
}

/**
  * [`OwnerIdentity`] implementations for supervisor & log watching task (for [`DynamicOwnership`])
  *
  * [`OwnerIdentity`]: crate::OwnerIdentity
  */
pub mod owner_id
{
  /// The [Supervisor] takes ownership of the service process for standard services
  /**
    *
    * **Note:** this is not the supervisor instance itself, just a marker for the transfer of ownership
    * to the supervisor
    *
    * [Supervisor]: super::Supervisor
    */
  #[derive(PartialEq, Eq, Copy, Clone, Debug, Default, derive_more::Display)]
  pub struct SupervisorTask;

  /// The [Logger] runs asynchronously to the process, for standard services
  /**
    *
    * **Note:** this is not the logger instance itself, just a marker for the transfer of ownership to
    * the logger
    *
    * [Logger]: super::logger::Logger
    */
  #[derive(PartialEq, Eq, Copy, Clone, Debug, Default, derive_more::Display)]
  pub struct LogWatchTask;

  impl crate::OwnerIdentity for SupervisorTask {}
  impl crate::OwnerIdentity for LogWatchTask {}
}

/*
 * Construction methods
 * ====================
 */
impl<L: ServiceLifetime> Service<L>
{
  /// Source service from its configuration file
  /**
    * # Definition
    *
    * Services are defined in a single TOML file, located in the service directory of the [`PREFIX`]
    * (default: `/usr/lib/kickit`). Refer to
    * [the documentation](<https://github.com/h4dynn/kickit/blob/main/docs/Service/config.md>) for more
    * information on the required & optional values
    *
    * # Errors
    *
    * * This function excepts the configuration file exists, is readable and contains valid TOML
    *   formatted configuration (parsing is handled by the [`toml`] crate).
    *
    * * It additionally checks to make sure the specific executable file exists.
    */
  pub fn try_source(service_name: impl AnyStr) -> Result<Self>
  {
    macro_rules! set
    {
      ($config: tt [Option<$($optional: tt),*>, $($any: tt),*]) =>
      {
        $(
          // Fallback to default value for this type if not provided in the config file
          let $optional = $config.$optional.unwrap_or_default();
        )*

        $(
          let $any = $config.$any;
        )*
      };
    }

    /*
     * `toml` crate uses a deserializable struct for value collection from the configuration. This includes
     * a lot of `Option` fields, we fallback to default value as needed to get the final configuration
     */
    #[derive(Deserialize)]
    struct TomlConfig
    {
      optional: Option<bool>, shout: Option<bool>, pattern: Option<Pattern>, no_logger: Option<bool>,
      run_dir: Option<RunDirectory>, sandbox: Option<Sandbox>, exec: VecDeque<BoxedStr>
    }

    let name = service_name.as_str_ref();
    // Resolves to `/usr/lib/kickit/service/<name>.toml`- services are defined in a single config similar to systemd
    let config_path = path!(PREFIX, "service", format!("{name}.toml"));

    guard!(!config_path.is_file() => Error::FileNotFound.trace("service not found").subject(name));

    // Read TOML configuration contents
    let config_toml = fs::read_to_string(&config_path).into_trace(Error::ServiceParse).subject(name)?;
    // Parse the contents
    let config: TomlConfig = toml::from_str(&config_toml).into_trace(Error::ServiceParse).subject(name)?;

    let exec = PathBuf::from(&*config.exec[0]);

    // Check the service's executable actually exists on filesystem
    guard!(exec.metadata().is_err() => Error::FileNotFound.trace("service executable missing").subject(name));

    // Optional values will fallback to default if not provided (default for bool is false)
    set!(config[Option<optional, shout, pattern>, no_logger, run_dir, sandbox, exec]);

    let cfg = ServiceConfig
    {
      name: Arc::new(name.into()), optional, shout, sandbox, run_dir, pattern, logger: no_logger.is_none(), exec
    };

    Ok(Self::from_config(cfg))
  }

  #[must_use]
  pub fn from_config(cfg: ServiceConfig) -> Self
  {
    Self { cfg, body: ServiceBody::PreExecution, _impl: PhantomData }
  }
}

/*
 * Miscellaneous helper methods
 * ============================
 */
impl RootService
{
  pub(crate) fn run_path(name: impl fmt::Display, file: impl fmt::Display) -> PathBuf
  {
    PathBuf::from(format!("/run/kickit/service/{name}/{file}"))
  }

  pub fn is_up(&self) -> bool
  {
    matches!(self.body, ServiceBody::Standard { up: true, .. }) || !self.body.is_pre_exec()
  }

  /// Take ownership of this service's logger instance (wrapper for [`DynamicOwnership::take`])
  /**
    * # Usage
    *
    * This method is used as a part of the [`up`] implementation. It is not intended to be called directly
    * by anyone else.
    *
    * # Errors
    *
    * Expects the service to own the logger, it cannot have already been taken.
    */
  pub(crate) fn logger(&mut self) -> Result<Logger>
  {
    if let ServiceBody::Standard { log, .. } = &mut self.body
    {
      log.take_as_default().into_trace(Error::ServiceAccess).subject(&self.cfg.name)
    }
    else {
      Err(Error::ServicePatternMismatch.trace("async logging is standard service only").subject(&self.cfg.name))
    }
  }

  /// Take ownership of this service's executable process (wrapper for [`DynamicOwnership::take`])
  /**
    * # Usage
    *
    * This method is used as a part of the [`up`] implementation. It is not intended to be called directly
    * by anyone else.
    *
    * # Errors
    *
    * Expects the service to own the process, it cannot have already been taken.
    */
  pub(crate) fn supervisor(&mut self) -> Result<Supervisor>
  {
    if let ServiceBody::Standard { process, .. } = &mut self.body
    {
      // Create a new reference count to the initial name allocation (from [`try_source`])
      let name = Arc::clone(&self.cfg.name);
      // 
      let process = process.take_as_default().into_trace(Error::ServiceAccess).subject(&self.cfg.name)?;

      Ok(Supervisor { name, process })
    }
    else {
      Err(Error::ServicePatternMismatch.trace("only standard services can be supervised").subject(&self.cfg.name))
    }
  }
}

/*
 * Parsing & formatting methods
 * ============================
 */
impl RootService
{
  // Convert provided args to executable & arguments based on service options
  fn args(&mut self) -> Result<(PathBuf, VecDeque<BoxedStr>, Option<Vec<BoxedStr>>)>
  {
    let mut args = self.cfg.exec.clone();

    /*
     * Convert the sandbox options into arguments for warden to then handle. The reason why we don't just
     * start the sandbox from here is because using `unshare` will cause the init to also unshare (bad!),
     * so we use the warden executable to start a new, seperate command which can be safely unshared
     */
    if let Some(mut sandbox_profile) = self.cfg.sandbox.take()
    {
      let mut final_args = VecDeque::<BoxedStr>::new();

      let container_dir = Self::run_path(&self.cfg.name, "container");
      // Setup the container rootfs
      sandbox_profile.setup(&container_dir)?;

      let exec = path!(PREFIX, "warden");
      let exports = sandbox_profile.export.take();

      // Add the generated arguments from our options
      final_args.extend(sandbox_profile.cli_args(container_dir.display().to_string().into(), &mut args)?);

      Ok((exec, final_args, exports))
    }
    else {
      // First field will be the executable path
      let exec = args.pop_front().ok_or(Error::ServiceSandboxInit.trace("no executable").subject(&self.cfg.name))?;

      Ok((PathBuf::from(exec.as_ref()), args, None))
    }
  }
}

/**
  * Lifetime event methods
  * ======================
  */
impl RootService
{
  /// This method will take ownership of the service, and only return a handle to it if standard
  /**
    * # Standard lifetime
    *
    * * 1. All services defined in the target are initialized by `initServices(..)` (method calls
    *      [`try_source`] for each service),
    *
    * * 2. This method is called by the init system,
    *
    * * 3. The process is started from the service's executable, and then the service is broken up into
    *      its logger and [`Supervisor`],
    *
    * * 4a. The logger reads from the process's `stdout` and `stderr`, relaying each entry (seperated by
    *   newlines) to the log file,
    *
    * * 4b. The supervisor watches the process, waiting for it to exit. If it ever exits, it will throw
    *   an init error if the service is not optional, or just a warning if not,
    *
    * * 5. What is left of the service is moved into [`STANDARD_SERVICES`], where it will be accessed upon
    *      system shutdown to gracefully kill the service ([`down`])
    *
    * # Errors
    *
    * * Failed to start the service's main process (see `Service::up` errors)
    *
    * * Failed to start the logger (watches stdin and stderr, see `Service::logger`)
    *
    * * Failed to start the process supervisor (see `Service::supervisor`)
    *
    * [`try_source`]: #method.try_source
    * [`down`]: #method.down
    */
  // See `up_inner` and `start_process` further on for the implementation
  #[inline]
  pub async fn up(mut self) -> Result<Option<Self>>
  {
    use std::{fs::File, thread::sleep, time::Duration};
    use crate::{console::{AssessError, HandleError}, state};

    // Don't continue starting services if the init is not ready
    while (!state!().is_ok())
    {
      // Repeat the check every half second to avoid high CPU % usage
      sleep(Duration::from_millis(500));
    }
    // This method should only be called once, and afterwards should run interrupted with no more calls
    guard!(self.is_up() => Error::ServiceUp.trace("already up!").subject(self.cfg.name));

    // The main method for starting up the service's process
    self.up_inner().await.assess(self.cfg.optional)?;

    // Use default permissions, the cache contains no sensitive information, so can be public (0o755)
    let mut cache_file = File::create_new(Self::run_path(&self.cfg.name, "config")).into_trace(Error::ServiceAccess)?;
    // Construct cache & save it to the file
    self.cache().write_as_data(&mut cache_file)?;

    if (self.cfg.pattern != RunOnce)
    {
      // We move the log & the supervisor, they will not be needed in service after this
      let (mut log, mut supervisor) = (self.logger()?, self.supervisor()?);

      // Watch the log for updates
      tokio::spawn(async move { log.watch().assess(self.cfg.optional).handle() });

      // Don't supervise a forking service since its expected that it exits
      if (self.cfg.pattern == Standard)
      {
        debug_cc!(&self.body);
        // Supervise the service's daemon to make sure it doesn't die
        tokio::spawn(async move { Self::supervise(&mut supervisor).await.assess(self.cfg.optional).handle() });
        // Return what is left of the service (some of its contents have been moved, like logger & process)
        return Ok(Some(self))
      }
    }
    Ok(None)
  }

  /// Gracefully kill a service, in the init process this is only called once by `power::poweroff`
  /**
    * # Errors
    *
    * * Service is already down
    *
    * * Service's process hasn't been moved out (supervisor is not running, which it should be)
    *
    * * Failed to get the PID of the service (see `Service::pid`)
    *
    * * Failed to kill the service (see `nix::sys::signal::kill`)
    */
  pub fn down(&self) -> Result<()>
  {
    use crate::{init::console::warn, console::{Colour, HandleError}};
    use nix::{unistd::Pid, sys::signal::{kill, Signal}, errno::Errno};

    guard!(!self.is_up() => Error::ServiceDown.trace("service is already dead"));

    if let ServiceBody::Standard { process, .. } = &self.body
    {
      // Process should have been moved out to the supervisor at this point
      guard!(process.is_owned() => Error::ServiceDown.trace("service supervisor process ownership mismatch"));
    }
    else {
      return Err(Error::ServicePatternMismatch.trace("service is not standard").subject(&self.cfg.name))
    }

    // nix wants a signed integer, even though a PID is never negative (why???)
    let pid: i32 = self.pid()?.cast_signed();

    // First try killing with SIGQUIT
    match (kill(Pid::from_raw(pid), Some(Signal::SIGQUIT)))
    {
      Ok(..) => (),
      // Process is already dead, no need to kill it
      Err(Errno::ESRCH) => warn!("Service is already down"),
      Err(err) =>
      {
        warn!("Failed to kill service, attempting force kill: {err}");
        // If that doesn't work we use SIGKILL, and if that doesn't work we can't do anything but warn really
        kill(Pid::from_raw(pid), Some(Signal::SIGKILL)).into_trace(Error::Unknown).or_warn();
      }
    }

    Ok(())
  }

  /// Resolve process ID (included in standard service's body after being started)
  /**
    * # Errors
    *
    * * Service isn't running
    *
    * * Service has a `RunOnce` pattern
    *
    * * Service's process couldn't be found
    */
  pub fn pid(&self) -> Result<u32>
  {
    match (&self.body)
    {
      ServiceBody::Standard { up, pid, .. } =>
      {
        guard!(!up => Error::ServiceAccess.trace("service is down, cannot get PID").subject(&self.cfg.name));
        oncelock!(&pid).map(|p| *p)
      },
      _ => Err(Error::ServicePatternMismatch.trace("only standard services provide a PID").subject(&self.cfg.name))
    }
  }
}

/*
 * Implementation methods used for `up` function
 * =============================================
 */

impl RootService
{
  /**
    * The underlying method used by `up`, this does majority of the work in handling the service's process
    * execution, logger initialization & caching. The `up` method will start the supervisor & logger task
    * if the service is standard, jobs which are not this function's responsibility
    *
    * # Errors
    *
    * * Service is already running
    *
    * * Couldn't spawn or start a process for the service
    *
    * # Panics
    *
    * * Failed to read from `std::io::Lines` iterator (error in `std::io::BufRead::read_line`)
    */
  #[inline]
  async fn up_inner(&mut self) -> Result<()>
  {
    use nix::unistd::{chown, Uid, Gid};
    use std::fs::Permissions;

    // Construct the default service body dependent on the service's execution pattern
    let mut exec_body = ServiceBody::default_from(self.cfg.pattern);

    // Logger instance construction
    if (self.cfg.logger)
    {
      // Create the log file with empty Zstandard contents & set its proper permissions
      let inst = Logger::init(Arc::clone(&self.cfg.name), self.cfg.shout)?;

      match (&mut exec_body)
      {
        // Standard services uses the dynamic ownership wrapper, for when it is moved to the log watcher
        ServiceBody::Standard { log, .. } => log.init(inst),
        // OnceLock is used for run-once services as they stay on this thread
        ServiceBody::RunOnce { log, .. } => oncelock! { log = inst }?,
        ServiceBody::PreExecution => unreachable!()
      }
    }

    // Create the run folder, if one is given
    if let Some(RunDirectory { ref name, owner, group, mode }) = self.cfg.run_dir
    {
      let path = path!("/run", name as &str);
      fs::create_dir(&path).into_trace(Error::ServiceAccess).subject(name)?;

      // Change the owner (UID) & group (GID)
      chown(&path, Some(Uid::from_raw(owner)), Some(Gid::from_raw(group))).into_trace(Error::ServiceAccess)?;
      // Set the permissions mode
      fs::set_permissions(path, Permissions::from_mode(mode)).into_trace(Error::ServiceAccess)?;
    }

    // Discard the placeholder pre-execution body & replace it with the relevant one
    self.body = exec_body;

    let _ = self.start_process().await?;

    Ok(())
  }

  async fn start_process(&mut self) -> Result<u32>
  {
    use std::{fs::Permissions, time::Duration, io::{BufReader, BufRead}};
    use tokio::time::timeout;

    /*
     * Opening a pipe here allows us to read both stderr/stdout as one (like in bash), since some services
     * will output logs to stdout & some to stderr
     */
    let (reader, writer) = std::io::pipe().into_trace(Error::Unknown)?;

    // The executable name we are going to call, plus its arguments and then exports if we are sandboxing
    let (exec, args, exports) = self.args()?;

    // Start the service's process & wait for it to finish
    let mut child = Command::new(exec).args(args.iter().map(AsRef::as_ref)).current_dir("/")
                        // Use the same instance of pipe writer for stderr & stdout (equivilant to `2>&1` in bash)
                        .stderr(writer.try_clone().into_trace(Error::Unknown)?).stdout(writer)
                        .spawn()
                        .into_trace(Error::ServiceUp).subject(&self.cfg.name)?;

    // Capture the PID before child process is taken
    let pid_int = child.id();

    /*
     * Based on the service's pattern we either spawn it in the background & watch it or run it in the
     * foreground and wait for it to finish
     */
    match (&mut self.body)
    {
      ServiceBody::Standard { up, process, pid, log, .. } =>
      {
        oncelock! { pid = pid_int }.subject(&self.cfg.name)?;
        // Transfer the process to our service
        process.init(child);

        if (self.cfg.logger)
        {
          let log_inst = oncelock!(&mut log).subject(&self.cfg.name)?;
          // Link the reader to the log
          log_inst.reader = Some(reader);

          log_inst.log(&LogEntry::Init("Started service"))?;
        }

        if let Some(ext) = exports
        {
          for export in (ext)
          {
            // Create a bind mount from container to the rootfs, mirroring files/directories
            Sandbox::externalize(&Self::run_path(&self.cfg.name, "container"), &export)?;
          }
        }

        *up = true;
      },
      ServiceBody::RunOnce { code, log } =>
      {
        // How long are we willing to wait for this service? (default: 5 seconds, see `init::target`)
        let maxWaitTime = Duration::from_secs(oncelock!(&TARGET)?.service_timeout);

        match (timeout(maxWaitTime, async { child.wait() }).await)
        {
          // wait method returned before timeout
          Ok(Ok(status)) =>
          {
            // Provide exit status to the service
            code.set(status).unwrap();

            guard!(!status.success() => Error::ServiceUp.trace(format!("process exited on error ({status})"))
                                                          .subject(&self.cfg.name));
          },
          Ok(Err(error)) => return Err(Error::ServiceUp.trace(error).subject(&self.cfg.name)),
          // Timeout was reached...
          Err(..) => return Err(Error::ServiceUp.trace("timed out waiting for exit").subject(&self.cfg.name))
        }

        if (self.cfg.logger)
        {
          let inst = oncelock!(&mut log).subject(&self.cfg.name)?;

          // A buffered read is the most efficient way to read line-by-line
          for line in (BufReader::new(reader).lines())
          {
            inst.log(&LogEntry::Service(&line.into_trace(Error::Format).subject(&self.cfg.name)?))?;
          }
          inst.log(&LogEntry::Init("Service finished successfully"))?;

          // Make logfile read-only as the process has finished
          inst.file.set_permissions(Permissions::from_mode(0o400)).into_trace(Error::ServiceAccess)?;
        }
      },
      ServiceBody::PreExecution => unreachable!()
    }

    Ok(pid_int)
  }
}

impl<LifetimeImpl: ServiceLifetime> Service<LifetimeImpl>
{
  pub(crate) async fn supervise(supervisor: &mut Supervisor) -> Result<AsyncBlock>
  {
    LifetimeImpl::supervise(supervisor).await
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
      _ => Err(io::Error::new(io::ErrorKind::InvalidInput, format!("expected valid pattern byte, got: {input}")))
    }
  }
}

impl ServiceBody
{
  pub fn is_pre_exec(&self) -> bool
  {
    // `PartialEq` is not implemented, so we use regular matching to resolve pre-execution state
    matches!(*self, Self::PreExecution)
  }

  #[must_use]
  pub const fn default_from(pattern: Pattern) -> Self
  {
    match (pattern)
    {
      Standard => Self::Standard { up: false, pid: OnceLock::new(), process: DynamicOwnership::new(),
                                    log: DynamicOwnership::new() },
      RunOnce => Self::RunOnce { code: OnceLock::new(), log: OnceLock::new() },
      Forking => todo!()
    }
  }
}
