// kickit init process

#![allow(unused_parens)]
#![allow(non_snake_case)]

use std::{fs, fs::File, io::Write, path::PathBuf, process};
use tokio::task;
use nix::unistd::getuid;
use kickit::{
  console::Colour, console::{ReturnError, HandleError},
  init::{console::{Error, ErrorResult, Result, StdResult, status, stall},
            service::Service, TARGET, TARGET_NAME, QUIET}, oncelock};

trait StartService
{
  fn start(self) -> Result<()>;
}

impl StartService for Service
{
  #[inline]
  fn start(mut self) -> Result<()>
  {
    use kickit::{init::service::Pattern::{Standard, Forking}, state};

    status!("Starting service: {}", self.name);

    // Don't continue starting services if we are stalled
    if (!state!().is_ok())
    {
      stall!();
    }

    if let Err(trace) = self.up()
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

    if (matches!(self.pattern, Standard | Forking))
    {
      task::spawn(async move { self.watch().handle() });
    }
    Ok(())
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
fn sanity(sideInit: bool) -> StdResult<(), Error>
{
  use kickit::{console::affirm, init::console::warn};

  // If /run/kickit is already there then another kickit is running
  affirm!(fs::metadata("/run/kickit").is_err(), Error::AlreadyRunning);

  // Make sure we are running as the root user
  affirm!(getuid().is_root(), Error::NotRoot);

  if (!sideInit)
  {
    if (cfg!(feature = "bypass_init_check"))
    {
      warn!("Bypassing init checks!");
    }
    else {
      affirm!(process::id() == 1, Error::NotInit);
    }
  }

  Ok(())
}

/*
 * /run/kickit
 * |
 * ├── service     -> All services which have been initialised by kickit
 * |   |
 * |   └── [NAME]
 * |       ├── pid    -> The service's process ID (Standard services only)
 * |       ├── exited -> The service's exit code (RunOnce services only)
 * |       └── log    -> The service's log (only accessible to root)
 * |
 * ├── private     -> Sockets accessible to root user only
 * |   |
 * |   ├── io.Power   -> Socket that handles poweroff/reboot requests
 * |   └── io.Log     -> Socket that provides the init's master log
 * |
 * └── io.Core     -> The socket that ktctl uses to gather info like state, version & target
 */
#[inline]
fn setupRunFs(services: &Vec<String>, debugDump: bool) -> Result<()>
{
  use std::{fs::Permissions, os::unix::fs::{symlink, PermissionsExt}};
  use kickit::path;

  if (!debugDump)
  {
    // Normal behaviour - create runfs as a folder
    fs::create_dir("/run/kickit").into_trace(Error::RunFsFail)?;
  }
  // A debug dump will create a folder in /var/log/kickit and symlink it to the runfs
  else if (cfg!(debug_assertions) && debugDump)
  {
    use std::{time::{SystemTime, UNIX_EPOCH}, fs};

    // Use the timestamp as an identifier for said folder
    let time = SystemTime::now().duration_since(UNIX_EPOCH).into_trace(Error::Unknown)?
                  .as_millis()
                  .to_string();

    let dumpDir = path!("/var/lib/kickit", time);

    // Recursively create our dump directory
    fs::create_dir_all(&dumpDir).into_trace(Error::RunFsFail)?;

    // Create a symlink so it acts as if it is a directory
    symlink(dumpDir, PathBuf::from("/run/kickit")).into_trace(Error::RunFsFail)?;
  }

  fs::create_dir("/run/kickit/service").into_trace(Error::RunFsFail)?;
  fs::create_dir("/run/kickit/private").into_trace(Error::RunFsFail)?;

  // Make the private folder, well, private
  fs::set_permissions("/run/kickit/private", Permissions::from_mode(0o600)).into_trace(Error::RunFsFail)?;

  for upService in (services)
  {
    fs::create_dir(path!("/run/kickit/service", upService)).into_trace(Error::RunFsFail)?;
  }

  Ok(())
}

#[inline]
fn mountSysFilesystems() -> Result<()>
{
  use kickit::init::mount::{
          Flag::{NoSuid, NoDev, NoExec, Remount, Private},
          mount, unmount, mounted, mountflags as flags};

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
  mount!("tmpfs", "/tmp", "tmpfs", flags![NoSuid, NoDev/*, NoExec*/]);
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

macro_rules! socks
{
  ($($sock: path),*) =>
  {
    {
      $(
        task::spawn(async move
        {
          $sock.open_sock().await.handle();
        });
      )*
    }
  };
}

// Use the dhat allocator for heap analysis, slower than regular allocator
#[cfg(feature = "dhat_heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

#[tokio::main]
async fn main()
{
  use std::{thread::park, env::args};
  use kickit::{init::{target, cmdlineParam, socket::Open, mount::mountFstabEntries}, socket};

  #[cfg(feature = "dhat_heap")]
  let profiler = dhat::Profiler::new_heap();

  let sysArgs: Vec<String> = args().collect();
  // Run alongside another init e.g. openrc or runit
  let noInit = sysArgs.len() > 1 && &sysArgs[1] == "--no-init";

  sanity(noInit).handle();

  // Respect the quiet argument if provided
  oncelock! { let QUIET = cmdlineParam("quiet") == Ok(None) }.handle();

  status!("kickit {}", kickit::PRETTY_VERSION());

  let targetName = if (cfg!(debug_assertions))
  {
    // Assume 'test' target on debug builds
    "test"
  }
  else {
    // If 'init.target=X' exists in cmdline, use X as our target, if not use system (the default)
    if let Ok(Some(c)) = (cmdlineParam("init.target"))
    {
      &c.clone() as &str
    }
    else {
      "system"
    }
  };

  status!("Target: {targetName}");

  /*
   * By having our target in a OnceLock we ensure that others parts of the init
   * can access it hassle-free for e.g. logging or services
   */
  oncelock! { let TARGET = target::source(targetName.to_owned()).handle() }.handle();
  oncelock! { let TARGET_NAME = targetName.to_owned() }.handle();

  // We've just set it so this shouldn't fail
  let target = TARGET.get().ok_or(Error::Unknown.trace("target is inaccessible")).handle();

  status!("Initialising services");
  let services = initServices(&target.services).handle();

  // If we are running alongside another init these things should've already been done
  if (!noInit)
  {
    status!("Mounting system filesystems");
    mountSysFilesystems().handle();

    status!("Setting hostname");
    let mut hostname = File::create("/proc/sys/kernel/hostname").into_trace(Error::Unknown).handle();
    hostname.write_all(target.hostname.as_bytes()).into_trace(Error::Unknown).handle();
  }

  // Now we can mount all the custom entries in /etc/fstab
  mountFstabEntries().handle();

  status!("Setting up work directory");
  setupRunFs(&target.services, target.debugDump).handle();

  // Open our sockets
  socks!(socket::Core, socket::Log, socket::Power);

  // Startup our services & wait for it to finish
  for service in (services)
  {
    service.start().handle();
  }

  // main() never exits so we never get a dhat analysis without explicitly dropping the profiler
  #[cfg(feature = "dhat_heap")]
  {
    use std::{thread::sleep, time::Duration};

    // Give some time for everything to be setup
    sleep(Duration::from_mins(3));

    // Drop the profiler so we get the analysis
    drop(profiler);
  }

  // Block indefinitely
  park();
}
