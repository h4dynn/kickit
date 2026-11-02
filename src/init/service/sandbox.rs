//! Kernel namespace sandboxing for services

use serde::Deserialize;
use crate::{BoxedStr, AnyStr, init::console::prelude::*};
use std::{collections::VecDeque, path::{Path, PathBuf}};
use nix::sched::CloneFlags;

/// Configuration options for the sandbox
#[derive(PartialEq, Eq, Clone, Deserialize, Debug)]
pub struct Sandbox
{
  // List of all the flags in string form, such as "ShareVm" or "NewUser"
  pub flags: Vec<BoxedStr>,
  // What binaries this container will be using
  pub import: Vec<BoxedStr>,
  // Files from the container we want to share to the outside
  pub export: Option<Vec<BoxedStr>>,
  // Bind mount the system pseudo filesystems, required for majority of services
  pub bindSystemFs: Option<bool>,
  // Bind mount the dbus socket, some services like lightdm or elogind will need this
  pub bindDbus: Option<bool>,
  // Share the provided sandboxed files with the rest of the OS via bind mount
  pub files: Option<BoxedStr>
}

c_style_flags! {
  ///
  /// Pretty wrapper over [`nix::sched::CloneFlags`]. Used in its array form ([`NsFlags`]) when
  /// specifying sandboxing options, outlining which namespaces are shared & cloned.
  ///
  /// This is not a unique implementation, it is identical to the [`nix`] implementation, which
  /// itself is simply Rust bindings to `libc`.
  ///
  /// For more information on these flags (`CLONE_FLAGS` in libc), see the [`unshare(2)`] man
  /// page for more information.
  ///
  /// [`unshare(2)`]: <https://www.man7.org/linux/man-pages/man2/unshare.2.html>
  ///
  #[derive(Copy, Clone, PartialEq, Eq, Debug)]
  pub enum NsFlag: i32
  {
    /// Share memory space between caller and child
    ShareVm = CloneFlags::CLONE_VM.bits(),
    /// Share System-V semaphore adjustment values between caller and child
    ShareVSem = CloneFlags::CLONE_SYSVSEM.bits(),
    /// Share filesystem info between caller and child
    ShareFs = CloneFlags::CLONE_FS.bits(),
    /// Share file-descriptor table between caller and child
    ShareFiles = CloneFlags::CLONE_FILES.bits(),
    /// Share the signal handler between caller and child
    ShareSignalHandler = CloneFlags::CLONE_SIGHAND.bits(),
    /// Tracing process cannot call ptrace on child process
    ShareUntraced = CloneFlags::CLONE_UNTRACED.bits(),
    /// If caller is being traced, then child will be as well
    SharePTrace = CloneFlags::CLONE_PTRACE.bits(),
    /// Execution of the caller is suspended until child releases its virtual memory via `execve` or `_exit`
    ShareVFork = CloneFlags::CLONE_VFORK.bits(),
    /// The parent of the new child is the same as the caller's
    ShareParent = CloneFlags::CLONE_PARENT.bits(),
    /// Share thread group between caller and child
    ShareThread = CloneFlags::CLONE_THREAD.bits(),
    /// Share input/output context between caller and child
    ShareIo = CloneFlags::CLONE_IO.bits(),
    /// The child will be started in a new mount namespace, seperate mount types from root & container
    NewMount = CloneFlags::CLONE_NEWNS.bits(),
    /// The child will be created in a new cgroup namespace
    NewCGroup = CloneFlags::CLONE_NEWCGROUP.bits(),
    /// Create child in a new UTS namespace
    NewUts = CloneFlags::CLONE_NEWUTS.bits(),
    /// Create child in a new IPC namespace
    NewIpc = CloneFlags::CLONE_NEWIPC.bits(),
    /// Create child in a new user namespace
    NewUser = CloneFlags::CLONE_NEWUSER.bits(),
    /// Create child in a new PID namespace
    NewPid = CloneFlags::CLONE_NEWPID.bits(),
    /// Create child in a new network namespace
    NewNetwork = CloneFlags::CLONE_NEWNET.bits()
  }

  #[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
  pub struct NsFlags([NsFlag] as i32);
}

impl Sandbox
{
  /// Initialize the container rootfs, with specified bind mounts
  /**
    * # Errors
    *
    * * Failed to create a required system directory in the container (e.g. etc, usr, sys, dev, proc)
    *
    * * Failed to change directory to the container for whatever reason
    *
    * * Failed to create a required symlink (e.g. bin -> usr/bin)
    */
  pub fn setup(&self, containerPath: impl AsRef<Path>) -> Result<()>
  {
    use std::{fs::create_dir, env::set_current_dir, os::unix::fs::symlink};

    macro_rules! mkdir
    {
      ($($dir: expr),*) =>
      {
        $(
          create_dir($dir).into_trace(Error::ServiceSandboxInit)?;
        )*
      }
    }

    let container: &Path = containerPath.as_ref();

    // Create the main usr directories
    mkdir!(container, path!(container, "etc"), path!(container, "usr"), path!(container, "usr", "bin"),
            path!(container, "usr", "lib"), path!(container, "usr", "share"));

    // Create system pseudo filesystem directories
    for systemFsDir in ["sys", "dev", "proc", "run", "tmp"]
    {
      mkdir!(path!(container, systemFsDir));
    }

    // Change to this directory to make accurate symlinks that work both in & outside of chroot
    set_current_dir(path!(container, "usr")).into_trace(Error::ServiceSandboxInit)?;
    symlink("lib", path!(container, "usr", "lib64")).into_trace(Error::ServiceSandboxInit)?;

    set_current_dir(container).into_trace(Error::ServiceSandboxInit)?;

    for (src, dest) in [("usr/lib", "lib"), ("usr/lib", "lib64"), ("usr/bin", "bin")]
    {
      symlink(src, path!(container, dest)).into_trace(Error::ServiceSandboxInit)?;
    }

    Ok(())
  }

  /// Expose internal files to the outside of the container
  pub(super) fn externalize(container: &PathBuf, file: &str) -> Result<()>
  {
    use std::os::unix::fs::symlink;

    // Where the internal file (inside container) is located
    let mut internal = PathBuf::from(&**container);
    internal.extend(&PathBuf::from(file));

    // Path where the symlink will be created, relative to the rootfs
    let external = PathBuf::from(format!("/{file}"));

    symlink(internal, external).into_trace(Error::ServiceSandboxInit)
  }

  pub(super) fn cli_args(mut self, container: BoxedStr, args: &mut VecDeque<BoxedStr>) -> Result<Vec<BoxedStr>>
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

    debug_cc!(&out);

    Ok(out)
  }
}


impl NsFlag
{
  pub const FLAGS: [&str; 18] = ["ShareVm", "ShareVSem", "ShareFs", "ShareFiles", "ShareSignalHandler",
                                  "ShareUntraced", "SharePTrace", "ShareVFork", "ShareParent", "ShareThread",
                                  "ShareIo", "NewMount", "NewCGroup", "NewUts", "NewIpc", "NewUser",
                                  "NewPid", "NewNetwork"];

  pub fn new(input: impl AnyStr) -> Option<Self>
  {
    enum_from_variant_str!(input.as_ref() => ShareVm | ShareVSem | ShareFs | ShareFiles | ShareSignalHandler
              | ShareUntraced | SharePTrace | ShareVFork | ShareParent | ShareThread | ShareIo | NewMount
              | NewCGroup | NewUts | NewIpc | NewUser | NewPid | NewNetwork)
  }
}

impl From<NsFlag> for CloneFlags
{
  fn from(flag: NsFlag) -> Self
  {
    Self::from_bits_truncate(flag as i32)
  }
}

impl From<NsFlags> for CloneFlags
{
  fn from(flags: NsFlags) -> Self
  {
    CloneFlags::from_bits_truncate(flags.0)
  }
}
