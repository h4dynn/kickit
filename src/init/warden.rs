//! Warden - optional sandbox for services

#![allow(unused_parens)]
#![allow(non_snake_case)]

extern crate nix;

use kickit::{wrap, oncelock, console::prelude::*, init::service::sandbox::{NsFlag, NsFlags}, BoxedStr};

use nix::{mount::{MsFlags as Flag, mount}};
use std::{fmt::Display, path::PathBuf, io, process::{Command, Child, exit}};

// A standard type, for anything that can be displayed
pub struct Error<Inner: Display>(Inner);

/*
 * This is a sandboxed command (wraps over `std::process::Command`).
 * The sandbox includes namespace seperation & a container rootfs (which
 * we will chroot into)
 */
#[derive(Debug)]
pub struct Sandbox<'inner>
{
  inner: &'inner mut Command,
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
  files: Vec<BoxedStr>,
  dirs: Vec<BoxedStr>
}

// This is for nix library mount call, where a referenced option path is expected
const NONE: Option<&PathBuf> = None;

wrap! {
  // Unwrap the error so we can display it when we call `fatal!()`
  impl<Inner: Display> Deref<Target = Inner> for Error;
}

// Global options, set only if the user provides the argument
oncelock! {
  // Mount the system pseudo filesystems, required for some commands to run properly
  static MOUNT_SYSTEM_FS: OnceLock<bool>;
  // Link the dbus run socket to the container
  static LINK_DBUS: OnceLock<bool>;
}

trait StandardizeError<OkType, ErrorType: Display>
{
  fn errorize(self) -> Result<OkType, Error<ErrorType>>;
}

macro_rules! fatal
{
  ($($frag: tt)*) =>
  {{
    use std::process::exit;

    eprintln!("warden {} {}", "(error):".colour(Colour::Red), format!($($frag)*).bold());
    exit(1);
  }};
}

macro_rules! warn
{
  ($($frag: tt)*) =>
  {{
    eprintln!("warden {} {}", "(warning):".colour(Colour::Orange), format!($($frag)*).bold());
  }};
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

trait SandboxCommand<'inner>
{
  fn sandbox(&'inner mut self, root: PathBuf, bind: BindMounts, flags: NsFlags) -> Sandbox<'inner>;
}

fn usage()
{
  eprintln!("Usage: {}warden{} <FLAGS> <ROOT> <PROGRAM> [ARGUMENTs..]", Colour::Bold, Colour::Reset);
  eprintln!("A service sandboxer, unshares namespaces and chroots");
  eprintln!();
  eprintln!("Flags:");
  eprintln!(" -h, --help               Show this help prompt & exit");
  eprintln!(" -l, --list               List all flags & exit");
  eprintln!(" -b, --bind-file PATH     Bind mount a file to the container");
  eprintln!(" -B, --bind-dir PATH      Bind mount a directory & its file to the container");
  eprintln!(" -d, --dbus               Link the dbus socket to the container");
  eprintln!(" -S, --mount-system-fs    Mount system pseudo filesystems");
  eprintln!(" -f, --flag FLAG          Namespace unsharing flags");
  eprintln!();
  exit(0);
}

fn list_flags()
{
  use kickit::delim_iter;

  macro_rules! eprint_each
  {
    ($iter: expr) =>
    {
      {
        for x in ($iter)
        {
          eprint!("{x}");
        }
        eprintln!();
      }
    };
  }

  eprintln!("{}", "Flags:".bold());

  eprint!("* ");
  eprint_each!(delim_iter(NsFlag::FLAGS.iter(), ",\n* "));

  exit(0);
}

impl<'inner> SandboxCommand<'inner> for Command
{
  fn sandbox(&'inner mut self, root: PathBuf, bind: BindMounts, flags: NsFlags) -> Sandbox<'inner>
  {
    Sandbox { inner: self, root, bind, flags }
  }
}

impl Sandbox<'_>
{
  // The dynamic linker/interpreter, architecture dependent
  const DYNAMIC_LD: &'static str = cfg_select! 
  {
    target_arch = "x86_64" => "usr/lib/ld-linux-x86-64.so.2",
    target_arch = "aarch64" => "usr/lib/ld-linux-aarch64.so.1",
    _ => compile_error!("Architecture does not have a known dynamic linker, please implement it here")
  };

  fn bind_files(root: impl Display, files: Vec<impl Display>) -> io::Result<()>
  {
    let flags = Flag::MS_BIND | Flag::MS_PRIVATE | Flag::MS_RDONLY | Flag::MS_SILENT | Flag::MS_REC;

    for file in (files)
    {
      let src = PathBuf::from(format!("/{file}"));
      let dest = PathBuf::from(format!("{root}/{file}"));
 
      mount(Some(&src), &dest, NONE, flags, NONE)?;
    }
    Ok(())
  }

  /**
    * # Errors
    *
    * * Failed to unshare the current process's namespaces (see `nix::sched::unshare`),
    * * Failed to create the dynamic linker's file which will be bind mounted to,
    * * Failed to mount the dynamic linker, provided files or a system filesystem,
    * * Failed to create the parent directory for a binded file or just a binded directory,
    * * Failed to chroot into the container (see `nix::unistd::chroot`),
    * * Failed to spawn the command using the standard library (see `std::Command::spawn`)
    */
  // Spawn new sandbox on THIS thread, meaning everything after this will also be sandboxed (no cloning is done)
  pub fn spawn_here(&mut self) -> io::Result<Child>
  {
    use kickit::path;
    use std::fs::{create_dir, create_dir_all, File};
    use nix::{sched::unshare, unistd::chroot};

    // Unshare first to apply the correct profile to the spawned process
    // TO-DO: Flag `NewUser` needs more implementation to work properly (uid/gid map)
    unshare(self.flags.into())?;

    let rootPath = &self.root;
    let root = rootPath.display();
    let mut binds = Vec::<BoxedStr>::new();

    // Dynamic linker will be required for vast majority of executables
    let _ = File::create_new(path!(&rootPath, Self::DYNAMIC_LD))?;
    binds.push(Self::DYNAMIC_LD.into());

    for bindFile in (self.bind.files.clone())
    {
      if let Some(parent) = PathBuf::from(&*bindFile).parent()
      {
        // The parent directory where the file will be stored
        create_dir_all(format!("{root}/{}", parent.display()))?;
      }

      // Create the binding file
      let _ = File::create_new(format!("{root}/{bindFile}"))?;

      binds.push(bindFile);
    }

    for bindDir in (self.bind.dirs.clone())
    {
      // Create the binding directory
      create_dir_all(format!("{root}/{bindDir}").as_str())?;
      binds.push(bindDir);
    }

    Self::bind_files(&root, binds)?;

    if (*oncelock!(&MOUNT_SYSTEM_FS.unwrap_or(false)))
    {
      // These are universal pseudo flags that are applied to sys, dev and proc
      let sys = Flag::MS_NOSUID | Flag::MS_NOEXEC;
      let tmp = Flag::MS_NOSUID | Flag::MS_NODEV;

      // Mount system pseudo-filesystems
      mount(Some("sysfs"), format!("{root}/sys").as_str(), Some("sysfs"), sys | Flag::MS_NODEV, NONE)?;
      mount(Some("proc"), format!("{root}/proc").as_str(), Some("proc"), sys | Flag::MS_NODEV, NONE)?;
      mount(Some("devtmpfs"), format!("{root}/dev").as_str(), Some("devtmpfs"), sys, NONE)?;
      mount(Some("tmpfs"), format!("{root}/tmp").as_str(), Some("tmpfs"), tmp, NONE)?;
      mount(Some("tmpfs"), format!("{root}/run").as_str(), Some("tmpfs"), tmp, NONE)?;

      if (*oncelock!(&LINK_DBUS.unwrap_or(false)) && PathBuf::from("/run/dbus").is_dir())
      {
        create_dir(format!("{root}/run/dbus"))?;
        // Bind the dbus socket-containing directory
        mount(Some("/run/dbus"), format!("{root}/run/dbus").as_str(), NONE, Flag::MS_BIND, NONE)?;
      }
    }

    // Change root directory to the prepared container
    chroot(rootPath)?;

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
  pub fn cleanup(self) -> io::Result<()>
  {
    use nix::mount::{MntFlags as UnmountFlag, umount, umount2};

    macro_rules! umount_or_force
    {
      ($mountpoint: expr) =>
      {
        if let Err(err) = umount($mountpoint)
        {
          warn!("Failed to unmount {}: {err}", $mountpoint.display());
          umount2($mountpoint, UnmountFlag::MNT_FORCE)?;
        }
      };
    }

    let mut unmount = Vec::new();

    if (*oncelock!(&LINK_DBUS.unwrap_or(false)))
    {
      unmount.push(PathBuf::from("/run/dbus"));
    }

    if (*oncelock!(&MOUNT_SYSTEM_FS.unwrap_or(false)))
    {
      unmount.extend(["/run", "/tmp", "/sys", "/proc", "/dev"].map(Into::into));
    }

    unmount.extend(self.bind.files.iter().map(|s| (&**s).into()));
    unmount.extend(self.bind.dirs.iter().map(|s| (&**s).into()));
    // Unmount the dynamic linker, this is always binded for every program
    unmount.push(Self::DYNAMIC_LD.into());

    for dest in unmount
    {
      umount_or_force!(&dest);
    }

    Ok(())
  }
}

fn main()
{
  use std::env::args;

  let mut argIter = args();
  argIter.next();

  macro_rules! next
  {
    ($iter: expr) =>
    {
      $iter.next().ok_or(Error("Expected an option for this argument!")).handle()
    };
  }

  // Set a oncelock
  macro_rules! set
  {
    { $lock: ident = $val: expr } =>
    {{
      $lock.set($val).map_err(|_| fatal!("Failed to set an argument's value!"));
    }};
  }
 
  let mut root = Option::<String>::None;
  let mut flags = NsFlags::default();
  let mut bindFiles = Vec::<BoxedStr>::new();
  let mut bindDirs = Vec::<BoxedStr>::new();

  // Loop through all the arguments in our iterator
  while let Some(arg) = argIter.next()
  {
    match (arg.as_str())
    {
      "-h" | "--help" | "help" => usage(),
      "-l" | "--list" => list_flags(),
      "-b" | "--bind-file" => bindFiles.push(next!(argIter).into()),
      "-B" | "--bind-dir" => bindDirs.push(next!(argIter).into()),
      "-S" | "--mount-system-fs" => set! { MOUNT_SYSTEM_FS = true },
      "-d" | "--dbus" => set! { LINK_DBUS = true },
      "-f" | "--flag" =>
      {
        let next = next!(argIter);
        flags.push(NsFlag::new(&next).ok_or(Error(format!("Invalid flag: {next}"))).handle());
      },
      new => { root = Some(new.to_owned()); break; }
    }
  }

  // If no arguments have been provided we will end up here
  match (root)
  {
    Some(new) =>
    {
      let exec = argIter.next().ok_or(Error("Provide an executable!")).handle();
      // Its okay to not provide any arguments with collect
      let args: Vec<String> = argIter.collect();
      let bind = BindMounts { files: bindFiles, dirs: bindDirs };

      let mut cmd = Command::new(exec);
      let mut sandbox = cmd.args(args).sandbox(new.into(), bind, flags);

      // Execute the command in the sandbox
      let mut child = sandbox.spawn_here().errorize().handle();

      // Wait until we have finished
      child.wait().unwrap();

      sandbox.cleanup().errorize().or_warn();
    },
    None => usage()
  }
}
