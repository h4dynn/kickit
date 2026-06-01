//! Warden - optional sandbox for services

#![allow(unused_parens)]
#![allow(non_snake_case)]

extern crate nix;

use kickit::{wrap, oncelock, console::{Colour, ReturnError, HandleError}};
use nix::sched::CloneFlags as NixFlag;
use std::{fmt::Display, path::{Path, PathBuf}, io, process};

// A standard type, for anything that can be displayed
pub struct Error<Inner: Display>(Inner);

/*
 * This is a sandboxed Command (wraps over `std::process::Command`).
 * The sandbox includes namespace seperation & a container rootfs (which
 * we will chroot into)
 */
#[derive(Debug)]
pub struct Command<'inner>
{
  inner: &'inner mut process::Command,
  // Where we will chroot into
  root: PathBuf,
  // All the files/directories that we will share via bind mounting to the container
  bind: BindMounts,
  // Namespace sandboxing flags (backend is libc's `unshare`)
  flags: NsFlags
}

#[derive(PartialEq, Eq, Clone, Default, Debug)]
pub struct BindMounts
{
  files: Vec<PathBuf>,
  dirs: Vec<PathBuf>
}

// Pretty wrapper over `nix::sched::CloneFlags`
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum NsFlag
{
  // Share memory space between caller and child
  ShareVm = NixFlag::CLONE_VM.bits() as isize,
  // Share System-V semaphore adjustment values between caller and child
  ShareVSem = NixFlag::CLONE_SYSVSEM.bits() as isize,
  // Share filesystem info between caller and child
  ShareFs = NixFlag::CLONE_FS.bits() as isize,
  // Share file-descriptor table between caller and child
  ShareFiles = NixFlag::CLONE_FILES.bits() as isize,
  // Share the signal handler between caller and child
  ShareSignalHandler = NixFlag::CLONE_SIGHAND.bits() as isize,
  // Tracing process cannot call ptrace on child process
  ShareUntraced = NixFlag::CLONE_UNTRACED.bits() as isize,
  // If caller is being traced, then child will be as well
  SharePTrace = NixFlag::CLONE_PTRACE.bits() as isize,
  // Execution of the caller is suspended until child releases its virtual memory via `execve` or `_exit`
  ShareVFork = NixFlag::CLONE_VFORK.bits() as isize,
  // The parent of the new child is the same as the caller's
  ShareParent = NixFlag::CLONE_PARENT.bits() as isize,
  // Share thread group between caller and child
  ShareThread = NixFlag::CLONE_THREAD.bits() as isize,
  // Share input/output context between caller and child
  ShareIo = NixFlag::CLONE_IO.bits() as isize,
  // The child will be started in a new mount namespace, seperate mount types from root & container
  NewMount = NixFlag::CLONE_NEWNS.bits() as isize,
  // The child will be created in a new cgroup namespace
  NewCGroup = NixFlag::CLONE_NEWCGROUP.bits() as isize,
  // Create child in a new UTS namespace
  NewUts = NixFlag::CLONE_NEWUTS.bits() as isize,
  // Create child in a new IPC namespace
  NewIpc = NixFlag::CLONE_NEWIPC.bits() as isize,
  // Create child in a new user namespace
  NewUser = NixFlag::CLONE_NEWUSER.bits() as isize,
  // Create child in a new PID namespace
  NewPid = NixFlag::CLONE_NEWPID.bits() as isize,
  // Create child in a new network namespace
  NewNetwork = NixFlag::CLONE_NEWNET.bits() as isize
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct NsFlags(i32);

wrap! {
  // C-style flags, wrapper to `nix::mount::MsFlags`
  impl Deref<Target = i32> for NsFlags;
  // Unwrap the error so we can display it when we call `fatal!()`
  impl<Inner: Display> Deref<Target = Inner> for Error;
}

// Global options, set only if the user provides the argument
oncelock! {
  // Mount the system pseudo filesystems, required for some commands to run properly
  static MOUNT_SYSTEM_FS: bool;
  // Link the dbus run socket to the container
  static LINK_DBUS: bool;
}

trait StandardizeError<OkType, ErrorType: Display>
{
  fn errorize(self) -> Result<OkType, Error<ErrorType>>;
}

macro_rules! fatal
{
  ($($frag: tt)*) =>
  {
    {
      use Colour::{RED, BOLD, RESET};
      eprintln!("warden {}(error): {}{}{}", RED, BOLD, format!($($frag)*), RESET);
      process::exit(1);
    }
  };
}

macro_rules! warn
{
  ($($frag: tt)*) =>
  {
    {
      use Colour::{ORANGE, BOLD, RESET};
      eprintln!("warden {}(error): {}{}{}", ORANGE, BOLD, format!($($frag)*), RESET);
    }
  };
}

impl<Inner: Display> ReturnError for Error<Inner>
{
  fn fatal(self) -> !
  {
    fatal!("{}", *self)
  }
  fn warn(self)
  {
    warn!("{}", *self);
  }
}

impl<E: Display> From<E> for Error<E>
{
  fn from(input: E) -> Self
  {
    Error::<E>(input)
  }
}

impl<OkType, ErrorType: Display> StandardizeError<OkType, ErrorType> for Result<OkType, ErrorType>
{
  fn errorize(self) -> Result<OkType, Error<ErrorType>>
  {
    match (self)
    {
      Ok(ok) => Ok(ok),
      Err(err) => Err(Error::<ErrorType>::from(err))
    }
  }
}

fn usage()
{
  eprintln!("Usage: {}warden{} <FLAGS> <ROOT> <PROGRAM> [-- ARGUMENTs..]", Colour::BOLD, Colour::RESET);
  eprintln!("A service sandboxer, unshares namespaces and chroots");
  eprintln!();
  eprintln!("Flags:");
  eprintln!(" -h, --help               Show this help prompt & exit");
  eprintln!(" -l, --list               List all share & new flags & exit");
  eprintln!(" -b, --bind-file PATH     Bind mount a file to the container");
  eprintln!(" -B, --bind-dir PATH      Bind mount a directory & its file to the container");
  eprintln!(" -d, --dbus               Link the dbus socket to the container");
  eprintln!(" -S, --mount-system-fs    Mount system pseudo filesystems");
  eprintln!(" -s, --share FLAG         Share the current namespace with child");
  eprintln!(" -n, --new FLAG           Create a new seperate namespace for child");
  eprintln!();
  process::exit(0);
}

fn listFlags()
{
  use kickit::DelimVecIter;

  macro_rules! eprintln_each
  {
    ($iter: expr) =>
    {
      {
        for x in ($iter)
        {
          eprintln!(" * {x}");
        }
      }
    };
  }

  eprintln!("{}Flags (share):{}", Colour::BOLD, Colour::RESET);
  eprintln_each!(DelimVecIter::<&str>::new(NsFlag::SHARE_FLAGS.to_vec(), ','));
  eprintln!();

  eprintln!("{}Flags (new):{}", Colour::BOLD, Colour::RESET);
  eprintln_each!(DelimVecIter::<&str>::new(NsFlag::NEW_FLAGS.to_vec(), ','));

  process::exit(0);
}

impl<'inner> Command<'inner>
{
  pub fn new(inner: &'inner mut process::Command, root: impl AsRef<Path>, bind: BindMounts, flags: NsFlags) -> Self
  {
    Self { inner, root: PathBuf::from(root.as_ref()), bind, flags }
  }

  /**
    * # Errors
    *
    * * Failed to unshare the current process's namespaces (see `nix::sched::unshare`),
    * * Failed to create the dynamic linker's file which will be bind mounted to,
    * * Failed to mount the dynamic linker, provided files or a system filesystem,
    * * Failed to create the parent directory for a binded file or just a binded directory,
    * * Failed to chroot into the container (see `nix::unistd::chroot`),
    * * Failed to spawn the command using the standard library (see `std::process::Command::spawn`)
    */
  pub fn spawn(self) -> io::Result<process::Child>
  {
    use kickit::path;
    use std::fs::{create_dir, create_dir_all, File};
    use nix::{sched::unshare, unistd::chroot, mount::{MsFlags as Flag, mount}};

    // The dynamic linker/interpreter, architecture dependent
    const DYNAMIC_LD: &str =
    {
      cfg_select!
      {
        target_arch = "x86_64" => "usr/lib/ld-linux-x86-64.so.2",
        target_arch = "aarch64" => "usr/lib/ld-linux-aarch64.so.1",
        _ => compile_error!("Architecture does not have a known dynamic linker, please implement it here")
      }
    };

    // Unshare first to apply the correct profile to the spawned process
    // TO-DO: Flag `share_user` needs more implementation to work properly (uid/gid map)
    unshare(self.flags.into())?;

    let root = self.root;
    let bindFlags = Flag::MS_BIND | Flag::MS_PRIVATE | Flag::MS_RDONLY | Flag::MS_SILENT | Flag::MS_REC;
    // rust wants to know dat type
    let noData: Option<&PathBuf> = None.as_ref();

    // Dynamic linker will be required for vast majority of executables
    let _ = File::create_new(path!(&root, DYNAMIC_LD))?;

    mount(Some(&path!("/", DYNAMIC_LD)), &path!(&root, DYNAMIC_LD), noData, bindFlags, noData)?;

    for bindFile in (self.bind.files)
    {
      if let Some(parent) = bindFile.parent()
      {
        // The parent directory where the file will be stored
        create_dir_all(format!("{}/{}", root.display(), parent.display()))?;
      }

      // Create the binding file
      let _ = File::create_new(format!("{}/{}", root.display(), bindFile.display()))?;

      mount(Some(&bindFile), format!("{}/{}", root.display(), bindFile.display()).as_str(), noData, bindFlags, noData)?;
    }

    for bindDir in (self.bind.dirs)
    {
      // Create the binding directory
      create_dir_all(format!("{}/{}", root.display(), bindDir.display()).as_str())?;
      mount(Some(&bindDir), format!("{}/{}", root.display(), bindDir.display()).as_str(), noData, bindFlags, noData)?;
    }

    // Change root directory to the prepared container
    chroot(&root)?;

    if (*oncelock!(&MOUNT_SYSTEM_FS.unwrap_or(false)))
    {
      // These are universal pseudo flags that are applied to sys, dev and proc
      let sys = Flag::MS_PRIVATE | Flag::MS_BIND | Flag::MS_NOSUID | Flag::MS_NOEXEC | Flag::MS_SILENT | Flag::MS_REC;
      let tmp = Flag::MS_RELATIME;

      // Mount system pseudo-filesystems
      mount(Some("sysfs"), "/sys", Some("sysfs"), sys | Flag::MS_NODEV, noData)?;
      mount(Some("proc"), "/proc", Some("proc"), sys | Flag::MS_NODEV, noData)?;
      mount(Some("devtmpfs"), "/dev", Some("devtmpfs"), sys, noData)?;
      mount(Some("tmpfs"), "/tmp", Some("tmpfs"), tmp, noData)?;
      mount(Some("tmpfs"), "/run", Some("tmpfs"), tmp, noData)?;

      if (*oncelock!(&LINK_DBUS.unwrap_or(false)) && PathBuf::from("/run/dbus").is_dir())
      {
        create_dir(format!("{}/run/dbus", root.display()))?;
        // Bind the dbus socket-containing directory
        mount(Some("/run/dbus"), format!("{}/run/dbus", root.display()).as_str(), noData, Flag::MS_BIND, noData)?;
      }
    }

    // Now that we are fully sandboxed we spawn the process
    self.inner.current_dir("/").spawn()
  }

  /**
    * # Errors
    *
    * * Failed to unmount the socket-containing /run/dbus directory (see `nix::mount::umount`),
    * * Failed to lazily unmount a directory after trying non-lazily (see `nix::mount::umount2`)
    */
  // Unmount pseudo system-filesystems
  pub fn cleanup() -> io::Result<()>
  {
    use nix::mount::{MntFlags as UnmountFlag, umount, umount2};

    if (*oncelock!(&LINK_DBUS.unwrap_or(false)))
    {
      umount("/run/dbus")?;
    }

    for dest in ["/run", "/tmp", "/sys", "/proc", "/dev"]
    {
      if let Err(err) = umount(dest)
      {
        warn!("Failed to unmount {}: {err}", &dest);
        umount2(dest, UnmountFlag::MNT_FORCE)?;
      }
    }
    Ok(())
  }
}

impl From<NsFlag> for NixFlag
{
  fn from(flag: NsFlag) -> Self
  {
    Self::from_bits_truncate(flag as i32)
  }
}

impl From<NsFlags> for NixFlag
{
  fn from(flags: NsFlags) -> Self
  {
    NixFlag::from_bits_truncate(*flags)
  }
}

impl NsFlag
{
  pub const SHARE_FLAGS: [&str; 11] = ["vm", "vsem", "fs", "files", "sighandler", "untraced", "ptrace",
                                        "vfork", "parent", "thread", "io"];

  pub const NEW_FLAGS: [&str; 7] = ["mount", "cgroup", "uts", "ipc", "user", "pid", "net"];

  pub fn new(input: impl Display) -> Option<Self>
  {
    Some(match (input.to_string().to_ascii_lowercase().as_str())
    {
      "share_vm" => Self::ShareVm, "share_vsem" => Self::ShareVSem,
      "share_fs" => Self::ShareFs, "share_files" => Self::ShareFiles,
      "share_sighandler" => Self::ShareSignalHandler, "share_untraced" => Self::ShareUntraced,
      "share_ptrace" => Self::SharePTrace, "share_vfork" => Self::ShareVFork,
      "share_parent" => Self::ShareParent, "share_thread" => Self::ShareThread,
      "share_io" => Self::ShareIo, "new_mount" => Self::NewMount,
      "new_cgroup" => Self::NewCGroup, "new_uts" => Self::NewUts, "new_ipc" => Self::NewIpc,
      "new_user" => Self::NewUser, "new_pid" => Self::NewPid, "new_net" => Self::NewNetwork,
      _ => None?
    })
  }
}

impl NsFlags
{
  pub fn push(&mut self, add: NsFlag)
  {
    self.0 += add as i32;
  }
}

fn main()
{
  use std::env::args;

  let mut argIter = args();

  // The executable name called (e.g. `warden`)
  let _warden = argIter.next().unwrap_or(String::from("/usr/lib/kickit/warden"));

  macro_rules! next
  {
    ($iter: expr) =>
    {
      argIter.next().ok_or(Error("Expected an option for this argument!")).handle()
    };
  }

  // Set a oncelock
  macro_rules! set
  {
    { $lock: ident = $val: expr } =>
    {
      {
        let _ = $lock.set($val).map_err(|_| { fatal!("Failed to set an argument's value!") });
      }
    };
  }

  let mut root: Option<String> = None;
  let mut flags = NsFlags::default();
  let mut bindFiles = Vec::<PathBuf>::new();
  let mut bindDirs = Vec::<PathBuf>::new();

  // Loop through all the arguments in our iterator
  while let Some(arg) = argIter.next()
  {
    match (arg.as_str())
    {
      "-h" | "--help" | "help" => usage(),
      "-l" | "--list" => listFlags(),
      "-b" | "--bind-file" => bindFiles.push(PathBuf::from(next!(argIter))),
      "-B" | "--bind-dir" => bindDirs.push(PathBuf::from(next!(argIter))),
      "-S" | "--mount-system-fs" => set! { MOUNT_SYSTEM_FS = true },
      "-d" | "--dbus" => set! { LINK_DBUS = true },
      "-s" | "--share" =>
      {
        let next = &next!(argIter);
        let flag = NsFlag::new(format!("share_{next}")).ok_or(Error(format!("Unknown share flag provided: {next}"))).handle();
        flags.push(flag);
      },
      "-n" | "--new" =>
      {
        let next = &next!(argIter);
        let flag = NsFlag::new(format!("new_{next}")).ok_or(Error(format!("Unknown new flag provided: {next}"))).handle();
        flags.push(flag);
      },
      newRoot => { root = Some(newRoot.to_owned()); break; }
    }
  }

  // If no arguments have been provided we will end up here
  match (root)
  {
    Some(root) =>
    {
      let Some(exec) = argIter.next() else { fatal!("Provide an executable!") };

      // Its okay to not provide any arguments with collect
      let args: Vec<String> = argIter.collect();
      let bind = BindMounts { files: bindFiles, dirs: bindDirs };

      // Execute the command in the sandbox
      let mut child = Command::new(process::Command::new(exec).args(args), &root, bind, flags).spawn().errorize().handle();

      // Wait until we have finished
      child.wait().unwrap();

      // If we have mounted system pseudo filesystems then we unmount all of them
      if (*oncelock!(&MOUNT_SYSTEM_FS.unwrap_or(false)))
      {
        Command::cleanup().errorize().handle();
      }
    },
    None => usage()
  }
}
