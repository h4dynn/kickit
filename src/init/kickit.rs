// kickit init process

#![allow(unused_parens)]
#![allow(non_snake_case)]

use std::{fs, fs::File, io::Write, path::PathBuf, process};
use nix::unistd::getuid as uid;
use kickit::{console::Colour, console::{ReturnError, HandleKTError},
             init::{init_console::{KTError, KTErrorTrace, ConvKTError, status, stall},
                    service::Service, UP_SERVICES}, letOnceLock};

///
/// # Errors:
/// * if /run/kickit exists (kickit is already running),
/// * if not ran as root user,
/// * if not ran as the init process (pid 1)
///
#[inline] fn sanity(sideInit: bool) -> Result<(), KTError>
{
  use kickit::{console::affirm, init::init_console::warn};
  // If /run/kickit is already there then another kickit is running
  affirm!(fs::metadata("/run/kickit").is_err(), KTError::AlreadyRunning);

  affirm!(uid().is_root(), KTError::NotRoot);

  if (!sideInit)
  {
    if (cfg!(feature = "bypass_init_check"))
    {
      warn!("Bypassing init checks!");
    }
    else {
      affirm!(process::id() == 1, KTError::NotInit);
    }
  }

  Ok(())
}

/*
 * /run/kickit
 * |
 * ├── target.toml -> A symlink to the actual target (usually stored in /usr/lib/kickit/target/)
 * |
 * ├── service     -> All services which have been initialised by kickit
 * |   |
 * |   └── [NAME]
 * |       ├── pid    -> The service's process ID (Standard services only)
 * |       ├── exited -> The service's exit code (RunOnce services only)
 * |       └── log    -> The service's log (only accessible to root)
 * |
 * └── io.Core     -> The socket that ktctl uses to gather info like state, version & target
 */
#[inline] fn setupRunFs(services: &Vec<String>, debugDump: bool) -> Result<(), KTErrorTrace>
{
  use std::{fs::Permissions, os::unix::fs::{symlink, PermissionsExt}};
  use kickit::{path, hex_data};

  // An empty zstd file, in hex bytes, created from /dev/null (`$ zstd -1c < /dev/null | xxd`)
  let EMPTY_ZSTD = hex_data("28b52ffd240001000099e9d851").unwrap();

  if !(cfg!(debug_assertions) || debugDump)
  {
    // Normal behaviour - create runfs as a folder
    fs::create_dir("/run/kickit").trace(KTError::RunFsFail)?;
    fs::create_dir("/run/kickit/service").trace(KTError::RunFsFail)?;
  }
  // A debug dump will create a folder in /var/log/kickit and symlink it to the runfs
  else if (debugDump)
  {
    use std::{time::{SystemTime, UNIX_EPOCH}, fs};

    // Use the timestamp as an identifier for said folder
    let time = SystemTime::now().duration_since(UNIX_EPOCH).trace(KTError::Unknown)?.as_millis()
                  .to_string();

    let dumpDir = path!("/var/log/kickit", time);

    // Recursively create our dump directory
    fs::create_dir_all(&dumpDir).trace(KTError::RunFsFail)?;

    // Create a symlink so it acts as if it is a directory
    //symlink(dumpDir, PathBuf::from("/run/kickit")).trace(KTError::RunFsFail)?;
    symlink(dumpDir, PathBuf::from("/run/kickit")).trace(KTError::RunFsFail)?;

    fs::create_dir(PathBuf::from("/run/kickit/service")).trace(KTError::RunFsFail)?;
  }

  for upService in (services)
  {
    let mainDir = path!("/run/kickit/service", upService);
    let log = path!("/run/kickit/service", upService, "log");

    fs::create_dir(&mainDir).trace(KTError::RunFsFail)?;

    let mut logFile = File::create(&log).trace(KTError::RunFsFail)?;

    // Set permissions so that only root can access the logfile
    logFile.set_permissions(Permissions::from_mode(0o100_600)).trace(KTError::RunFsFail)?;
    // Setup empty ZSTD file (just the header)
    logFile.write_all(EMPTY_ZSTD.as_slice()).trace(KTError::RunFsFail)?;
  }

  Ok(())
}

#[inline] fn mountSysFilesystems() -> Result<(), KTErrorTrace>
{
  use kickit::init::mount::{MountFlag::{NoSuid, NoDev, NoExec},
                            mount, unmount, mounted, mountflags, mountopts};

  macro_rules! mounter
  {
    ($from: tt, $to: tt, $fsType: tt, $flags: expr, $opts: expr) =>
    {
      // Check if each destination is already mounted, and if so unmount it
      if !(mounted($to)? && unmount($to).is_err()) { mount($from, $to, $fsType, $flags, &$opts)? }
    }
  }

  mounter!("proc", "/proc", "proc", mountflags!(NoSuid, NoDev, NoExec), mountopts!());
  mounter!("sysfs", "/sys", "sysfs", mountflags!(NoSuid, NoDev, NoExec), mountopts!());
  mounter!("dev", "/dev", "devtmpfs", mountflags!(NoSuid), mountopts!());
  mounter!("tmpfs", "/run", "tmpfs", mountflags!(NoSuid), mountopts!());

  // No errors yippie
  Ok(())
}

// Ran before starting to catch any potential early errors in a service config
#[inline] fn initServices(services: &Vec<String>) -> Result<Vec<Service>, KTErrorTrace>
{
  let mut initServices: Vec<Service> = Vec::new();

  for service in (services)
  {
    initServices.push(Service::init(service)?);
  }

  Ok(initServices)
}

#[inline] fn startServices(services: Vec<Service>) -> Result<(), KTErrorTrace>
{
  use kickit::{init::service::Pattern::Standard, state};

  for mut upService in (services)
  {
    status!("Starting service: {}", upService.name);

    // Don't continue starting services if we are stalled
    if (!state!().is_ok()) { stall!(); }

    if let Err(trace) = upService.up()
    {
      // If this service is optional we only need to warn not abort
      if (upService.optional) { trace.warn(); } else { return Err(trace) }
    }
    else if (upService.pattern == Standard)
    {
      // Spawn the service watcher on another thread
      tokio::task::spawn(async move { upService.watch().handle() });
    }
  }

  Ok(())
}

fn watchPowerLevel() -> !
{
  use kickit::init::{PowerLevel, POWER_LEVEL};
  use nix::sys::reboot::{reboot, RebootMode};
  use std::{thread, time::Duration};

  let mode = loop
  {
    // Repeat this loop every half a second to prevent using too many CPU cycles
    thread::sleep(Duration::from_millis(500));

    match (POWER_LEVEL.get())
    {
      Some(&PowerLevel::Off) => break RebootMode::RB_HALT_SYSTEM,
      Some(&PowerLevel::Reboot) => break RebootMode::RB_AUTOBOOT,
      // No power signal was found, move on
      None => ()
    }
  };

  /* for service in (UP_SERVICES.get().unwrap())
  {
    // Wait for each of the services to stop
    while (Service::is_up(service)) {}
  } */

  /*
   * This will never ever return unless an error occurrs so we will
   * unwrap this & handle it
   */
  reboot(mode).trace(KTError::Shutdown).unwrap_err().fatal();
}

macro_rules! socks
{
  ($($sock: path),*) => { $(tokio::task::spawn(async move { $sock.start().await.handle(); });)* };
}

#[tokio::main] async fn main()
{
  use kickit::{init::{target, target::TARGET_NAME, cmdlineParam}, socket, socket::Start};
  use std::{thread, time::Duration, env};

  let sysArgs: Vec<String> = env::args().collect();
  // Run alongside another init e.g. openrc or runit
  let noInit = sysArgs.len() > 1 && &sysArgs[1] as &str == "--no-init";

  sanity(noInit).handle();

  status!("kickit {}", kickit::version());

  let targetName = if (cfg!(debug_assertions))
  {
    // Assume 'test' target on debug builds
    "test"
  }
  else {
    // If 'init.target=X' exists in cmdline, use X as our target, if not use system (the default)
    if let Ok(Some(c)) = (cmdlineParam("init.target")) { &c.clone() as &str } else { "system" }
  };

  status!("Target: {targetName}");
  let target = target::source(String::from(targetName)).handle();

  letOnceLock! { let TARGET_NAME = String::from(targetName) }.handle();

  status!("Initialising services");
  let ktServices = initServices(&target.services).handle();

  status!("Setting up work directory");
  setupRunFs(&target.services, target.debugDump).handle();

  // Open our sockets
  socks!(socket::Core, socket::Log, socket::Power);

  // If we are running alongside another init these things should've already been done
  if (!noInit)
  {
    status!("Mounting system filesystems");
    mountSysFilesystems().handle();

    status!("Setting hostname");

    let mut hostname = File::create("/proc/sys/kernel/hostname").trace(KTError::Unknown).handle();
    hostname.write_all(target.hostname.as_bytes()).trace(KTError::Unknown).handle();
  }

  // Startup our services & wait for it to finish
  startServices(ktServices).handle();

  letOnceLock! { let UP_SERVICES = target.services }.handle();

  tokio::task::spawn(async move { watchPowerLevel() });

  thread::sleep(Duration::new(u64::MAX, 0));
}
