// kickit init process

#![allow(unused_parens)]
#![allow(non_snake_case)]

use std::{fs, fs::File, io::Write, path::PathBuf};
use tokio::task;
use nix::unistd::getuid;
use kickit::{oncelock, init::{console::prelude::*, init_guard, service::RootService, target::TARGET}};

/*
 * The way dhat does its profiling in heap mode, is that the profiler will wait until the main() function
 * has finished. However, in our case, since we are an init system, main() blocks indefinitely. So we have
 * to set a specific amount of time to wait for the init to setup everything and then manually drop the
 * profiler.
 */
#[cfg(feature = "dhat_heap")]
const DHAT_HEAP_WAIT_SECONDS: u64 = 5 * 60;

/**
  * # Errors
  *
  * * /run/kickit exists (kickit is already running)
  *
  * * Not ran as root user
  *
  * * Not ran as the init process (pid 1) and `--no-init` wasn't provided
 **/
#[inline]
fn sanity(permit_non_init: bool) -> Result<()>
{
  // If /run/kickit is already there then another kickit is running
  guard!(fs::metadata("/run/kickit").is_ok() => Error::AlreadyRunning.into());

  // Make sure we are running as the root user
  guard!(!getuid().is_root() => Error::NotRoot.into());

  guard!(!cfg!(feature = "bypass_init_check") && !permit_non_init && init_guard() => Error::NotInit.into());

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

  for service_name in (services)
  {
    create_dir(path!("/run/kickit/service", service_name)).into_trace(Error::RunFsFail)?;
  }

  Ok(())
}

/*
 * Mount system pseudo filesystems (proc, sys & dev) as well as temporary filesystems. This method also
 * additionally remounts the root filesystem as read-write
 */
#[inline]
fn mountSystemFilesystems() -> Result<()>
{
  use kickit::init::mount::mount_table;

  let sys_fs_tab = mount_table!
  [
    ("proc",  "/proc", "proc",     [NoSuid, NoDev, NoExec], []),
    ("sysfs", "/sys",  "sysfs",    [NoSuid, NoDev, NoExec], []),
    ("dev",   "/dev",  "devtmpfs", [NoSuid],                []),
    ("tmpfs", "/run",  "tmpfs",    [NoSuid],                []),
    // `NoExec` is ideal here, however, some programs will save install scripts to /tmp and such
    ("tmpfs", "/tmp",  "tmpfs",    [NoSuid, NoDev],         []),
    // Remount the rootfs as read-write in case of `ro` kernel cmdline param
    (_,       "/",     _,          [Private, Remount],      [])
  ];

  for entry in (sys_fs_tab)
  {
    entry.mount()?;
  }

  Ok(())
}

// Use the dhat allocator for heap analysis, slower than regular allocator
#[global_allocator]
#[cfg(feature = "dhat_heap")]
static ALLOC: dhat::Alloc = dhat::Alloc;

#[tokio::main]
async fn main() -> !
{
  use std::{thread::park, env::args};
  use kickit::{init::{target, cmdline_flag, socket::Open, service::STANDARD_SERVICES, mount::fstab}, socket};

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
  // This argument tells us to act like we're not the init process, only use kickit for service management
  let noInitArg = sysArgs.len() > 1 && &sysArgs[1] == "--no-init";

  sanity(noInitArg).handle();
  status!("kickit {}", kickit::VERSION);

  // Respect the quiet argument if provided
  oncelock! { QUIET = cmdline_flag("quiet") && !init_guard() }.handle();

  // Assume the test target on debug builds
  #[cfg(debug_assertions)]
  let target = sysArgs.last().unwrap();

  // Check if cmdline parameter 'init.target=XXX' has been provided, if not use the default "system" target
  #[cfg(not(debug_assertions))]
  let target = kickit::init::cmdline_value("init.target").unwrap_or(String::from("system"));

  status!("Target: {target}");

  /*
   * By having our target in a OnceLock we ensure that others parts of the init can access it hassle-free
   * for e.g. logging or services
   */
  oncelock! { TARGET = target::source(target.to_owned()).handle() }.handle();

  // We've just set it so this shouldn't fail
  let target = TARGET.get().ok_or(Error::Unknown.trace("target is inaccessible")).handle();

  status!("Initialising services");
  // Parse service's configuration - done before execution to weed out any config errors first
  let services: Vec<_> = target.services.iter().map(|s| RootService::try_source(s).handle()).collect();

  // If we are running alongside another init these things should've already been done
  if (!init_guard())
  {
    status!("Mounting system filesystems");
    mountSystemFilesystems().handle();

    status!("Setting hostname");
    let mut hostname = File::create("/proc/sys/kernel/hostname").into_trace(Error::ProcFs).handle();
    hostname.write_all(target.hostname.as_bytes()).into_trace(Error::ProcFs).handle();

    // Now we can mount all the custom entries in /etc/fstab
    for entry in (fstab().handle())
    {
      let (src, dest) = (entry.source().cloned().unwrap_or(PathBuf::from("none")), entry.dest());

      status!("Mounting {} -> {}", src.display(), dest.display());
      entry.mount().handle();
    }
  }

  status!("Setting up work directory");
  setupRunFs(target.services.as_slice(), target.debug_dump).handle();

  // Open socket listeners, uses a macro for different types
  open_socks!(socket::Core, socket::Log, socket::Power);

  let mut out_services = Vec::new();

  for service in (services)
  {
    status!("Starting service: {}", &service.cfg.name);
    // Spawn the service's process, start its supervisor & log watcher
    if let Some(service_handle) = service.up().await.handle()
    {
      out_services.push(service_handle);
    }
  }

  oncelock! { STANDARD_SERVICES = out_services }.handle();

  // main() never exits so we never get a dhat analysis without explicitly dropping the profiler
  #[cfg(feature = "dhat_heap")]
  {
    // Give some time for everything to be setup
    std::thread::sleep(std::time::Duration::from_secs(DHAT_HEAP_WAIT_SECONDS));
    // Drop the profiler so we get the analysis
    drop(profiler);
  }

  // Block indefinitely
  loop { park(); }
}
