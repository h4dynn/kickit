// kickit init process

#![allow(unused_parens)]
#![allow(non_snake_case)]

use std::{fs, fs::File, io::Write, path::PathBuf, process};
use tokio::task;
use nix::unistd::getuid;
use kickit::{oncelock,
              console::Colour, console::{ErrorResult, ReturnError, HandleError},
              init::{service::Service, target::TARGET, PID, console::{Error, Result, status, stall, QUIET}}};

/*
 * The way dhat does its profiling in heap mode, is that the profiler
 * will wait until the main() function has finished. However, in our case,
 * since we are an init system, main() blocks indefinitely. So we have to set
 * a specific amount of time to wait for the init to setup everything and then
 * manually drop the profiler.
 */
#[cfg(feature = "dhat_heap")]
const DHAT_HEAP_WAIT_TIME_MINS: u64 = 5;

trait AssessError<OkType>
{
  // Do not error if optional is true
  fn assess(self, optional: bool) -> Option<OkType>;
}

impl<OkType> AssessError<OkType> for Result<OkType>
{
  fn assess(self, optional: bool) -> Option<OkType>
  {
    if (optional)
    {
      // We do not want to return an error, so only warn
      self.or_warn()
    }
    else {
      Some(self.handle())
    }
  }
}

trait StartService
{
  async fn start(self) -> Result<Option<Service>>;
}

impl StartService for Service
{
  /**
    * Lifetime of a service with Standard pattern
    * ===========================================
    *
    * - All services defined in the target are initialized by `initServices(..)`,
    *
    * - Each service is started by this method,
    *
    * - Once started by `Service::up(..)`, the service is broken up into its logger &
    *    supervisor,
    *
    * - The logger watches the service's stderr & stdout streams and reports them
    *    (if `logger` is set to true),
    *
    * - The supervisor makes sure the service doesn't die, and if it does and is
    *    not optional, throw a global error,
    *
    * - What is left of the service is moved into `SERVICES`, where it will be used
    *    when shutting down to stop the service
    */
  #[inline]
  async fn start(mut self) -> Result<Option<Service>>
  {
    use kickit::{init::service::Pattern::{Standard, RunOnce}, state};

    status!("Starting service: {}", self.name);

    // Don't continue starting services if we are stalled
    if (!state!().is_ok())
    {
      stall!();
    }

    if let Err(trace) = self.up().await
    {
      // If this service is optional we only need to warn not abort
      if (self.optional)
      {
        trace.warn();
      }
      else {
        return Err(trace)
      }
    }

    if (self.pattern != RunOnce)
    {
      // We move the log & the supervisor, they will not be needed in service after this
      let mut log = self.logger()?;
      let mut supervisor = self.supervisor()?;

      // Watch the log for updates
      task::spawn(async move { log.watch().assess(self.optional) });

      // Don't supervise a forking service since its expected that it exits
      if (self.pattern == Standard)
      {
        // Supervise the service's daemon to make sure it doesn't die
        task::spawn(async move { supervisor.supervise().assess(self.optional) });
        // Return what is left of the service (some of its contents have been moved, like logger & process)
        return Ok(Some(self))
      }
    }
    Ok(None)
  }
}

/**
  * # Errors
  *
  * - if /run/kickit exists (kickit is already running),
  * - if not ran as root user,
  * - if not ran as the init process (pid 1)
 **/
#[inline]
fn sanity() -> Result<()>
{
  use kickit::{console::guard, init::console::warn};

  // If /run/kickit is already there then another kickit is running
  guard!(fs::metadata("/run/kickit").is_ok() => Error::AlreadyRunning.into());

  // Make sure we are running as the root user
  guard!(!getuid().is_root() => Error::NotRoot.into());

  if let Some(pid) = PID.get() && (pid.is_none())
  {
    if (cfg!(feature = "bypass_init_check"))
    {
      warn!("Bypassing init checks!");
    }
    else {
      guard!(process::id() != 1 => Error::NotInit.into());
    }
  }

  Ok(())
}

/*
 * /run/kickit
 * |
 * ├─ service        ==> All services which have been initialised by kickit
 * |  |
 * |  └─ [NAME]
 * |     ├─ container   ==> The service's sandbox container (only if sandbox option is given)
 * |     ├─ config      ==> The cached configuration for this service (see `ktctl::service::CacheInstance`)
 * |     └─ log         ==> The service's log (only accessible to root)
 * |
 * ├─ private        ==> Sockets accessible to root user only
 * |  |
 * |  ├─ io.Power   ==> Socket that handles poweroff/reboot requests
 * |  └─ io.Log     ==> Socket that provides the init's master log
 * |
 * └─ io.Core        ==> The socket that ktctl uses to gather info like state, version & target
 */
#[inline]
fn setupRunFs(services: &[String], debugDump: bool) -> Result<()>
{
  use std::{fs::{Permissions, create_dir, create_dir_all, set_permissions}, os::unix::fs::{symlink, PermissionsExt}};
  use kickit::path;

  if (!debugDump)
  {
    // Normal behaviour - create runfs as a folder
    create_dir("/run/kickit").into_trace(Error::RunFsFail)?;
  }
  // A debug dump will create a folder in /var/log/kickit and symlink it to the runfs
  else if (cfg!(debug_assertions) && debugDump)
  {
    use std::time::{SystemTime, UNIX_EPOCH};

    // Use the timestamp as an identifier for said folder
    let time = SystemTime::now().duration_since(UNIX_EPOCH).into_trace(Error::Time)?
                  .as_millis()
                  .to_string();

    let dumpDir = path!("/var/lib/kickit", time);

    // Recursively create our dump directory
    create_dir_all(&dumpDir).into_trace(Error::RunFsFail)?;

    // Create a symlink so it acts as if it is a directory
    symlink(dumpDir, PathBuf::from("/run/kickit")).into_trace(Error::RunFsFail)?;
  }

  create_dir("/run/kickit/service").into_trace(Error::RunFsFail)?;
  create_dir("/run/kickit/private").into_trace(Error::RunFsFail)?;

  // Make the private folder, well, private
  set_permissions("/run/kickit/private", Permissions::from_mode(0o600)).into_trace(Error::RunFsFail)?;

  for upService in (services)
  {
    create_dir(path!("/run/kickit/service", upService)).into_trace(Error::RunFsFail)?;
  }

  Ok(())
}

#[inline]
fn mountSysFilesystems() -> Result<()>
{
  use kickit::init::mount::{Flag::{NoSuid, NoDev, NoExec, Remount, Private}, mount, unmount, mounted, flags};

  macro_rules! mount
  {
    ($from: tt, $to: tt, $fsType: tt, $flags: expr) =>
    {
      // Check if each destination is already mounted, and if so unmount it
      if !(mounted($to)? && unmount($to, None).is_err())
      {
        mount(Some($from), $to, Some($fsType), $flags, None)?
      }
    }
  }

  mount!("proc", "/proc", "proc", flags![NoSuid, NoDev, NoExec]);
  mount!("sysfs", "/sys", "sysfs", flags![NoSuid, NoDev, NoExec]);
  mount!("dev", "/dev", "devtmpfs", flags![NoSuid]);
  mount!("tmpfs", "/run", "tmpfs", flags![NoSuid]);
  mount!("tmpfs", "/tmp", "tmpfs", flags![NoSuid, NoDev]);
  // Remount the rootfs as read-write if booted with `ro` cmdline argument
  mount(None, "/", None, flags![Private, Remount], None)?;

  // No errors yippie
  Ok(())
}

// Ran before starting to catch any potential early errors in a service config
#[inline]
fn initServices(services: &Vec<String>) -> Result<Vec<Service>>
{
  let mut initServices: Vec<Service> = Vec::new();

  for service in (services)
  {
    initServices.push(Service::init(service)?);
  }

  Ok(initServices)
}

// Use the dhat allocator for heap analysis, slower than regular allocator
#[cfg(feature = "dhat_heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

#[tokio::main]
async fn main()
{
  use std::{thread::park, process::id as pid, env::args};
  use kickit::{init::{target, cmdline, socket::Open, service::STANDARD_SERVICES, mount::mountFstabEntries}, socket};

  macro_rules! open_socks
  {
    ($($sock: path),*) =>
    {{
      $(
        task::spawn(async move { $sock.open_sock().await.handle(); });
      )*
    }};
  }

  #[cfg(feature = "dhat_heap")]
  let profiler = dhat::Profiler::new_heap();

  let sysArgs: Vec<String> = args().collect();
  // This will only be Some if we are not running as the init process
  let pid = (sysArgs.len() > 1 && &sysArgs[1] == "--no-init").then_some(pid());

  oncelock! { PID = pid }.handle();

  sanity().handle();
  status!("kickit {}", kickit::VERSION);

  // Respect the quiet argument if provided
  oncelock! { QUIET = cmdline("quiet") == Ok(None) && pid.is_none() }.handle();

  // Assume the test target on debug builds
  #[cfg(debug_assertions)]
  let target = "test";

  // Check if cmdline parameter 'init.target=XXX' has been provided, if not use the default "system" target
  #[cfg(not(debug_assertions))]
  let target = cmdline("init.target").unwrap_or(Some(String::from("system")))
                    .ok_or(Error::BadCmdline.trace("init.target requires a target name to be provided!"))
                    .handle();

  status!("Target: {target}");

  /*
   * By having our target in a OnceLock we ensure that others parts of the init
   * can access it hassle-free for e.g. logging or services
   */
  oncelock! { TARGET = target::source(target.to_owned()).handle() }.handle();

  // We've just set it so this shouldn't fail
  let target = TARGET.get().ok_or(Error::Unknown.trace("target is inaccessible")).handle();

  status!("Initialising services");
  let services = initServices(&target.services).handle();

  // If we are running alongside another init these things should've already been done
  if (pid.is_none())
  {
    status!("Mounting system filesystems");
    mountSysFilesystems().handle();

    status!("Setting hostname");
    let mut hostname = File::create("/proc/sys/kernel/hostname").into_trace(Error::ProcFs).handle();
    hostname.write_all(target.hostname.as_bytes()).into_trace(Error::ProcFs).handle();

    // Now we can mount all the custom entries in /etc/fstab
    mountFstabEntries().handle();
  }

  status!("Setting up work directory");
  setupRunFs(target.services.as_slice(), target.debugDump).handle();

  // Open our sockets
  open_socks!(socket::Core, socket::Log, socket::Power);

  // We will set `SERVICES` to this after all services have been started
  let mut standardServices = Vec::new();

  for service in (services)
  {
    // Spawn the service's process, start its supervisor & log watcher
    if let Some(handle) = service.start().await.handle()
    {
      standardServices.push(handle);
    }
  }

  oncelock! { STANDARD_SERVICES = standardServices }.handle();

  // main() never exits so we never get a dhat analysis without explicitly dropping the profiler
  #[cfg(feature = "dhat_heap")]
  {
    use std::{thread::sleep, time::Duration};

    // Give some time for everything to be setup
    sleep(Duration::from_mins(DHAT_HEAP_WAIT_TIME_MINS));

    // Drop the profiler so we get the analysis
    drop(profiler);
  }

  // Block indefinitely
  loop {
    park();
  }
}
